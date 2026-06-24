//! wlr-shot — screen capture for wlroots compositors.
//!
//! A thin CLI over the shared `wlr-capture` engine: it resolves a *source* (a full
//! output, a region — logical geometry, slurp-compatible, stitched across outputs —
//! or a window), via `wlr_capture::capture`, and either encodes a still to
//! PNG/JPEG/PPM (`screenshot`, file/stdout/clipboard) or streams it to an H.264 file
//! (`record`, via the `wlr_capture::video` sink — the `video` feature). The
//! interactive frozen region selector (`-s`) lives in `wlr_capture::overlay`.

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io::{Cursor, Write};
use wlr_capture::capture::{self, DEFAULT_BUDGET};
use wlr_capture::{focus, overlay, wl};

#[derive(Parser)]
#[command(
    name = "wlr-shot",
    version,
    about = "Screen capture for wlroots (screenshots, recording, timelapse)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Capture a screenshot of an output, a region, or a window.
    Screenshot(ShotArgs),
    /// Record an output, a window, or a region — H.264 video, or animated GIF/WebP.
    #[cfg(feature = "video")]
    Record(RecordArgs),
    /// Internal: serve a clipboard selection read from stdin. Spawned detached by
    /// `screenshot --clipboard`; not meant to be run by hand.
    #[command(hide = true)]
    ClipboardServe {
        /// MIME type to advertise for the bytes on stdin.
        #[arg(long)]
        mime: String,
    },
}

#[derive(Args)]
struct ShotArgs {
    /// Interactively select a region on a frozen overlay (drag to select, Esc to
    /// cancel). Spans all outputs.
    #[arg(short = 's', long, group = "source")]
    select: bool,
    /// Capture this named output (e.g. `DP-4`). Defaults to the only output, or
    /// errors listing the names if there is more than one.
    #[arg(short = 'o', long, value_name = "NAME", group = "source")]
    output: Option<String>,
    /// Capture this logical region, `"X,Y WxH"` (the format slurp prints), stitched
    /// across the outputs it covers.
    #[arg(short = 'g', long, value_name = "GEOM", group = "source")]
    geometry: Option<String>,
    /// Capture the window with this `ext-foreign-toplevel` identifier (as printed
    /// by `wlr-chooser`).
    #[arg(short = 'w', long, value_name = "ID", group = "source")]
    window: Option<String>,
    /// Launch `wlr-chooser` to pick a window to capture.
    #[arg(long, group = "source")]
    pick_window: bool,
    /// Capture the whole layout: every output combined into one image.
    #[arg(long, group = "source")]
    all: bool,
    /// Capture the active (focused) window — needs compositor focus info.
    #[arg(short = 'a', long, group = "source")]
    active_window: bool,
    /// Capture the focused output — needs compositor focus info.
    #[arg(long, group = "source")]
    current_output: bool,
    /// Output image format.
    #[arg(short = 't', long, value_enum, default_value_t = Fmt::Png)]
    r#type: Fmt,
    /// JPEG quality (1–100), only for `--type jpeg`.
    #[arg(short = 'q', long, default_value_t = 90)]
    quality: u8,
    /// Copy the screenshot to the Wayland clipboard instead of writing it out.
    /// Runs a small background daemon that serves the selection (wlroots
    /// `data-control`) until another client replaces it.
    #[arg(short = 'c', long)]
    clipboard: bool,
    /// With `--clipboard`: serve the selection in the foreground (don't detach).
    /// Mainly for scripts and debugging; the process blocks until replaced.
    #[arg(long, requires = "clipboard")]
    clipboard_foreground: bool,
    /// List the available outputs and exit.
    #[arg(long)]
    list_outputs: bool,
    /// Destination file, or `-` for stdout (the default). Ignored with `--clipboard`.
    #[arg(value_name = "FILE", default_value = "-")]
    file: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum Fmt {
    Png,
    Jpeg,
    Ppm,
}

fn main() {
    wlr_capture::i18n::init();
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Screenshot(args) => screenshot(args),
        #[cfg(feature = "video")]
        Cmd::Record(args) => record(args),
        Cmd::ClipboardServe { mime } => clipboard_serve(&mime),
    };
    if let Err(e) = res {
        eprintln!("wlr-shot: {e:#}");
        std::process::exit(1);
    }
}

fn screenshot(args: ShotArgs) -> Result<()> {
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();

    if args.list_outputs {
        for o in client.outputs() {
            let (w, h) = o.logical_size();
            println!("{}\t{}x{}+{},{}", o.name, w, h, o.logical_x, o.logical_y);
        }
        return Ok(());
    }

    // Resolve the source (the flags form an exclusive group).
    let img = if args.select {
        // Freeze every output, let the user drag a region, then crop from the same
        // frozen pixels (so the shot matches exactly what was on screen).
        let caps = capture::capture_all(&mut client, DEFAULT_BUDGET)?;
        match overlay::select_region(&caps)? {
            Some(region) => capture::composite(&caps, region)?,
            None => std::process::exit(1), // cancelled
        }
    } else if let Some(geo) = &args.geometry {
        capture::capture_region(&mut client, capture::parse_geometry(geo)?, DEFAULT_BUDGET)?
    } else if args.all {
        let region = capture::whole_layout(&client)?;
        capture::capture_region(&mut client, region, DEFAULT_BUDGET)?
    } else if args.active_window {
        capture::capture_region(&mut client, active_window_rect()?, DEFAULT_BUDGET)?
    } else if args.current_output {
        capture::capture_output(&mut client, Some(&focused_output()?), DEFAULT_BUDGET)?
    } else if args.pick_window {
        capture::capture_window(&mut client, &pick_window()?, DEFAULT_BUDGET)?
    } else if let Some(id) = &args.window {
        capture::capture_window(&mut client, id, DEFAULT_BUDGET)?
    } else {
        capture::capture_output(&mut client, args.output.as_deref(), DEFAULT_BUDGET)?
    };

    let bytes = encode(&img, args.r#type, args.quality).context("encoding image")?;
    if args.clipboard {
        clipboard_copy(mime_for(args.r#type), bytes, args.clipboard_foreground)
            .context("copying to clipboard")?;
    } else {
        write_out(&args.file, &bytes).context("writing output")?;
    }
    Ok(())
}

/// The clipboard MIME type for an output format.
fn mime_for(fmt: Fmt) -> &'static str {
    match fmt {
        Fmt::Png => "image/png",
        Fmt::Jpeg => "image/jpeg",
        Fmt::Ppm => "image/x-portable-pixmap",
    }
}

/// Put `bytes` on the Wayland clipboard. Unless `foreground`, this detaches a
/// background daemon (a re-exec of ourselves in `clipboard-serve` mode) that serves
/// the selection until replaced — the clipboard is pull-based, so the data must
/// outlive this process.
fn clipboard_copy(mime: &str, bytes: Vec<u8>, foreground: bool) -> Result<()> {
    if foreground {
        return wlr_capture::clipboard::serve(mime, bytes);
    }
    wlr_capture::clipboard::spawn_detached(&bytes, &["clipboard-serve", "--mime", mime])
}

/// The `clipboard-serve` daemon body: read the blob from stdin, then serve it.
fn clipboard_serve(mime: &str) -> Result<()> {
    use std::io::Read;
    let mut data = Vec::new();
    std::io::stdin()
        .read_to_end(&mut data)
        .context("reading clipboard data")?;
    wlr_capture::clipboard::serve(mime, data)
}

/// The active window's logical rectangle, via compositor focus IPC.
fn active_window_rect() -> Result<wl::Region> {
    let backend = focus::detect()
        .context("focus info unavailable (unsupported compositor); try --pick-window")?;
    backend
        .active_window_rect()
        .with_context(|| format!("no active window detected (via {})", backend.name()))
}

/// The focused output's name, via compositor focus IPC.
fn focused_output() -> Result<String> {
    let backend = focus::detect()
        .context("focus info unavailable (unsupported compositor); specify -o NAME")?;
    backend
        .focused_output()
        .with_context(|| format!("no focused output detected (via {})", backend.name()))
}

/// Launch `wlr-chooser --windows` and parse its `Window: <id>` stdout contract.
/// Prefers a `wlr-chooser` next to our own binary, else one on `PATH`.
fn pick_window() -> Result<String> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("wlr-chooser")))
        .filter(|p| p.exists());
    let mut cmd = match sibling {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("wlr-chooser"),
    };
    let out = cmd
        .arg("--windows")
        .output()
        .context("lancement de wlr-chooser")?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("Window: "))
        .map(|s| s.to_string())
        .context("no window picked")
}

/// Encode an RGBA capture to the requested format. Captures are opaque (alpha is
/// forced to 255 for X formats), so dropping alpha for JPEG/PPM is lossless.
fn encode(img: &wl::CapturedImage, fmt: Fmt, quality: u8) -> Result<Vec<u8>> {
    use image::{DynamicImage, ImageFormat, RgbaImage, codecs::jpeg::JpegEncoder};
    if img.width == 0 || img.height == 0 {
        bail!("empty image (region off-screen?)");
    }
    let rgba = RgbaImage::from_raw(img.width, img.height, img.rgba.clone())
        .context("inconsistent dimensions/buffer")?;
    let dynimg = DynamicImage::ImageRgba8(rgba);

    let mut out = Vec::new();
    let mut cur = Cursor::new(&mut out);
    match fmt {
        Fmt::Png => dynimg.write_to(&mut cur, ImageFormat::Png)?,
        Fmt::Ppm => {
            DynamicImage::ImageRgb8(dynimg.to_rgb8()).write_to(&mut cur, ImageFormat::Pnm)?
        }
        Fmt::Jpeg => {
            JpegEncoder::new_with_quality(&mut cur, quality.clamp(1, 100))
                .encode_image(&dynimg.to_rgb8())?;
        }
    }
    Ok(out)
}

fn write_out(file: &str, bytes: &[u8]) -> Result<()> {
    if file == "-" {
        std::io::stdout().write_all(bytes)?;
    } else {
        std::fs::write(file, bytes)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `record` — stream a source to a video file (the `video` feature).
// ---------------------------------------------------------------------------

#[cfg(feature = "video")]
mod record_impl {
    use super::{active_window_rect, focused_output, pick_window};
    use anyhow::{Context, Result, bail};
    use clap::{Args, ValueEnum};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    use wlr_capture::capture;
    use wlr_capture::gl::GpuReadback;
    use wlr_capture::sink::FrameSink;
    use wlr_capture::stream;
    use wlr_capture::video::{self, VideoEncoder};
    use wlr_capture::wl::{self, CapturedImage, Output, Region};

    /// How long to wait for new frames each poll round.
    const ROUND: Duration = Duration::from_millis(200);
    /// How long to wait for a window/output to appear before giving up.
    const APPEAR_GRACE: Duration = Duration::from_secs(5);

    /// GIF/WebP are share-sized formats, and per-frame GIF quantization is O(pixels) — a
    /// full 4K frame is unusably slow — so both downscale the long side to a cap.
    const GIF_MAX_DIM: u32 = 800;
    const WEBP_MAX_DIM: u32 = 1280;

    /// A frame as an `RgbaImage`, downscaled so its long side is at most `max_dim`.
    fn downscaled(img: &CapturedImage, max_dim: u32) -> Result<image::RgbaImage> {
        let buf = image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone())
            .ok_or_else(|| anyhow::anyhow!("image dimensions don't match the buffer"))?;
        let long = img.width.max(img.height);
        if long <= max_dim {
            return Ok(buf);
        }
        let scale = max_dim as f32 / long as f32;
        let nw = ((img.width as f32 * scale).round() as u32).max(1);
        let nh = ((img.height as f32 * scale).round() as u32).max(1);
        Ok(image::imageops::resize(
            &buf,
            nw,
            nh,
            image::imageops::FilterType::Triangle,
        ))
    }

    /// Animated-GIF sink: each captured frame is quantized to 256 colours and appended
    /// with a fixed 1/fps delay (so a timelapse plays back sped up, like the video path).
    struct GifSink {
        enc: image::codecs::gif::GifEncoder<std::io::BufWriter<std::fs::File>>,
        delay: image::Delay,
    }

    impl GifSink {
        fn new(path: &str, fps: u32) -> Result<Self> {
            use image::codecs::gif::{GifEncoder, Repeat};
            let file = std::fs::File::create(path).context("creating the GIF file")?;
            // A fast quantizer: we encode every frame synchronously in the capture loop.
            let mut enc = GifEncoder::new_with_speed(std::io::BufWriter::new(file), 30);
            enc.set_repeat(Repeat::Infinite).context("GIF loop flag")?;
            // GIF delays are centisecond-resolution; clamp to a browser-safe 2cs floor.
            let ms = (1000.0 / fps as f64).round().max(20.0) as u32;
            Ok(Self {
                enc,
                delay: image::Delay::from_numer_denom_ms(ms, 1),
            })
        }
    }

    impl FrameSink for GifSink {
        fn push(&mut self, img: &CapturedImage, _ts: Duration) -> Result<()> {
            let buf = downscaled(img, GIF_MAX_DIM)?;
            self.enc
                .encode_frame(image::Frame::from_parts(buf, 0, 0, self.delay))
                .context("encoding a GIF frame")?;
            Ok(())
        }
        // The GIF trailer is written when the encoder is dropped (after finish()).
    }

    /// Animated-WebP sink. libwebp's animation encoder borrows each frame's pixels until
    /// the whole file is encoded, so we buffer frames and emit on finish().
    struct WebpSink {
        path: String,
        fps: u32,
        dims: Option<(u32, u32)>,
        frames: Vec<Vec<u8>>,
    }

    impl WebpSink {
        fn new(path: &str, fps: u32) -> Self {
            Self {
                path: path.to_string(),
                fps,
                dims: None,
                frames: Vec::new(),
            }
        }
    }

    impl FrameSink for WebpSink {
        fn push(&mut self, img: &CapturedImage, _ts: Duration) -> Result<()> {
            let buf = downscaled(img, WEBP_MAX_DIM)?;
            let dims = *self.dims.get_or_insert((buf.width(), buf.height()));
            // Frames must share dimensions; the source is a fixed output/region/window.
            if (buf.width(), buf.height()) == dims {
                self.frames.push(buf.into_raw());
            }
            Ok(())
        }

        fn finish(&mut self) -> Result<()> {
            let Some((w, h)) = self.dims else {
                return Ok(());
            };
            let mut config = webp::WebPConfig::new().map_err(|_| anyhow::anyhow!("WebP config"))?;
            config.quality = 75.0;
            let mut enc = webp::AnimEncoder::new(w, h, &config);
            let step = (1000.0 / self.fps as f64).round() as i32;
            for (i, rgba) in self.frames.iter().enumerate() {
                enc.add_frame(webp::AnimFrame::from_rgba(rgba, w, h, i as i32 * step));
            }
            let data = enc
                .try_encode()
                .map_err(|e| anyhow::anyhow!("WebP encode: {e:?}"))?;
            std::fs::write(&self.path, &*data).context("writing the WebP file")?;
            Ok(())
        }
    }

    /// True unless the output extension is an animated image (which carries no audio).
    fn is_video_ext(file: &str) -> bool {
        let ext = std::path::Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        !matches!(ext.as_str(), "gif" | "webp")
    }

    /// Build the output sink from the file extension: `.gif`/`.webp` give animated images,
    /// anything else an FFmpeg-muxed video (with an AAC stream when `audio`). Returns the
    /// sink and a label for the log.
    fn make_sink(
        args: &RecordArgs,
        mode: video::Mode,
        audio: bool,
    ) -> Result<(Box<dyn FrameSink>, String)> {
        let fps = args.fps.max(1);
        if !is_video_ext(&args.file) {
            let ext = std::path::Path::new(&args.file)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            return match ext.as_str() {
                "gif" => Ok((Box::new(GifSink::new(&args.file, fps)?), "GIF".into())),
                _ => Ok((Box::new(WebpSink::new(&args.file, fps)), "WebP".into())),
            };
        }
        let enc = VideoEncoder::new(
            &args.file,
            video::Options {
                backend: args.encoder.into(),
                fps,
                mode,
                device: Some(args.device.clone().into()),
                audio,
            },
        )?;
        let label = format!("{:?}", enc.resolved_backend()?);
        Ok((Box::new(enc), label))
    }

    /// Whether audio recording is wanted: not `--no-audio`, and a video (not image) file.
    #[cfg(feature = "audio")]
    fn audio_wanted(args: &RecordArgs) -> bool {
        !args.no_audio && is_video_ext(&args.file)
    }

    #[derive(Args)]
    pub struct RecordArgs {
        /// Interactively select a region on a frozen overlay (drag, Esc to cancel).
        #[arg(short = 's', long, group = "source")]
        select: bool,
        /// Record this named output (e.g. `DP-4`). Defaults to the only output.
        #[arg(short = 'o', long, value_name = "NAME", group = "source")]
        output: Option<String>,
        /// Record this logical region, `"X,Y WxH"` (single output for now).
        #[arg(short = 'g', long, value_name = "GEOM", group = "source")]
        geometry: Option<String>,
        /// Record the window with this `ext-foreign-toplevel` identifier — follows it
        /// across workspaces and even when occluded.
        #[arg(short = 'w', long, value_name = "ID", group = "source")]
        window: Option<String>,
        /// Launch `wlr-chooser` to pick a window to record.
        #[arg(long, group = "source")]
        pick_window: bool,
        /// Record the active (focused) window's area — needs compositor focus info.
        #[arg(short = 'a', long, group = "source")]
        active_window: bool,
        /// Record the focused output — needs compositor focus info.
        #[arg(long, group = "source")]
        current_output: bool,
        /// Encoder backend. `auto` prefers hardware (NVENC, then VAAPI) over software.
        #[arg(long, value_enum, default_value_t = Enc::Auto)]
        encoder: Enc,
        /// DRM render node for the VAAPI backend.
        #[arg(long, value_name = "PATH", default_value = "/dev/dri/renderD128")]
        device: String,
        /// Output frame rate (playback rate; also the rate-control hint).
        #[arg(long, default_value_t = 30)]
        fps: u32,
        /// Timelapse: sample one frame every INTERVAL (e.g. `2s`, `500ms`, `1m`) and
        /// play them back at `--fps`, so the footage is sped up.
        #[arg(long, value_name = "INTERVAL")]
        timelapse: Option<String>,
        /// Stop automatically after this many seconds (otherwise: Ctrl-C).
        #[arg(short = 'd', long, value_name = "SECS")]
        duration: Option<f64>,
        /// Don't record audio (it is on by default for a video file).
        #[cfg(feature = "audio")]
        #[arg(long)]
        no_audio: bool,
        /// Capture this PipeWire audio node instead of the default sink monitor (system
        /// audio) — e.g. a microphone source.
        #[cfg(feature = "audio")]
        #[arg(long, value_name = "NODE")]
        audio_source: Option<String>,
        /// Destination file; the format is inferred from its extension. `.mp4`/`.mkv`
        /// (H.264 video) or animated `.gif`/`.webp` (downscaled; best for a region).
        #[arg(value_name = "FILE")]
        file: String,
    }

    #[derive(Clone, Copy, ValueEnum)]
    pub enum Enc {
        Auto,
        Nvenc,
        Vaapi,
        Software,
    }

    impl From<Enc> for video::Backend {
        fn from(e: Enc) -> Self {
            match e {
                Enc::Auto => video::Backend::Auto,
                Enc::Nvenc => video::Backend::Nvenc,
                Enc::Vaapi => video::Backend::Vaapi,
                Enc::Software => video::Backend::Software,
            }
        }
    }

    /// What and how to capture: an output (optionally cropped to a region) or a window.
    enum Target {
        /// Record `output`, optionally cropping each frame to a physical sub-rect.
        Output {
            output: Output,
            crop: Option<Region>,
        },
        /// Record the toplevel with this identifier (follows it).
        Window(String),
    }

    impl Target {
        fn label(&self) -> String {
            match self {
                Target::Output { output, crop: None } => format!("output {}", output.name),
                Target::Output {
                    output,
                    crop: Some(c),
                } => {
                    format!("region {}x{} on {}", c.w, c.h, output.name)
                }
                Target::Window(id) => format!("window {id}"),
            }
        }
    }

    /// Parse a human interval like `2s`, `500ms`, `1m`, `1.5s` into a `Duration`.
    fn parse_interval(s: &str) -> Result<Duration> {
        let s = s.trim();
        let err = || anyhow::anyhow!("invalid interval '{s}' (try e.g. 2s, 500ms, 1m)");
        let (num, mult) = if let Some(n) = s.strip_suffix("ms") {
            (n, 0.001)
        } else if let Some(n) = s.strip_suffix('s') {
            (n, 1.0)
        } else if let Some(n) = s.strip_suffix('m') {
            (n, 60.0)
        } else {
            (s, 1.0) // bare number = seconds
        };
        let secs: f64 = num.trim().parse().map_err(|_| err())?;
        if !(secs.is_finite() && secs > 0.0) {
            return Err(err());
        }
        Ok(Duration::from_secs_f64(secs * mult))
    }

    /// Resolve a logical region to the output its top-left corner sits on, plus the
    /// physical crop rectangle within that output's capture (single output for now).
    fn region_target(client: &wl::Client, region: Region) -> Result<Target> {
        if region.is_empty() {
            bail!("empty region");
        }
        let corner = Region {
            x: region.x,
            y: region.y,
            w: 1,
            h: 1,
        };
        let output = client
            .outputs()
            .iter()
            .find(|o| o.logical_rect().intersect(&corner).is_some())
            .cloned()
            .context("the region's top-left corner is on no output")?;
        let clipped = region
            .intersect(&output.logical_rect())
            .context("region does not overlap its output")?;
        if clipped != region {
            eprintln!(
                "wlr-shot: region spans multiple outputs; recording the {}x{} part on {}",
                clipped.w, clipped.h, output.name
            );
        }
        let crop = capture::logical_to_physical(&output, clipped);
        Ok(Target::Output {
            output,
            crop: Some(crop),
        })
    }

    /// Resolve the CLI source flags (an exclusive group) to a [`Target`].
    fn resolve_target(client: &mut wl::Client, args: &RecordArgs) -> Result<Target> {
        if args.select {
            let caps = capture::capture_all(client, capture::DEFAULT_BUDGET)?;
            match wlr_capture::overlay::select_region(&caps)? {
                Some(region) => region_target(client, region),
                None => std::process::exit(1), // cancelled
            }
        } else if let Some(geo) = &args.geometry {
            region_target(client, capture::parse_geometry(geo)?)
        } else if args.active_window {
            region_target(client, active_window_rect()?)
        } else if let Some(id) = &args.window {
            Ok(Target::Window(id.clone()))
        } else if args.pick_window {
            Ok(Target::Window(pick_window()?))
        } else {
            let name = if args.current_output {
                Some(focused_output()?)
            } else {
                args.output.clone()
            };
            Ok(Target::Output {
                output: resolve_output(client, name.as_deref())?,
                crop: None,
            })
        }
    }

    /// Find a named output, or the sole output if unnamed (else list the names).
    fn resolve_output(client: &wl::Client, name: Option<&str>) -> Result<Output> {
        let outputs = client.outputs();
        match name {
            Some(n) => outputs
                .iter()
                .find(|o| o.name == n)
                .cloned()
                .with_context(|| format!("output '{n}' not found")),
            None => match outputs {
                [single] => Ok(single.clone()),
                [] => bail!("no outputs available"),
                many => {
                    let names: Vec<&str> = many.iter().map(|o| o.name.as_str()).collect();
                    bail!(
                        "multiple outputs; specify -o NAME among: {}",
                        names.join(", ")
                    )
                }
            },
        }
    }

    pub fn record(args: RecordArgs) -> Result<()> {
        let mut client = wl::Client::connect().context("Wayland connection")?;
        client.refresh().ok();

        let target = resolve_target(&mut client, &args)?;
        let mode = match &args.timelapse {
            Some(_) => video::Mode::Timelapse,
            None => video::Mode::Record,
        };
        let interval = args.timelapse.as_deref().map(parse_interval).transpose()?;

        // Start audio capture first, so the muxer only adds an AAC stream if sound is
        // actually flowing (a failed capture falls back to a silent video, not a broken
        // empty stream). Only for video output (animated images carry no audio).
        #[cfg(feature = "audio")]
        let audio = if audio_wanted(&args) {
            match wlr_capture::audio::AudioCapture::start(args.audio_source.clone()) {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("wlr-shot: recording without audio ({e})");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(feature = "audio")]
        let have_audio = audio.is_some();
        #[cfg(not(feature = "audio"))]
        let have_audio = false;

        let (mut sink, fmt) = make_sink(&args, mode, have_audio)?;
        eprintln!(
            "wlr-shot: recording {} to {} ({fmt}{}). Press Ctrl-C to stop.",
            target.label(),
            args.file,
            if have_audio { " + audio" } else { "" },
        );

        // Ctrl-C flips the stop flag so we finalise the file cleanly.
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        ctrlc::set_handler(move || s.store(true, Ordering::SeqCst))
            .context("installing Ctrl-C handler")?;

        let start = Instant::now();
        let deadline = args.duration.map(|d| start + Duration::from_secs_f64(d));
        let crop = match &target {
            Target::Output { crop, .. } => *crop,
            Target::Window(_) => None,
        };
        let stream_source = match &target {
            Target::Output { output, .. } => stream::Source::Output(output.name.clone()),
            Target::Window(id) => stream::Source::Toplevel(id.clone()),
        };

        // Capture is damage-driven (a frame only arrives when the source changes), so
        // we emit on a fixed cadence instead: a normal recording ticks at 1/fps for a
        // constant frame rate (repeating the last frame through static stretches);
        // a timelapse ticks at its interval and plays the samples back at --fps.
        let frame_interval =
            interval.unwrap_or_else(|| Duration::from_secs_f64(1.0 / args.fps.max(1) as f64));

        let mut rb: Option<GpuReadback> = None;
        let mut stream = stream::Stream::new(stream_source, APPEAR_GRACE);
        let mut frames = 0u64;
        let mut last_img: Option<CapturedImage> = None; // most recent captured frame
        let mut next_tick: Option<Duration> = None; // scheduled time of the next emit
        let mut last_log = start;

        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    break;
                }
            }

            // Poll for new content, but never overshoot the next emit tick.
            let budget = match next_tick {
                Some(nt) => nt
                    .saturating_sub(start.elapsed())
                    .clamp(Duration::from_millis(1), ROUND),
                None => ROUND,
            };
            let step = stream.step(&mut client, budget);
            for frame in step.frames {
                let mut img = stream::decode_frame(&mut rb, frame)?;
                if let Some(c) = crop {
                    img = img.crop(c);
                }
                last_img = Some(img);
                // Anchor the cadence to the first captured frame (no startup catch-up).
                next_tick.get_or_insert_with(|| start.elapsed());
            }
            if let Some(end) = step.end {
                match end {
                    stream::End::NeverAppeared => {
                        bail!("source did not appear within {}s", APPEAR_GRACE.as_secs())
                    }
                    stream::End::SourceGone => break, // window closed / output gone
                }
            }

            // Drain captured audio into the encoder (muxed on the next video push).
            #[cfg(feature = "audio")]
            if let Some(cap) = &audio {
                let pcm = cap.drain();
                if !pcm.is_empty() {
                    sink.push_audio(&pcm);
                }
            }

            // Emit the latest frame at every elapsed tick (repeating it when the
            // source is static, so the output keeps a steady frame rate).
            if let (Some(img), Some(mut nt)) = (last_img.as_ref(), next_tick) {
                while start.elapsed() >= nt {
                    sink.push(img, nt)?; // ts ignored in timelapse mode (sequential PTS)
                    frames += 1;
                    nt += frame_interval;
                }
                next_tick = Some(nt);
            }

            if last_log.elapsed() >= Duration::from_secs(1) {
                eprint!(
                    "\rwlr-shot: {frames} frames, {:.0}s ",
                    start.elapsed().as_secs_f64()
                );
                std::io::Write::flush(&mut std::io::stderr()).ok();
                last_log = Instant::now();
            }
        }

        // One last audio push (and stop capture) so the tail isn't dropped, then finalise.
        #[cfg(feature = "audio")]
        if let Some(cap) = &audio {
            let pcm = cap.drain();
            if !pcm.is_empty() {
                sink.push_audio(&pcm);
            }
        }
        #[cfg(feature = "audio")]
        drop(audio);

        sink.finish().context("finalising the output file")?;
        drop(sink); // flush GIF/WebP trailers held until the encoder is dropped
        eprintln!(
            "\rwlr-shot: saved {} ({frames} frames, {:.1}s)        ",
            args.file,
            start.elapsed().as_secs_f64()
        );
        Ok(())
    }
}

#[cfg(feature = "video")]
use record_impl::{RecordArgs, record};
