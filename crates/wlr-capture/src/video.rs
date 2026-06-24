//! Video encoding sink: turn a capture stream into a file via FFmpeg.
//!
//! A [`VideoEncoder`] implements [`FrameSink`](crate::sink::FrameSink), so the same
//! capture loop that feeds a screenshot can feed a recorder. The pixel path is
//! deliberately simple and portable: each RGBA frame is scaled to the encoder's
//! pixel format (NV12 / YUV420P) by libswscale on the CPU, then handed to the
//! encoder, which uploads to the GPU internally where applicable (NVENC). The
//! VAAPI backend, which needs an explicit hardware frame pool, is added separately.
//!
//! Two timing modes (see [`Mode`]): a real-time recording keeps each frame's wall
//! clock as a variable-frame-rate timestamp; a timelapse renumbers the sampled
//! frames sequentially at the output frame rate, so the result plays back sped up.
//!
//! The pipeline is initialised lazily on the first frame, so the encoder learns its
//! dimensions from the stream — the caller doesn't have to know them in advance.

use crate::sink::FrameSink;
use crate::wl::CapturedImage;
use anyhow::{Context, Result, anyhow, bail};
use ffmpeg::format::Pixel;
use ffmpeg_next as ffmpeg;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

static FFMPEG_INIT: Once = Once::new();

/// Initialise FFmpeg once per process (registers codecs, silences its logger to
/// warnings so a recording doesn't spam stderr).
fn ensure_ffmpeg() {
    FFMPEG_INIT.call_once(|| {
        // Errors here mean a broken FFmpeg build; surfaced later when we open a codec.
        let _ = ffmpeg::init();
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Warning);
    });
}

/// Which encoder to use. [`Backend::Auto`] picks the first available, preferring
/// hardware (NVENC, then VAAPI) over the software fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Choose the best available backend at runtime.
    Auto,
    /// NVIDIA NVENC (`h264_nvenc`). Takes CPU frames; uploads internally.
    Nvenc,
    /// VAAPI (`h264_vaapi`) on a DRM render node. Uses a hardware frame pool.
    Vaapi,
    /// Software `libx264`. Always works; uses the CPU.
    Software,
}

impl Backend {
    /// The FFmpeg encoder name for a concrete (non-`Auto`) backend.
    fn codec_name(self) -> &'static str {
        match self {
            Backend::Nvenc => "h264_nvenc",
            Backend::Vaapi => "h264_vaapi",
            Backend::Software => "libx264",
            Backend::Auto => unreachable!("resolved before use"),
        }
    }
}

/// Timing behaviour for the output stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Real-time: each frame keeps its capture timestamp (variable frame rate).
    Record,
    /// Timelapse: sampled frames are renumbered sequentially at `fps`, so the
    /// footage plays back faster than real time.
    Timelapse,
}

/// Encoder configuration. Dimensions are learned from the first frame.
#[derive(Clone, Debug)]
pub struct Options {
    pub backend: Backend,
    /// Output frame rate (the playback rate; also the rate-control hint).
    pub fps: u32,
    pub mode: Mode,
    /// DRM render node for the VAAPI backend (ignored otherwise).
    pub device: Option<PathBuf>,
    /// Mux an AAC audio stream fed by [`VideoEncoder::push_audio`] (the PCM source is
    /// the caller's concern — see [`crate::audio`]). Ignored for timelapse.
    pub audio: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
            fps: 30,
            mode: Mode::Record,
            device: None,
            audio: false,
        }
    }
}

/// Millisecond timebase used for real-time (VFR) recordings.
const MS_TIMEBASE: ffmpeg::Rational = ffmpeg::Rational(1, 1000);
/// Audio capture/encode format. Must match the PCM that [`crate::audio`] delivers.
pub(crate) const AUDIO_RATE: u32 = 48_000;
pub(crate) const AUDIO_CHANNELS: usize = 2;
/// Target AAC bit rate.
const AUDIO_BIT_RATE: usize = 160_000;

/// The live pipeline, built on the first frame.
struct Pipeline {
    octx: ffmpeg::format::context::Output,
    encoder: ffmpeg::encoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    /// Source size the current scaler was built for; rebuilt if the stream changes.
    src: (u32, u32),
    /// Even target dimensions (H.264 requires even width/height).
    dst: (u32, u32),
    enc_time_base: ffmpeg::Rational,
    ost_time_base: ffmpeg::Rational,
    target_format: Pixel,
    /// Strictly increasing PTS guard (VFR frames can share a millisecond).
    last_pts: i64,
    /// Sequential index, for timelapse PTS.
    index: i64,
    /// VAAPI hardware context (device + frame pool), `None` for the CPU-fed backends.
    vaapi: Option<VaapiCtx>,
    /// AAC audio stream, when recording with sound.
    audio: Option<AudioPipe>,
}

/// The muxed AAC audio stream: an encoder, its output stream, and a running PTS in
/// sample units (its timebase is `1/RATE`).
struct AudioPipe {
    encoder: ffmpeg::encoder::Audio,
    stream_index: usize,
    enc_time_base: ffmpeg::Rational,
    ost_time_base: ffmpeg::Rational,
    /// Samples per AAC frame (per channel), learned from the opened encoder.
    frame_size: usize,
    /// Next frame's PTS, in samples.
    pts: i64,
}

/// A VAAPI hardware device and its surface pool, kept alive for the encoder's
/// lifetime. Raw FFmpeg buffer refs; unref'd (frames before device) on drop.
struct VaapiCtx {
    device: *mut ffmpeg::ffi::AVBufferRef,
    frames: *mut ffmpeg::ffi::AVBufferRef,
}

impl VaapiCtx {
    /// Open a VAAPI device on `device` (a DRM render node, or the default if `None`)
    /// and build an NV12 surface pool sized `w`×`h`. Returns an error (never UB) if
    /// the device or pool can't be created.
    fn new(device: Option<&Path>, w: u32, h: u32) -> Result<Self> {
        use ffmpeg::ffi;
        use std::os::unix::ffi::OsStrExt;

        let cpath = match device {
            Some(p) => Some(
                std::ffi::CString::new(p.as_os_str().as_bytes())
                    .context("device path contains a NUL byte")?,
            ),
            None => None,
        };
        let dptr = cpath.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());

        unsafe {
            let mut dev: *mut ffi::AVBufferRef = std::ptr::null_mut();
            let r = ffi::av_hwdevice_ctx_create(
                &mut dev,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                dptr,
                std::ptr::null_mut(),
                0,
            );
            if r < 0 {
                let name =
                    device.map_or_else(|| "(default)".to_string(), |p| p.display().to_string());
                bail!("opening VAAPI device {name} (code {r})");
            }

            let frames = ffi::av_hwframe_ctx_alloc(dev);
            if frames.is_null() {
                ffi::av_buffer_unref(&mut dev);
                bail!("allocating the VAAPI frame pool");
            }
            let fctx = (*frames).data as *mut ffi::AVHWFramesContext;
            (*fctx).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*fctx).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            (*fctx).width = w as i32;
            (*fctx).height = h as i32;
            (*fctx).initial_pool_size = 20;

            let r = ffi::av_hwframe_ctx_init(frames);
            if r < 0 {
                let mut frames = frames;
                ffi::av_buffer_unref(&mut frames);
                ffi::av_buffer_unref(&mut dev);
                bail!("initialising the VAAPI frame pool (code {r})");
            }
            Ok(Self {
                device: dev,
                frames,
            })
        }
    }
}

impl Drop for VaapiCtx {
    fn drop(&mut self) {
        // Unref the pool before the device it borrows.
        unsafe {
            ffmpeg::ffi::av_buffer_unref(&mut self.frames);
            ffmpeg::ffi::av_buffer_unref(&mut self.device);
        }
    }
}

/// A [`FrameSink`] that encodes the capture stream to a file.
pub struct VideoEncoder {
    path: PathBuf,
    opts: Options,
    pipeline: Option<Pipeline>,
    /// Interleaved PCM awaiting encode (buffered until the pipeline exists, then drained
    /// in whole AAC frames on every video tick).
    audio_buf: Vec<f32>,
}

impl VideoEncoder {
    /// Create an encoder writing to `path` (container inferred from its extension,
    /// e.g. `.mp4`/`.mkv`). The codec is opened lazily on the first frame.
    pub fn new(path: impl Into<PathBuf>, opts: Options) -> Result<Self> {
        ensure_ffmpeg();
        Ok(Self {
            path: path.into(),
            opts,
            pipeline: None,
            audio_buf: Vec::new(),
        })
    }

    /// The backend that will actually be used (resolves `Auto`); handy for logging.
    pub fn resolved_backend(&self) -> Result<Backend> {
        resolve_backend(self.opts.backend)
    }
}

/// Resolve `Auto` to the first available backend; verify a concrete one exists.
fn resolve_backend(backend: Backend) -> Result<Backend> {
    ensure_ffmpeg();
    let available = |b: Backend| ffmpeg::encoder::find_by_name(b.codec_name()).is_some();
    match backend {
        Backend::Auto => [Backend::Nvenc, Backend::Vaapi, Backend::Software]
            .into_iter()
            .find(|&b| available(b))
            .ok_or_else(|| anyhow!("no H.264 encoder available (need NVENC, VAAPI or libx264)")),
        b if available(b) => Ok(b),
        b => bail!(
            "encoder '{}' is not available in this FFmpeg build",
            b.codec_name()
        ),
    }
}

/// Add an AAC stream to `octx` and open its encoder (48 kHz stereo, planar float).
fn build_audio_stream(
    octx: &mut ffmpeg::format::context::Output,
    global_header: bool,
) -> Result<AudioPipe> {
    let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::AAC)
        .ok_or_else(|| anyhow!("no AAC encoder in this FFmpeg build"))?;
    let mut astream = octx.add_stream(codec).context("adding audio stream")?;
    let stream_index = astream.index();

    let mut aenc = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder()
        .audio()?;
    aenc.set_rate(AUDIO_RATE as i32);
    aenc.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::STEREO);
    aenc.set_format(ffmpeg::format::Sample::F32(
        ffmpeg::format::sample::Type::Planar,
    ));
    aenc.set_bit_rate(AUDIO_BIT_RATE);
    let enc_time_base = ffmpeg::Rational(1, AUDIO_RATE as i32);
    aenc.set_time_base(enc_time_base);
    if global_header {
        aenc.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }

    let encoder = aenc.open_as(codec).context("opening the AAC encoder")?;
    astream.set_parameters(&encoder);
    let frame_size = (encoder.frame_size() as usize).max(1);

    Ok(AudioPipe {
        encoder,
        stream_index,
        enc_time_base,
        ost_time_base: enc_time_base, // replaced once the header is written
        frame_size,
        pts: 0,
    })
}

impl Pipeline {
    /// Build the output context + encoder for a source of size `(sw, sh)`.
    fn new(path: &Path, opts: &Options, sw: u32, sh: u32) -> Result<Self> {
        let backend = resolve_backend(opts.backend)?;
        let codec = ffmpeg::encoder::find_by_name(backend.codec_name())
            .ok_or_else(|| anyhow!("encoder '{}' unavailable", backend.codec_name()))?;

        // Even dimensions (H.264 chroma is subsampled 2×2).
        let dst = (sw & !1, sh & !1);
        if dst.0 == 0 || dst.1 == 0 {
            bail!("source too small to encode ({sw}x{sh})");
        }
        // The encoder's input format, and the scaler's output. NVENC takes NV12 and
        // libx264 takes planar YUV420P, both CPU frames sent directly. VAAPI consumes
        // hardware (VAAPI) frames, so we scale to a CPU NV12 frame and upload it.
        let (enc_format, target_format) = match backend {
            Backend::Software => (Pixel::YUV420P, Pixel::YUV420P),
            Backend::Nvenc => (Pixel::NV12, Pixel::NV12),
            Backend::Vaapi => (Pixel::VAAPI, Pixel::NV12),
            Backend::Auto => unreachable!("resolved above"),
        };

        let mut octx = ffmpeg::format::output(&path)
            .with_context(|| format!("opening output '{}'", path.display()))?;
        let global_header = octx
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);

        let mut ost = octx.add_stream(codec).context("adding video stream")?;
        let mut enc = ffmpeg::codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        enc.set_width(dst.0);
        enc.set_height(dst.1);
        enc.set_format(enc_format);
        enc.set_frame_rate(Some(ffmpeg::Rational(opts.fps as i32, 1)));
        // Real-time recordings are VFR (millisecond PTS); timelapses renumber at fps.
        let enc_time_base = match opts.mode {
            Mode::Record => MS_TIMEBASE,
            Mode::Timelapse => ffmpeg::Rational(1, opts.fps as i32),
        };
        enc.set_time_base(enc_time_base);
        if global_header {
            enc.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }

        // VAAPI needs a hardware frame pool wired into the codec context before open.
        let vaapi = if backend == Backend::Vaapi {
            let ctx =
                VaapiCtx::new(opts.device.as_deref(), dst.0, dst.1).context("setting up VAAPI")?;
            unsafe {
                (*enc.as_mut_ptr()).hw_frames_ctx = ffmpeg::ffi::av_buffer_ref(ctx.frames);
            }
            Some(ctx)
        } else {
            None
        };

        let encoder = enc
            .open_as(codec)
            .with_context(|| format!("opening encoder '{}'", backend.codec_name()))?;
        ost.set_parameters(&encoder);

        // Optional AAC audio stream (real-time recordings only — a timelapse has no
        // meaningful soundtrack). Added before the header is written.
        let mut audio = if opts.audio && opts.mode == Mode::Record {
            Some(build_audio_stream(&mut octx, global_header)?)
        } else {
            None
        };

        octx.write_header().context("writing container header")?;
        // The muxer may rewrite the stream timebase; read it back for packet rescale.
        let ost_time_base = octx.stream(0).context("no output stream")?.time_base();
        if let Some(ap) = audio.as_mut() {
            ap.ost_time_base = octx
                .stream(ap.stream_index)
                .context("no audio stream")?
                .time_base();
        }

        let scaler = ffmpeg::software::scaling::Context::get(
            Pixel::RGBA,
            sw,
            sh,
            target_format,
            dst.0,
            dst.1,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .context("creating RGBA->YUV scaler")?;

        Ok(Self {
            octx,
            encoder,
            scaler,
            src: (sw, sh),
            dst,
            enc_time_base,
            ost_time_base,
            target_format,
            last_pts: -1,
            index: 0,
            vaapi,
            audio,
        })
    }

    /// Rebuild the scaler if the source frame size changed (e.g. window resized).
    fn ensure_scaler(&mut self, sw: u32, sh: u32) -> Result<()> {
        if self.src == (sw, sh) {
            return Ok(());
        }
        self.scaler = ffmpeg::software::scaling::Context::get(
            Pixel::RGBA,
            sw,
            sh,
            self.target_format,
            self.dst.0,
            self.dst.1,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .context("rebuilding scaler for new source size")?;
        self.src = (sw, sh);
        Ok(())
    }

    /// Scale one RGBA frame, stamp its PTS, encode, and mux any ready packets.
    fn encode(&mut self, img: &CapturedImage, ts: Duration, mode: Mode) -> Result<()> {
        if img.width == 0 || img.height == 0 {
            return Ok(());
        }
        self.ensure_scaler(img.width, img.height)?;

        let mut src = ffmpeg::frame::Video::new(Pixel::RGBA, img.width, img.height);
        copy_rgba_into(&mut src, img);

        let mut dst = ffmpeg::frame::Video::new(self.target_format, self.dst.0, self.dst.1);
        self.scaler.run(&src, &mut dst).context("scaling frame")?;

        let pts = match mode {
            Mode::Record => (ts.as_millis() as i64).max(self.last_pts + 1),
            Mode::Timelapse => self.index,
        };
        self.last_pts = pts;
        self.index += 1;

        if let Some(vaapi) = &self.vaapi {
            // Upload the CPU NV12 frame to a VAAPI surface, then encode that.
            let mut hw = ffmpeg::frame::Video::empty();
            unsafe {
                let r = ffmpeg::ffi::av_hwframe_get_buffer(vaapi.frames, hw.as_mut_ptr(), 0);
                if r < 0 {
                    bail!("allocating a VAAPI surface (code {r})");
                }
                let r = ffmpeg::ffi::av_hwframe_transfer_data(hw.as_mut_ptr(), dst.as_ptr(), 0);
                if r < 0 {
                    bail!("uploading the frame to the GPU (code {r})");
                }
            }
            hw.set_pts(Some(pts));
            self.encoder.send_frame(&hw).context("sending frame")?;
        } else {
            dst.set_pts(Some(pts));
            self.encoder.send_frame(&dst).context("sending frame")?;
        }
        self.drain()
    }

    /// Pull encoded packets and write them, rescaling to the container timebase.
    fn drain(&mut self) -> Result<()> {
        let mut packet = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(self.enc_time_base, self.ost_time_base);
            packet
                .write_interleaved(&mut self.octx)
                .context("writing packet")?;
        }
        Ok(())
    }

    /// Consume whole AAC frames worth of interleaved PCM from `buf`, deinterleaving each
    /// into a planar float frame, encoding and muxing it. Leaves the remainder in `buf`.
    fn encode_audio(&mut self, buf: &mut Vec<f32>) -> Result<()> {
        let frame_size = match &self.audio {
            Some(a) => a.frame_size,
            None => {
                buf.clear();
                return Ok(());
            }
        };
        let need = frame_size * AUDIO_CHANNELS;
        while buf.len() >= need {
            let mut planes: Vec<Vec<f32>> = (0..AUDIO_CHANNELS)
                .map(|_| Vec::with_capacity(frame_size))
                .collect();
            for fr in buf[..need].chunks_exact(AUDIO_CHANNELS) {
                for (c, p) in planes.iter_mut().enumerate() {
                    p.push(fr[c]);
                }
            }
            buf.drain(..need);

            let mut frame = ffmpeg::frame::Audio::new(
                ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
                frame_size,
                ffmpeg::channel_layout::ChannelLayout::STEREO,
            );
            frame.set_rate(AUDIO_RATE);
            for (c, p) in planes.iter().enumerate() {
                frame.plane_mut::<f32>(c).copy_from_slice(p);
            }

            let ap = self.audio.as_mut().expect("audio present");
            frame.set_pts(Some(ap.pts));
            ap.pts += frame_size as i64;
            ap.encoder
                .send_frame(&frame)
                .context("sending audio frame")?;

            let mut packet = ffmpeg::Packet::empty();
            while ap.encoder.receive_packet(&mut packet).is_ok() {
                packet.set_stream(ap.stream_index);
                packet.rescale_ts(ap.enc_time_base, ap.ost_time_base);
                packet
                    .write_interleaved(&mut self.octx)
                    .context("writing audio packet")?;
            }
        }
        Ok(())
    }

    /// Flush both encoders and finalise the container.
    fn finish(mut self) -> Result<()> {
        self.encoder.send_eof().context("flushing encoder")?;
        self.drain()?;
        if let Some(ap) = self.audio.as_mut() {
            ap.encoder.send_eof().context("flushing audio encoder")?;
            let mut packet = ffmpeg::Packet::empty();
            while ap.encoder.receive_packet(&mut packet).is_ok() {
                packet.set_stream(ap.stream_index);
                packet.rescale_ts(ap.enc_time_base, ap.ost_time_base);
                packet
                    .write_interleaved(&mut self.octx)
                    .context("writing final audio packet")?;
            }
        }
        self.octx
            .write_trailer()
            .context("writing container trailer")?;
        Ok(())
    }
}

/// Copy tightly-packed RGBA pixels into an FFmpeg frame, honouring its row stride
/// (FFmpeg pads rows for alignment, so a flat `copy_from_slice` would shear).
fn copy_rgba_into(frame: &mut ffmpeg::frame::Video, img: &CapturedImage) {
    let w = img.width as usize;
    let stride = frame.stride(0);
    let row_bytes = w * 4;
    let dst = frame.data_mut(0);
    for y in 0..img.height as usize {
        let s = y * row_bytes;
        let d = y * stride;
        dst[d..d + row_bytes].copy_from_slice(&img.rgba[s..s + row_bytes]);
    }
}

impl FrameSink for VideoEncoder {
    fn push(&mut self, img: &CapturedImage, ts: Duration) -> Result<()> {
        if self.pipeline.is_none() {
            self.pipeline = Some(Pipeline::new(
                &self.path, &self.opts, img.width, img.height,
            )?);
        }
        let mode = self.opts.mode;
        let p = self.pipeline.as_mut().expect("just initialised");
        p.encode(img, ts, mode)?;
        p.encode_audio(&mut self.audio_buf)
    }

    /// Buffer interleaved PCM ([`AUDIO_CHANNELS`] per frame, [`AUDIO_RATE`] Hz); it is
    /// muxed on the next [`FrameSink::push`]. A no-op unless `opts.audio` is set. Bounded
    /// while the pipeline warms up (no video frame yet) so it can't grow without limit.
    fn push_audio(&mut self, pcm: &[f32]) {
        if !self.opts.audio {
            return;
        }
        self.audio_buf.extend_from_slice(pcm);
        let cap = AUDIO_RATE as usize * AUDIO_CHANNELS * 5;
        if self.audio_buf.len() > cap {
            let drop = self.audio_buf.len() - cap;
            self.audio_buf.drain(..drop);
        }
    }

    fn finish(&mut self) -> Result<()> {
        match self.pipeline.take() {
            Some(mut p) => {
                p.encode_audio(&mut self.audio_buf)?; // flush whole buffered AAC frames
                p.finish()
            }
            None => Ok(()), // no frames captured: nothing to write
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic RGBA frame: a diagonal gradient shifted by `t` (so motion exists
    /// for the encoder to chew on).
    fn frame(w: u32, h: u32, t: u32) -> CapturedImage {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                rgba[i] = ((x + t) & 0xff) as u8;
                rgba[i + 1] = ((y + t) & 0xff) as u8;
                rgba[i + 2] = ((x + y) & 0xff) as u8;
                rgba[i + 3] = 255;
            }
        }
        CapturedImage {
            width: w,
            height: h,
            rgba,
        }
    }

    /// End-to-end encode of synthetic frames to a real file, with no Wayland session.
    /// Skips cleanly if `requested` resolves to no usable encoder (e.g. CI without GPU
    /// or libx264). When `ffprobe` is on PATH, asserts the stream's codec and size.
    fn run_encode(requested: Backend) {
        let backend = match resolve_backend(requested) {
            Ok(b) => b,
            Err(_) => {
                eprintln!("backend {requested:?} unavailable; skipping");
                return;
            }
        };

        let (w, h, fps, n) = (320u32, 240u32, 30u32, 30u32);
        // Unique per backend: tests run in parallel and would otherwise share a file.
        let path = std::env::temp_dir().join(format!(
            "wlr_capture_enc_{}_{}.mp4",
            std::process::id(),
            backend.codec_name()
        ));
        let mut enc = VideoEncoder::new(
            &path,
            Options {
                backend,
                fps,
                mode: Mode::Record,
                device: Some("/dev/dri/renderD128".into()),
                audio: false,
            },
        )
        .expect("create encoder");

        for i in 0..n {
            let ts = Duration::from_millis((i * 1000 / fps) as u64);
            enc.push(&frame(w, h, i), ts).expect("push frame");
        }
        enc.finish().expect("finish");

        let meta = std::fs::metadata(&path).expect("output file exists");
        assert!(
            meta.len() > 1000,
            "output suspiciously small: {} bytes",
            meta.len()
        );

        // Deeper check when ffprobe is available.
        if let Ok(out) = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(&path)
            .output()
            && out.status.success()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            let fields: Vec<&str> = s.split_whitespace().collect();
            assert_eq!(fields, ["h264", "320", "240"], "ffprobe stream metadata");
        }

        let _ = std::fs::remove_file(&path);
    }

    /// Software (libx264) path — the portable fallback.
    #[test]
    fn encodes_software() {
        run_encode(Backend::Software);
    }

    /// Hardware NVENC path — the default on an NVIDIA box. Feeds NV12 CPU frames the
    /// encoder uploads internally (no hardware frame pool, unlike VAAPI).
    ///
    /// `#[ignore]`d: a CI runner has the `h264_nvenc` codec *compiled* but no NVIDIA
    /// hardware, so opening it fails rather than skipping. Run locally on a GPU box with
    /// `--ignored`.
    #[test]
    #[ignore]
    fn encodes_nvenc() {
        run_encode(Backend::Nvenc);
    }

    /// Hardware VAAPI path — exercises the `av_hwframe` upload to a surface pool. Needs a
    /// real render node, so `#[ignore]`d in CI (run locally with `--ignored`).
    #[test]
    #[ignore]
    fn encodes_vaapi() {
        run_encode(Backend::Vaapi);
    }
}
