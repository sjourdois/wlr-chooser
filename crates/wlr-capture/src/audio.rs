//! Audio capture for recording: native PipeWire, with an optional Pulse/ALSA fallback.
//!
//! Captures system audio (the default sink's monitor) — or a named source — as 48 kHz
//! stereo F32 interleaved PCM into a shared buffer the video muxer drains. The capture
//! runs on its own thread; dropping the [`AudioCapture`] stops it.
//!
//! Capture and encoding are decoupled (the muxer in [`crate::video`] turns this PCM into
//! an AAC stream), so the backend is swappable. The native **PipeWire** path is always
//! built; with the `audio-fallback` feature, **Pulse** and **ALSA** are tried after it
//! (via FFmpeg's libavdevice) for the rare PipeWire-less host. `WLR_SHOT_AUDIO_BACKEND`
//! (`pipewire`/`pulse`/`alsa`) forces one, mainly for testing.

use crate::error::{CaptureError, Context, Result};
use pipewire as pw;
use pw::{properties::properties, spa};
use spa::pod::Pod;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Sample rate the buffer is normalised to (PipeWire/swr resample the source to this).
pub const RATE: u32 = 48_000;
/// Channel count the buffer is normalised to.
pub const CHANNELS: u32 = 2;

/// A running capture. Interleaved f32 samples accumulate in `pcm`; the encoder drains
/// them with [`AudioCapture::drain`]. Dropping it stops the backend and joins the thread.
pub struct AudioCapture {
    pcm: Arc<Mutex<VecDeque<f32>>>,
    stop: Stop,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// How to signal the capture thread to stop, per backend.
enum Stop {
    /// PipeWire: a message on its loop channel quits the loop.
    Pipewire(pw::channel::Sender<()>),
    /// libavdevice: a flag the blocking read loop checks between packets.
    #[cfg(feature = "audio-fallback")]
    Flag(Arc<std::sync::atomic::AtomicBool>),
}

impl AudioCapture {
    /// Start capturing, trying PipeWire first, then (with `audio-fallback`) Pulse and
    /// ALSA. `target` is a backend-specific source name; `None` captures system audio.
    pub fn start(target: Option<String>) -> Result<Self> {
        let forced = std::env::var("WLR_SHOT_AUDIO_BACKEND").ok();
        let want = |b: &str| forced.as_deref().is_none_or(|f| f.eq_ignore_ascii_case(b));
        let mut tried = Vec::new();

        if want("pipewire") {
            match Self::start_pipewire(target.clone()) {
                Ok(c) => return Ok(c),
                Err(e) => tried.push(format!("pipewire ({e})")),
            }
        }
        #[cfg(feature = "audio-fallback")]
        {
            if want("pulse") {
                match Self::start_lavd("pulse", fallback::pulse_source(target.as_deref())) {
                    Ok(c) => return Ok(c),
                    Err(e) => tried.push(format!("pulse ({e})")),
                }
            }
            if want("alsa") {
                let dev = target.clone().unwrap_or_else(|| "default".into());
                match Self::start_lavd("alsa", dev) {
                    Ok(c) => return Ok(c),
                    Err(e) => tried.push(format!("alsa ({e})")),
                }
            }
        }
        Err(CaptureError::NoAudioBackend)
    }

    /// Native PipeWire capture (always built).
    fn start_pipewire(target: Option<String>) -> Result<Self> {
        let pcm: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let pcm_thread = pcm.clone();
        // The loop thread hands back its stop-sender (or a setup error) before run()s.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<pw::channel::Sender<()>, String>>();

        let thread = std::thread::Builder::new()
            .name("wlr-audio-pw".into())
            .spawn(move || {
                if let Err(e) = pw_loop(pcm_thread, target, &ready_tx) {
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            })
            .context("spawning the audio capture thread")?;

        match ready_rx.recv() {
            Ok(Ok(stop)) => Ok(Self {
                pcm,
                stop: Stop::Pipewire(stop),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(CaptureError::msg(e.to_string()))
            }
            Err(_) => Err(CaptureError::msg("thread exited during setup")),
        }
    }

    /// Take all PCM captured so far (interleaved, [`CHANNELS`] per sample frame).
    pub fn drain(&self) -> Vec<f32> {
        let mut q = self.pcm.lock().unwrap();
        q.drain(..).collect()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        match &self.stop {
            Stop::Pipewire(s) => {
                let _ = s.send(());
            }
            #[cfg(feature = "audio-fallback")]
            Stop::Flag(f) => f.store(true, std::sync::atomic::Ordering::SeqCst),
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The PipeWire loop body, run on the capture thread.
fn pw_loop(
    pcm: Arc<Mutex<VecDeque<f32>>>,
    target: Option<String>,
    ready: &mpsc::Sender<Result<pw::channel::Sender<()>, String>>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).context("main loop")?;
    let context = pw::context::ContextRc::new(&mainloop, None).context("context")?;
    let core = context.connect_rc(None).context("connect")?;

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
    };
    match &target {
        Some(t) => {
            props.insert(*pw::keys::TARGET_OBJECT, t.clone());
        }
        // No explicit target: capture from a sink's monitor (i.e. system audio).
        None => {
            props.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");
        }
    }

    let stream = pw::stream::StreamBox::new(&core, "wlr-shot-audio", props).context("stream")?;

    let pcm_cb = pcm.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .process(move |stream, ()| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else {
                return;
            };
            let n_bytes = d.chunk().size() as usize;
            if let Some(slice) = d.data() {
                let slice = &slice[..n_bytes.min(slice.len())];
                let mut q = pcm_cb.lock().unwrap();
                for s in slice.chunks_exact(4) {
                    q.push_back(f32::from_le_bytes([s[0], s[1], s[2], s[3]]));
                }
            }
        })
        .register()
        .context("listener")?;

    // Ask for 48 kHz stereo F32LE; PipeWire's adapter resamples/remixes to match.
    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(RATE);
    audio_info.set_channels(CHANNELS);
    let obj = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .context("POD serialize")?
    .0
    .into_inner();
    let mut params =
        [Pod::from_bytes(&values).ok_or_else(|| CaptureError::msg("invalid format POD"))?];

    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .context("connect stream")?;

    // A message on this channel quits the loop (sent by `AudioCapture::drop`).
    let (stop_tx, stop_rx) = pw::channel::channel::<()>();
    let ml = mainloop.clone();
    let _recv = stop_rx.attach(mainloop.loop_(), move |_| ml.quit());

    ready
        .send(Ok(stop_tx))
        .map_err(|_| CaptureError::msg("handing back the stop channel"))?;
    mainloop.run();
    Ok(())
}

/// Pulse/ALSA capture via FFmpeg's libavdevice (the `audio-fallback` feature).
#[cfg(feature = "audio-fallback")]
mod fallback {
    use super::{CHANNELS, RATE};
    use crate::error::{CaptureError, Context, Result};
    use ffmpeg_next as ffmpeg;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    /// The default Pulse source for system audio: the default sink's monitor (via
    /// `pactl`), falling back to `default`. An explicit `target` wins.
    pub fn pulse_source(target: Option<&str>) -> String {
        if let Some(t) = target {
            return t.to_string();
        }
        run_cmd("pactl", &["get-default-sink"])
            .map(|s| format!("{s}.monitor"))
            .unwrap_or_else(|| "default".into())
    }

    fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    impl super::AudioCapture {
        /// Open `device` with libavdevice input `format` (`pulse`/`alsa`) and stream its
        /// PCM, resampled to 48 kHz stereo f32, into the shared buffer.
        pub(super) fn start_lavd(format: &str, device: String) -> Result<Self> {
            let pcm: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
            let pcm_t = pcm.clone();
            let flag = Arc::new(AtomicBool::new(false));
            let flag_t = flag.clone();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let fmt = format.to_string();

            let thread = std::thread::Builder::new()
                .name("wlr-audio-lavd".into())
                .spawn(move || {
                    if let Err(e) = lavd_loop(pcm_t, &fmt, &device, &flag_t, &ready_tx) {
                        // If the device already opened, the handshake is gone, so surface
                        // a mid-stream failure on stderr rather than swallowing it.
                        if ready_tx.send(Err(e.to_string())).is_err() {
                            eprintln!("wlr-shot: audio capture stopped: {e}");
                        }
                    }
                })
                .context("spawning the audio capture thread")?;

            match ready_rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    pcm,
                    stop: super::Stop::Flag(flag),
                    thread: Some(thread),
                }),
                Ok(Err(e)) => {
                    let _ = thread.join();
                    Err(CaptureError::msg(e))
                }
                Err(_) => Err(CaptureError::msg("thread exited during setup")),
            }
        }
    }

    /// Read/decode/resample loop for a libavdevice input. Sends `Ok(())` once the device
    /// is open, then pushes PCM until `stop` is set.
    fn lavd_loop(
        pcm: Arc<Mutex<VecDeque<f32>>>,
        format: &str,
        device: &str,
        stop: &AtomicBool,
        ready: &mpsc::Sender<Result<(), String>>,
    ) -> Result<()> {
        ffmpeg::init().ok();
        ffmpeg::device::register_all();

        // Find the libavdevice input demuxer and open the device by name.
        let ifmt = unsafe {
            let name = std::ffi::CString::new(format).context("device format name")?;
            let p = ffmpeg::ffi::av_find_input_format(name.as_ptr());
            if p.is_null() {
                return Err(CaptureError::msg(format!(
                    "input '{format}' not available in this FFmpeg build"
                )));
            }
            ffmpeg::format::format::Input::wrap(p as *mut _)
        };
        let mut ictx =
            match ffmpeg::format::open(&device, &ffmpeg::format::format::Format::Input(ifmt))
                .map_err(|e| CaptureError::msg(format!("opening {format} '{device}': {e}")))?
            {
                ffmpeg::format::context::Context::Input(i) => i,
                _ => {
                    return Err(CaptureError::msg(format!(
                        "{format} '{device}' is not an input"
                    )));
                }
            };

        let stream = ictx
            .streams()
            .best(ffmpeg::media::Type::Audio)
            .ok_or_else(|| {
                CaptureError::msg(format!("no audio stream from {format} '{device}'"))
            })?;
        let stream_index = stream.index();
        let mut decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .context("decoder from stream parameters")?
            .decoder()
            .audio()
            .context("audio decoder")?;

        ready
            .send(Ok(()))
            .map_err(|_| CaptureError::msg("ready signal"))?;

        let mut resampler: Option<ffmpeg::software::resampling::Context> = None;
        let mut frame = ffmpeg::frame::Audio::empty();
        let mut out = ffmpeg::frame::Audio::empty();

        for (s, packet) in ictx.packets() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if s.index() != stream_index {
                continue;
            }
            decoder
                .send_packet(&packet)
                .context("decoding audio packet")?;
            while decoder.receive_frame(&mut frame).is_ok() {
                // Stamp a canonical layout for the channel count on every frame, so swr
                // doesn't reject a differing AVChannelLayout representation ("Input
                // changed") between its config and the decoded frames.
                let in_layout =
                    ffmpeg::channel_layout::ChannelLayout::default(frame.channels().max(1) as i32);
                if resampler.is_none() {
                    resampler = Some(
                        ffmpeg::software::resampling::Context::get(
                            frame.format(),
                            in_layout,
                            frame.rate(),
                            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
                            ffmpeg::channel_layout::ChannelLayout::STEREO,
                            RATE,
                        )
                        .context("building the audio resampler")?,
                    );
                }
                frame.set_channel_layout(in_layout);
                resampler
                    .as_mut()
                    .unwrap()
                    .run(&frame, &mut out)
                    .context("resampling audio")?;
                let n = out.samples() * CHANNELS as usize;
                let bytes = out.data(0);
                if bytes.len() >= n * 4 {
                    let mut q = pcm.lock().unwrap();
                    for b in bytes[..n * 4].chunks_exact(4) {
                        q.push_back(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live smoke test (needs a running PipeWire): capture system audio briefly and
    /// confirm samples flow. `cargo test -p wlr-capture --features audio -- --ignored`.
    #[test]
    #[ignore]
    fn captures_system_audio() {
        let cap = AudioCapture::start(None).expect("start capture");
        std::thread::sleep(std::time::Duration::from_millis(500));
        let pcm = cap.drain();
        let peak = pcm.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        eprintln!("captured {} samples, peak {peak:.3}", pcm.len());
        assert!(!pcm.is_empty(), "no PCM captured in 500ms");
        assert_eq!(
            pcm.len() % CHANNELS as usize,
            0,
            "ragged channel interleave"
        );
    }
}
