//! wlr-peek — inspect the screen on wlroots compositors.
//!
//! The "look at the screen" companion to `wlr-shot` (which produces image artifacts).
//! Built on the shared `wlr-capture` engine: `color` (pixel colour picker / pipette),
//! `loupe` (frozen full-screen magnifier), `mirror` (live floating window/region
//! mirror, the former `wlr-pip`), and `ocr` (Tesseract text recognition).

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use wlr_capture::capture::{self, DEFAULT_BUDGET};
use wlr_capture::overlay;
use wlr_capture::wl::{self, Region};

mod i18n;

#[derive(Parser)]
#[command(
    name = "wlr-peek",
    version,
    about = "Inspect the screen on wlroots (colour picker, OCR)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Pick a colour from anywhere on screen (pipette), with a magnifying loupe.
    Color(ColorArgs),
    /// Magnify the screen around the cursor; scroll to zoom, Esc to quit.
    Loupe,
    /// Mirror a window live in a floating, always-on-top window (picture-in-picture).
    Mirror(MirrorArgs),
    /// Select a region (or point) with the mouse and print its geometry — a native
    /// slurp replacement (`X,Y WxH`). Exit 1 if cancelled.
    Region(RegionArgs),
    /// Recognise text in a screen region (OCR, via Tesseract).
    #[cfg(feature = "ocr")]
    Ocr(OcrArgs),
    /// Watch a region, window or output and act when it changes (or stops changing).
    #[cfg(feature = "watch")]
    Watch(WatchArgs),
    /// Find text on screen (OCR) and print where it is — a visual grep.
    #[cfg(feature = "ocr")]
    Grep(GrepArgs),
    /// Report which capture protocols the current compositor supports.
    Doctor,
    /// Internal: serve a clipboard selection read from stdin. Spawned detached by
    /// `color --clipboard`; not meant to be run by hand.
    #[command(hide = true)]
    ClipboardServe {
        /// MIME type to advertise for the bytes on stdin.
        #[arg(long)]
        mime: String,
    },
}

#[derive(Args)]
struct ColorArgs {
    /// How to format the picked colour.
    #[arg(short = 'f', long, value_enum, default_value_t = Format::Hex)]
    format: Format,
    /// Copy the colour to the Wayland clipboard instead of printing it. Runs a small
    /// background daemon that serves the selection until another client replaces it.
    #[arg(short = 'c', long)]
    clipboard: bool,
    /// With `--clipboard`: serve the selection in the foreground (don't detach).
    /// Mainly for scripts and debugging; the process blocks until replaced.
    #[arg(long, requires = "clipboard")]
    clipboard_foreground: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// `#RRGGBB`
    Hex,
    /// `rgb(R, G, B)`
    Rgb,
    /// `R G B`
    Plain,
}

pub fn main() {
    crate::i18n::init();
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Color(args) => color(args),
        Cmd::Loupe => loupe(),
        Cmd::Mirror(args) => mirror(args),
        Cmd::Region(args) => region(args),
        #[cfg(feature = "ocr")]
        Cmd::Ocr(args) => ocr(args),
        #[cfg(feature = "watch")]
        Cmd::Watch(args) => watch(args),
        #[cfg(feature = "ocr")]
        Cmd::Grep(args) => grep(args),
        Cmd::Doctor => doctor(),
        Cmd::ClipboardServe { mime } => clipboard_serve(&mime),
    };
    if let Err(e) = res {
        eprintln!("wlr-peek: {e:#}");
        std::process::exit(1);
    }
}

fn color(args: ColorArgs) -> Result<()> {
    // Open the overlay's connection BEFORE capturing. Capture can briefly open and
    // drop a transient EGL connection (the GPU readback); EGL caches its display by the
    // `wl_display` pointer, so an overlay connection opened afterwards may alias the
    // freed one and fail with `eglCreateWindowSurface: BadAlloc`. Establishing the
    // overlay connection first keeps its `EGLDisplay` valid. (See overlay::pick_point_on.)
    let conn = wlr_capture::Connection::connect_to_env().context("Wayland connection")?;
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();

    // Freeze every output, let the user aim and click a pixel on the loupe overlay,
    // then read that pixel back from the very same frozen capture.
    let caps = capture::capture_all(&mut client, DEFAULT_BUDGET)?;
    let Some((x, y)) = overlay::pick_point_on(&conn, &caps, &crate::tr!("overlay-pick-hint"))?
    else {
        std::process::exit(1); // cancelled
    };

    let px = capture::composite(&caps, Region { x, y, w: 1, h: 1 }).context("reading pixel")?;
    let [r, g, b, _] = px.pixel(0, 0).context("pixel off-screen")?;
    let text = format_color(args.format, r, g, b);

    if args.clipboard {
        copy_text(text, args.clipboard_foreground).context("copying to clipboard")?;
    } else {
        println!("{text}");
    }
    Ok(())
}

/// Freeze the screen and show a full-screen magnifier that follows the cursor (scroll
/// to zoom, Esc to quit). Frozen rather than live: a full-screen live magnifier would
/// capture its own output. For a live zoom of a fixed region, use `mirror -g`.
fn loupe() -> Result<()> {
    // Overlay connection first, then capture — see `color` for why (EGL BadAlloc).
    let conn = wlr_capture::Connection::connect_to_env().context("Wayland connection")?;
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();
    let caps = capture::capture_all(&mut client, DEFAULT_BUDGET)?;
    overlay::magnify_on(&conn, &caps, &crate::tr!("overlay-magnify-hint")).map_err(Into::into)
}

#[derive(Args)]
struct RegionArgs {
    /// Pick a single point instead of a region (prints `X,Y`).
    #[arg(short = 'p', long)]
    point: bool,
    /// Output format with `%x`/`%y`/`%w`/`%h` placeholders (`%w`/`%h` are 0 for a
    /// point). Default: `%x,%y %wx%h` for a region, `%x,%y` for a point.
    #[arg(short = 'f', long, value_name = "FMT")]
    format: Option<String>,
}

/// Select a region (or a point) with the mouse on a frozen screen and print its
/// geometry — a native slurp replacement. Exits 1 if cancelled (Esc). Reuses the same
/// frozen overlay as `wlr-shot -s` and the colour picker.
fn region(args: RegionArgs) -> Result<()> {
    // Overlay connection first, then capture — see `color` for why (EGL BadAlloc).
    let conn = wlr_capture::Connection::connect_to_env().context("Wayland connection")?;
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();
    let caps = capture::capture_all(&mut client, DEFAULT_BUDGET)?;
    if args.point {
        let (x, y) = match overlay::pick_point_on(&conn, &caps, &crate::tr!("overlay-pick-hint"))? {
            Some(p) => p,
            None => std::process::exit(1),
        };
        let fmt = args.format.as_deref().unwrap_or("%x,%y");
        println!("{}", fill_geometry(fmt, x, y, 0, 0));
    } else {
        let r = match overlay::select_region_on(&conn, &caps, &crate::tr!("overlay-region-hint"))? {
            Some(r) => r,
            None => std::process::exit(1),
        };
        let fmt = args.format.as_deref().unwrap_or("%x,%y %wx%h");
        println!("{}", fill_geometry(fmt, r.x, r.y, r.w as i32, r.h as i32));
    }
    Ok(())
}

/// Substitute `%x`/`%y`/`%w`/`%h` in a slurp-style format string.
fn fill_geometry(fmt: &str, x: i32, y: i32, w: i32, h: i32) -> String {
    fmt.replace("%x", &x.to_string())
        .replace("%y", &y.to_string())
        .replace("%w", &w.to_string())
        .replace("%h", &h.to_string())
}

#[derive(Args)]
struct MirrorArgs {
    /// The `ext-foreign-toplevel` identifier of the window to mirror (as printed by
    /// `wlr-chooser`). With no source flag, the chooser is launched to pick one.
    #[arg(group = "source")]
    id: Option<String>,
    /// Select a region with the mouse, then mirror it (a frozen-screen drag, like
    /// `wlr-shot -s`) — no need for slurp.
    #[arg(short = 's', long, group = "source")]
    select: bool,
    /// Mirror (and magnify) this logical region, `"X,Y WxH"` (the format slurp prints).
    #[arg(short = 'g', long, value_name = "GEOM", group = "source")]
    geometry: Option<String>,
    /// Mirror this whole named output / screen (e.g. `DP-4`).
    #[arg(short = 'o', long, value_name = "NAME", group = "source")]
    output: Option<String>,
    /// Pick a window to mirror via the chooser (same as no source flag).
    #[arg(short = 'w', long = "pick-window", group = "source")]
    pick_window: bool,
    /// Mirror the active (focused) window's area — needs compositor focus info.
    #[cfg(any(feature = "ocr", feature = "watch"))]
    #[arg(short = 'a', long, group = "source")]
    active_window: bool,
    /// Mirror the focused output — needs compositor focus info.
    #[cfg(any(feature = "ocr", feature = "watch"))]
    #[arg(long, group = "source")]
    current_output: bool,
    /// Magnification factor for region/output sources (window starts at region × zoom).
    #[arg(long, default_value_t = 2.0)]
    zoom: f32,
    /// For a region (`-s`/`-g`): whether the loupe follows the **output** (shows
    /// whatever workspace is on it) or the **window** under the region (captures that
    /// toplevel, so it follows the window across moves/workspaces). `window` needs
    /// compositor focus info.
    #[arg(long, value_enum, default_value_t = Follow::Output)]
    follow: Follow,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Follow {
    /// Capture the output; the loupe shows the visible workspace.
    Output,
    /// Capture the window under the region; the loupe follows that window.
    Window,
}

/// Mirror a window (or, with `-g`, a region) live in a floating, always-on-top
/// window. One mirror per window (an advisory lock makes a second launch for the
/// same window a no-op).
fn mirror(args: MirrorArgs) -> Result<()> {
    // Interactive region select runs its own EGL overlay, then the mirror opens another.
    // EGL caches its display by the `wl_display` pointer, so a *second* connection there
    // can alias the selector's freed one (`eglCreateWindowSurface: BadAlloc`). We share
    // one connection (one `EGLDisplay`) between the two — like the per-output overlay
    // surfaces already do.
    if args.select {
        let conn = wlr_capture::Connection::connect_to_env().context("Wayland connection")?;
        let mut client = wl::Client::connect().context("Wayland connection")?;
        client.refresh().ok();
        let caps = capture::capture_all(&mut client, DEFAULT_BUDGET)?;
        let region =
            match overlay::select_region_on(&conn, &caps, &crate::tr!("overlay-region-hint"))? {
                Some(r) => r,
                None => std::process::exit(1), // cancelled
            };
        let (source, config) = build_source(&client, region, args.zoom, args.follow)?;
        return wlr_capture::mirror::run_on(&conn, source, config).map_err(Into::into);
    }
    // Other region / output sources → a live region magnifier (single EGL context).
    if let Some(region) = resolve_mirror_region(&args)? {
        return mirror_region(region, args.zoom, args.follow);
    }
    // Otherwise a window: an explicit id, else the chooser (incl. `-w`).
    let id = match args.id {
        Some(id) => id,
        None => match pick_via_chooser() {
            Some(id) => id,
            None => std::process::exit(1), // cancelled or chooser unavailable
        },
    };
    let _lock = match mirror_lock(&id) {
        Some(lock) => lock,
        None => return Ok(()), // already mirroring this window
    };
    let (label, icon) = resolve_window(&id);
    wlr_capture::mirror::run(
        wlr_capture::mirror::Source::Toplevel(id),
        wlr_capture::mirror::Config {
            app_id: "wlr-peek-mirror".to_string(),
            label,
            icon,
            relaunch: vec!["mirror".to_string()],
            loading: crate::tr!("loading"),
            source_gone: crate::tr!("pip-gone"),
        },
    )
    .map_err(Into::into)
}

/// Resolve a non-interactive region/output mirror source to a [`Region`] in global
/// logical coordinates, or `None` for a window source (handled by the caller). The
/// interactive `-s` select is handled in [`mirror`] (it shares one connection with the
/// mirror, so it isn't here).
fn resolve_mirror_region(args: &MirrorArgs) -> Result<Option<Region>> {
    if let Some(geo) = &args.geometry {
        return Ok(Some(capture::parse_geometry(geo)?));
    }
    if let Some(name) = &args.output {
        return Ok(Some(output_rect(name)?));
    }
    #[cfg(any(feature = "ocr", feature = "watch"))]
    {
        if args.active_window {
            return Ok(Some(active_window_rect()?));
        }
        if args.current_output {
            return Ok(Some(output_rect(&focused_output()?)?));
        }
    }
    Ok(None)
}

/// The logical rectangle of the named output, as a [`Region`].
fn output_rect(name: &str) -> Result<Region> {
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();
    client
        .outputs()
        .iter()
        .find(|o| o.name == name)
        .map(|o| o.logical_rect())
        .with_context(|| format!("no output named `{name}`"))
}

/// Mirror a fixed logical region live, magnified by `zoom`. Resolves the output the
/// region's top-left corner sits on (mono-output for now), clips the region to it,
/// and streams that output, showing only the region's sub-rectangle.
fn mirror_region(region: Region, zoom: f32, follow: Follow) -> Result<()> {
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();
    let (source, config) = build_source(&client, region, zoom, follow)?;
    wlr_capture::mirror::run(source, config).map_err(Into::into)
}

/// Build the mirror source for a region: follow the **window** under it when asked (and
/// one is found there), else follow the **output** (the default). Falls back to output
/// with a notice if no window is under the region.
fn build_source(
    client: &wl::Client,
    region: Region,
    zoom: f32,
    follow: Follow,
) -> Result<(wlr_capture::mirror::Source, wlr_capture::mirror::Config)> {
    if follow == Follow::Window {
        #[cfg(any(feature = "ocr", feature = "watch"))]
        {
            if let Some(sc) = region_window_source(client, region, zoom)? {
                return Ok(sc);
            }
            eprintln!("wlr-peek: no window under the region — mirroring the output instead");
        }
        #[cfg(not(any(feature = "ocr", feature = "watch")))]
        anyhow::bail!("--follow window needs focus support (built without ocr/watch)");
    }
    region_source(client, region, zoom)
}

/// Resolve a region to a window-following mirror source: find the window under the
/// region's centre (compositor IPC), match it to a foreign-toplevel handle, and crop to
/// the region's sub-rectangle within the window's content. `None` if there's no window
/// there, or no matching toplevel handle (the caller falls back to output mode).
#[cfg(any(feature = "ocr", feature = "watch"))]
fn region_window_source(
    client: &wl::Client,
    region: Region,
    zoom: f32,
) -> Result<Option<(wlr_capture::mirror::Source, wlr_capture::mirror::Config)>> {
    let (cx, cy) = (
        region.x + region.w as i32 / 2,
        region.y + region.h as i32 / 2,
    );
    let Some(backend) = wlr_capture::focus::detect() else {
        return Ok(None);
    };
    let Some(win) = backend.window_at(cx, cy) else {
        return Ok(None);
    };
    // Match the compositor's window (app_id + title) to a foreign-toplevel handle.
    let Some(tl) = client
        .toplevels()
        .iter()
        .find(|t| t.app_id == win.app_id && t.title == win.title)
    else {
        return Ok(None);
    };

    // The region as a normalized sub-rectangle of the window's content.
    let wr = win.rect;
    let nx = |v: i32| ((v - wr.x) as f32 / wr.w.max(1) as f32).clamp(0.0, 1.0);
    let ny = |v: i32| ((v - wr.y) as f32 / wr.h.max(1) as f32).clamp(0.0, 1.0);
    let crop = [
        nx(region.x),
        ny(region.y),
        nx(region.x + region.w as i32),
        ny(region.y + region.h as i32),
    ];

    let label = if win.title.is_empty() {
        win.app_id.clone()
    } else {
        win.title.clone()
    };
    Ok(Some((
        wlr_capture::mirror::Source::ToplevelRegion {
            id: tl.identifier.clone(),
            crop,
            region_w: region.w,
            region_h: region.h,
            zoom,
        },
        wlr_capture::mirror::Config {
            app_id: "wlr-peek-mirror".to_string(),
            label,
            icon: None,
            relaunch: vec![],
            loading: crate::tr!("loading"),
            source_gone: crate::tr!("pip-gone"),
        },
    )))
}

/// Resolve a logical region to a mirror [`Source::Region`](wlr_capture::mirror::Source)
/// (+ window config): the output the region's top-left corner sits on (mono-output for
/// now), clipped to it, with the region as a normalized sub-rectangle of that output.
fn region_source(
    client: &wl::Client,
    region: Region,
    zoom: f32,
) -> Result<(wlr_capture::mirror::Source, wlr_capture::mirror::Config)> {
    if region.is_empty() {
        anyhow::bail!("empty region");
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
    let lr = output.logical_rect();
    let region = region
        .intersect(&lr)
        .context("region does not overlap its output")?;

    // Normalized sub-rectangle of the region within the output (scale-independent).
    let crop = [
        (region.x - lr.x) as f32 / lr.w.max(1) as f32,
        (region.y - lr.y) as f32 / lr.h.max(1) as f32,
        (region.x + region.w as i32 - lr.x) as f32 / lr.w.max(1) as f32,
        (region.y + region.h as i32 - lr.y) as f32 / lr.h.max(1) as f32,
    ];

    Ok((
        wlr_capture::mirror::Source::Region {
            output: output.name.clone(),
            crop,
            region_w: region.w,
            region_h: region.h,
            zoom,
        },
        wlr_capture::mirror::Config {
            app_id: "wlr-peek-mirror".to_string(),
            label: format!("{},{} {}×{}", region.x, region.y, region.w, region.h),
            icon: None,
            relaunch: vec![], // no re-pick in region mode
            loading: crate::tr!("loading"),
            source_gone: crate::tr!("pip-gone"),
        },
    ))
}

/// Launch `wlr-chooser --windows` to pick a window; parse its `Window: <id>` stdout
/// contract. Prefers a `wlr-chooser` next to our own binary, else one on `PATH`.
fn pick_via_chooser() -> Option<String> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("wlr-chooser")))
        .filter(|p| p.exists());
    let mut cmd = match sibling {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("wlr-chooser"),
    };
    let out = cmd.arg("--windows").output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("Window: ").map(|id| id.trim().to_string()))
        .filter(|id| !id.is_empty())
}

/// Acquire the single-instance advisory lock for this window's mirror; the held file
/// (keep it alive) or `None` if another mirror already owns it.
fn mirror_lock(identifier: &str) -> Option<std::fs::File> {
    use rustix::fs::{FlockOperation, flock};
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let safe: String = identifier
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join(format!("wlr-peek-mirror-{safe}.lock")))
        .ok()?;
    flock(&f, FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(f)
}

/// Look up the target window's app-id/title (label) and app icon. Falls back to a
/// generic label if it isn't found right now (it may map shortly).
fn resolve_window(identifier: &str) -> (String, Option<(u32, u32, Vec<u8>)>) {
    let mut client = match wl::Client::connect() {
        Ok(c) => c,
        Err(_) => return ("wlr-peek".to_string(), None),
    };
    let _ = client.refresh();
    let Some(t) = client
        .toplevels()
        .iter()
        .find(|t| t.identifier == identifier)
    else {
        return ("wlr-peek".to_string(), None);
    };
    let label = if t.app_id.is_empty() {
        t.title.clone()
    } else if t.title.is_empty() {
        t.app_id.clone()
    } else {
        format!("{} — {}", t.app_id, t.title)
    };
    let icon =
        wlr_capture::icons::resolve(&t.app_id).and_then(|p| wlr_capture::icons::load(&p, 64));
    (label, icon)
}

/// Render a colour in the requested format.
fn format_color(fmt: Format, r: u8, g: u8, b: u8) -> String {
    match fmt {
        Format::Hex => format!("#{r:02X}{g:02X}{b:02X}"),
        Format::Rgb => format!("rgb({r}, {g}, {b})"),
        Format::Plain => format!("{r} {g} {b}"),
    }
}

/// Put `text` on the Wayland clipboard (as UTF-8 text). Unless `foreground`, detach a
/// background daemon (a re-exec in `clipboard-serve` mode) that serves the selection
/// until replaced — the clipboard is pull-based, so it must outlive this process.
fn copy_text(text: String, foreground: bool) -> Result<()> {
    const MIME: &str = "text/plain;charset=utf-8";
    if foreground {
        return wlr_capture::clipboard::serve(MIME, text.into_bytes()).map_err(Into::into);
    }
    wlr_capture::clipboard::spawn_detached(text.as_bytes(), &["clipboard-serve", "--mime", MIME])
        .map_err(Into::into)
}

#[cfg(feature = "ocr")]
#[derive(Args)]
struct OcrArgs {
    /// OCR this logical region, `"X,Y WxH"` (the format slurp prints), stitched
    /// across the outputs it covers.
    #[arg(short = 'g', long, value_name = "GEOM", group = "source")]
    geometry: Option<String>,
    /// OCR this whole named output (e.g. `DP-4`).
    #[arg(short = 'o', long, value_name = "NAME", group = "source")]
    output: Option<String>,
    /// OCR the active (focused) window — needs compositor focus info.
    #[arg(short = 'a', long, group = "source")]
    active_window: bool,
    /// OCR the focused output — needs compositor focus info.
    #[arg(long, group = "source")]
    current_output: bool,
    /// Tesseract language(s), e.g. `eng`, `fra`, `fra+eng` (needs the tessdata pack).
    #[arg(short = 'l', long, default_value = "eng")]
    lang: String,
    /// Copy the recognised text to the Wayland clipboard instead of printing it.
    #[arg(short = 'c', long)]
    clipboard: bool,
    /// With `--clipboard`: serve the selection in the foreground (don't detach).
    #[arg(long, requires = "clipboard")]
    clipboard_foreground: bool,
}

#[cfg(feature = "ocr")]
fn ocr(args: OcrArgs) -> Result<()> {
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();

    let img = ocr_source(&mut client, &args)?;
    let text = clean_ocr(&run_ocr(&img, &args.lang)?);

    if args.clipboard {
        copy_text(text, args.clipboard_foreground).context("copying to clipboard")?;
    } else {
        println!("{text}");
    }
    Ok(())
}

/// Resolve the OCR source (the flags form an exclusive group; with none, the default
/// is the interactive region selector) to a captured image.
#[cfg(feature = "ocr")]
fn ocr_source(client: &mut wl::Client, args: &OcrArgs) -> Result<wl::CapturedImage> {
    if let Some(geo) = &args.geometry {
        capture::capture_region(client, capture::parse_geometry(geo)?, DEFAULT_BUDGET)
            .map_err(Into::into)
    } else if let Some(name) = &args.output {
        capture::capture_output(client, Some(name), DEFAULT_BUDGET).map_err(Into::into)
    } else if args.active_window {
        capture::capture_region(client, active_window_rect()?, DEFAULT_BUDGET).map_err(Into::into)
    } else if args.current_output {
        capture::capture_output(client, Some(&focused_output()?), DEFAULT_BUDGET)
            .map_err(Into::into)
    } else {
        // Default (no source flag): freeze, let the user drag a region.
        let caps = capture::capture_all(client, DEFAULT_BUDGET)?;
        match overlay::select_region(&caps, &crate::tr!("overlay-region-hint"))? {
            Some(region) => capture::composite(&caps, region).map_err(Into::into),
            None => std::process::exit(1), // cancelled
        }
    }
}

/// Tidy Tesseract output: drop the form-feed page separators it appends, trim
/// trailing whitespace from each line, collapse runs of blank lines to at most one,
/// and trim the ends. Tesseract is liberal with blank lines and a trailing `\x0c`.
#[cfg(feature = "ocr")]
fn clean_ocr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let line = line.trim_end_matches([' ', '\t', '\r', '\u{0c}']);
        if line.is_empty() {
            blank_run += 1;
            if blank_run == 1 {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// Build a Tesseract instance with the captured image loaded, ready to read text or
/// positions. PNG-encodes the capture (lossless, cheap) since Tesseract reads an
/// encoded image from memory.
#[cfg(feature = "ocr")]
fn ocr_engine(img: &wl::CapturedImage, lang: &str) -> Result<leptess::LepTess> {
    use std::io::Cursor;
    if img.width == 0 || img.height == 0 {
        anyhow::bail!("empty image (region off-screen?)");
    }
    let rgba = image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone())
        .context("inconsistent dimensions/buffer")?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .context("encoding image for OCR")?;

    let mut lt = leptess::LepTess::new(None, lang)
        .map_err(|e| anyhow::anyhow!("Tesseract init (language '{lang}'): {e:?}"))?;
    lt.set_image_from_mem(&png)
        .map_err(|e| anyhow::anyhow!("loading image into Tesseract: {e:?}"))?;
    // Screen captures carry no DPI metadata; set one (after the image) so Tesseract
    // skips guessing it. Must follow set_image, or libtesseract warns and ignores it.
    lt.set_source_resolution(96);
    Ok(lt)
}

/// Run Tesseract over a captured image and return the recognised text.
#[cfg(feature = "ocr")]
fn run_ocr(img: &wl::CapturedImage, lang: &str) -> Result<String> {
    ocr_engine(img, lang)?
        .get_utf8_text()
        .context("extracting text")
}

/// Visual grep: OCR a source and print where matching text is.
#[cfg(feature = "ocr")]
#[derive(Args)]
struct GrepArgs {
    /// Substring to search for in the recognised text.
    pattern: String,
    /// Search this logical region, `"X,Y WxH"` (the format slurp prints).
    #[arg(short = 'g', long, value_name = "GEOM", group = "source")]
    geometry: Option<String>,
    /// Search this whole named output (e.g. `DP-4`).
    #[arg(short = 'o', long, value_name = "NAME", group = "source")]
    output: Option<String>,
    /// Search the active (focused) window — needs compositor focus info.
    #[arg(short = 'a', long, group = "source")]
    active_window: bool,
    /// Search the focused output — needs compositor focus info.
    #[arg(long, group = "source")]
    current_output: bool,
    /// Tesseract language(s), e.g. `eng`, `fra`, `fra+eng`.
    #[arg(short = 'l', long, default_value = "eng")]
    lang: String,
    /// Case-insensitive match.
    #[arg(short = 'i', long)]
    ignore_case: bool,
}

/// OCR the source and print each matching word as `X,Y WxH<TAB>text` in global
/// logical coordinates (slurp-compatible geometry, so it can feed other tools).
/// Exits 1 if nothing matched, like `grep`.
#[cfg(feature = "ocr")]
fn grep(args: GrepArgs) -> Result<()> {
    use std::io::Write;
    let mut client = wl::Client::connect().context("Wayland connection")?;
    client.refresh().ok();

    let (img, rect) = grep_source(&mut client, &args)?;
    let tsv = ocr_engine(&img, &args.lang)?
        .get_tsv_text(0)
        .context("extracting text positions")?;

    let needle = if args.ignore_case {
        args.pattern.to_lowercase()
    } else {
        args.pattern.clone()
    };
    // Map word boxes (image pixels) to global logical coordinates.
    let sx = rect.w as f64 / img.width.max(1) as f64;
    let sy = rect.h as f64 / img.height.max(1) as f64;
    let mut out = std::io::stdout().lock();
    let mut found = 0u32;
    for w in parse_tsv_words(&tsv) {
        let hay = if args.ignore_case {
            w.text.to_lowercase()
        } else {
            w.text.clone()
        };
        if !hay.contains(&needle) {
            continue;
        }
        let x = rect.x + (w.left as f64 * sx).round() as i32;
        let y = rect.y + (w.top as f64 * sy).round() as i32;
        let gw = (w.width as f64 * sx).round() as u32;
        let gh = (w.height as f64 * sy).round() as u32;
        // Stop quietly if stdout closes (e.g. piped to `head`), rather than panicking.
        if writeln!(out, "{x},{y} {gw}x{gh}\t{}", w.text).is_err() {
            return Ok(());
        }
        found += 1;
    }
    if found == 0 {
        std::process::exit(1); // no match, like grep
    }
    Ok(())
}

/// One OCR'd word with its bounding box, in image pixels.
#[cfg(feature = "ocr")]
struct TsvWord {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    text: String,
}

/// Parse Tesseract TSV, yielding word-level rows (level 5) with non-empty text.
/// Columns: level page block par line word left top width height conf text.
#[cfg(feature = "ocr")]
fn parse_tsv_words(tsv: &str) -> Vec<TsvWord> {
    let mut out = Vec::new();
    for line in tsv.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 12 || f[0] != "5" {
            continue;
        }
        let text = f[11].trim();
        if text.is_empty() {
            continue;
        }
        let (Ok(left), Ok(top), Ok(width), Ok(height)) =
            (f[6].parse(), f[7].parse(), f[8].parse(), f[9].parse())
        else {
            continue;
        };
        out.push(TsvWord {
            left,
            top,
            width,
            height,
            text: text.to_string(),
        });
    }
    out
}

/// Resolve the grep source to a captured image plus its logical rectangle (so word
/// boxes map back to global coordinates). Single output for now.
#[cfg(feature = "ocr")]
fn grep_source(client: &mut wl::Client, args: &GrepArgs) -> Result<(wl::CapturedImage, Region)> {
    if let Some(geo) = &args.geometry {
        let r = capture::parse_geometry(geo)?;
        Ok((capture::capture_region(client, r, DEFAULT_BUDGET)?, r))
    } else if let Some(name) = &args.output {
        let out = client
            .outputs()
            .iter()
            .find(|o| o.name == *name)
            .cloned()
            .with_context(|| format!("output '{name}' not found"))?;
        Ok((
            capture::capture_output(client, Some(name), DEFAULT_BUDGET)?,
            out.logical_rect(),
        ))
    } else if args.active_window {
        let r = active_window_rect()?;
        Ok((capture::capture_region(client, r, DEFAULT_BUDGET)?, r))
    } else if args.current_output {
        let name = focused_output()?;
        let out = client
            .outputs()
            .iter()
            .find(|o| o.name == name)
            .cloned()
            .with_context(|| format!("output '{name}' not found"))?;
        Ok((
            capture::capture_output(client, Some(&name), DEFAULT_BUDGET)?,
            out.logical_rect(),
        ))
    } else {
        let caps = capture::capture_all(client, DEFAULT_BUDGET)?;
        match overlay::select_region(&caps, &crate::tr!("overlay-region-hint"))? {
            Some(region) => Ok((capture::composite(&caps, region)?, region)),
            None => std::process::exit(1), // cancelled
        }
    }
}

/// The active window's logical rectangle, via compositor focus IPC.
#[cfg(any(feature = "ocr", feature = "watch"))]
fn active_window_rect() -> Result<Region> {
    let backend = wlr_capture::focus::detect()
        .context("focus info unavailable (unsupported compositor); try -s or -g")?;
    backend
        .active_window_rect()
        .with_context(|| format!("no active window detected (via {})", backend.name()))
}

/// The focused output's name, via compositor focus IPC.
#[cfg(any(feature = "ocr", feature = "watch"))]
fn focused_output() -> Result<String> {
    let backend = wlr_capture::focus::detect()
        .context("focus info unavailable (unsupported compositor); specify -o NAME")?;
    backend
        .focused_output()
        .with_context(|| format!("no focused output detected (via {})", backend.name()))
}

/// Report the capture-relevant Wayland globals the current compositor advertises,
/// so users can tell at a glance whether (and how well) the suite will work here.
fn doctor() -> Result<()> {
    // (interface, what it enables). Order roughly by importance.
    const CHECKS: &[(&str, &str)] = &[
        ("ext_image_copy_capture_manager_v1", "capture frames (core)"),
        (
            "ext_output_image_capture_source_manager_v1",
            "capture an output (core)",
        ),
        (
            "ext_foreign_toplevel_image_capture_source_manager_v1",
            "capture a window",
        ),
        (
            "ext_foreign_toplevel_list_v1",
            "enumerate windows (chooser, -w)",
        ),
        ("zxdg_output_manager_v1", "accurate output geometry"),
        (
            "zwlr_layer_shell_v1",
            "overlays: region select, loupe, switcher",
        ),
        ("zwlr_data_control_manager_v1", "clipboard copy (-c)"),
        ("zwp_linux_dmabuf_v1", "zero-copy GPU capture"),
        (
            "zwp_keyboard_shortcuts_inhibit_manager_v1",
            "switcher keyboard grab",
        ),
    ];

    let globals = wl::advertised_globals().context("listing Wayland globals")?;
    let version = |iface: &str| globals.iter().find(|(n, _)| n == iface).map(|(_, v)| *v);

    println!("Compositor capabilities (advertised Wayland globals):\n");
    for (iface, desc) in CHECKS {
        match version(iface) {
            Some(v) => println!("  ✓ {iface} (v{v}) — {desc}"),
            None => println!("  ✗ {iface} — {desc}"),
        }
    }

    let core = version("ext_image_copy_capture_manager_v1").is_some()
        && version("ext_output_image_capture_source_manager_v1").is_some();
    let window = version("ext_foreign_toplevel_image_capture_source_manager_v1").is_some();
    println!();
    if core {
        println!(
            "Screen capture: supported (screenshots, recording, loupe, colour picker, wlr-draw)."
        );
    } else {
        println!(
            "Screen capture: UNSUPPORTED — needs ext-image-copy-capture-v1 + the output \
             source (wlroots ≥ 0.19 / Sway ≥ 1.11; not on Mutter/KWin via this path)."
        );
    }
    if window {
        println!(
            "Window capture: supported (wlr-switcher, -w/--pick-window, window mirror/record)."
        );
    } else {
        println!(
            "Window capture: UNSUPPORTED — needs the foreign-toplevel source \
             (wlroots ≥ 0.20 / Sway ≥ 1.12). Screen capture still works; only window-only \
             features are unavailable (wlr-switcher exits with a notice)."
        );
    }

    // Focus IPC (active-window / current-output) needs the `focus` engine feature,
    // which `wlr-peek` pulls in via `ocr`/`watch`.
    #[cfg(any(feature = "ocr", feature = "watch"))]
    match wlr_capture::focus::detect() {
        Some(b) => println!(
            "Focus IPC: {} detected (-a / --current-output work).",
            b.name()
        ),
        None => println!("Focus IPC: none detected (-a / --current-output unavailable)."),
    }

    Ok(())
}

/// The `clipboard-serve` daemon body: read the blob from stdin, then serve it.
fn clipboard_serve(mime: &str) -> Result<()> {
    use std::io::Read;
    let mut data = Vec::new();
    std::io::stdin()
        .read_to_end(&mut data)
        .context("reading clipboard data")?;
    wlr_capture::clipboard::serve(mime, data).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// `watch` — fire when a source changes or goes idle (the `watch` feature).
// ---------------------------------------------------------------------------

#[cfg(feature = "watch")]
mod watch_impl {
    use super::{active_window_rect, focused_output, pick_via_chooser};
    use anyhow::{Context, Result, bail};
    use clap::{Args, ValueEnum};
    use std::io::Write;
    use std::time::{Duration, Instant};
    use wlr_capture::wl::{self, CapturedImage, Output, Region};
    use wlr_capture::{capture, diff, gl::GpuReadback, overlay, stream};

    /// How long to wait for new content each poll round.
    const ROUND: Duration = Duration::from_millis(200);

    #[derive(Args)]
    pub struct WatchArgs {
        /// Watch an interactively selected region (drag on a frozen overlay).
        #[arg(short = 's', long, group = "source")]
        select: bool,
        /// Watch this named output (e.g. `DP-4`). Defaults to the only output.
        #[arg(short = 'o', long, value_name = "NAME", group = "source")]
        output: Option<String>,
        /// Watch this logical region, `"X,Y WxH"` (single output for now).
        #[arg(short = 'g', long, value_name = "GEOM", group = "source")]
        geometry: Option<String>,
        /// Watch the window with this `ext-foreign-toplevel` identifier.
        #[arg(short = 'w', long, value_name = "ID", group = "source")]
        window: Option<String>,
        /// Launch `wlr-chooser` to pick a window to watch.
        #[arg(long, group = "source")]
        pick_window: bool,
        /// Watch the active (focused) window's area — needs compositor focus info.
        #[arg(short = 'a', long, group = "source")]
        active_window: bool,
        /// Watch the focused output — needs compositor focus info.
        #[arg(long, group = "source")]
        current_output: bool,
        /// When to fire: on each change, or once the content goes idle.
        #[arg(long, value_enum, default_value_t = Trigger::Change)]
        on: Trigger,
        /// Ignore changes smaller than this percentage of the watched pixels
        /// (default 0 = any change). Only meaningful with `--on change`.
        #[arg(long, value_name = "PCT", default_value_t = 0.0)]
        threshold: f64,
        /// How long with no change counts as "idle" (e.g. `3s`). Only with `--on idle`.
        #[arg(long = "for", value_name = "DUR", default_value = "2s")]
        settle: String,
        /// Give up after this long with no trigger (exit code 2). Otherwise: Ctrl-C.
        #[arg(long, value_name = "DUR")]
        timeout: Option<String>,
        /// Keep watching and fire every time (default: exit after the first trigger).
        #[arg(long)]
        repeat: bool,
        /// Run this shell command on each trigger (in addition to printing).
        #[arg(long, value_name = "CMD")]
        exec: Option<String>,
    }

    /// What makes the monitor fire.
    #[derive(Clone, Copy, ValueEnum, PartialEq)]
    pub enum Trigger {
        /// Fire whenever the content changes (past `--threshold`).
        Change,
        /// Fire once the content has been stable for `--for`.
        Idle,
    }

    /// What to watch: an output (optionally cropped to a region) or a window.
    enum Target {
        Output {
            output: Output,
            crop: Option<Region>,
        },
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

    pub fn watch(args: WatchArgs) -> Result<()> {
        let mut client = wl::Client::connect().context("Wayland connection")?;
        client.refresh().ok();

        let target = resolve_target(&mut client, &args)?;
        let crop = match &target {
            Target::Output { crop, .. } => *crop,
            Target::Window(_) => None,
        };
        let source = match &target {
            Target::Output { output, .. } => stream::Source::Output(output.name.clone()),
            Target::Window(id) => stream::Source::Toplevel(id.clone()),
        };

        let threshold = args.threshold.clamp(0.0, 100.0);
        let settle = parse_interval(&args.settle)?;
        let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;

        eprintln!(
            "wlr-peek: watching {} (on {}){}",
            target.label(),
            if args.on == Trigger::Idle {
                "idle"
            } else {
                "change"
            },
            if args.repeat { ", Ctrl-C to stop" } else { "" }
        );

        let start = Instant::now();
        let mut s = stream::Stream::new(source, stream::DEFAULT_GRACE);
        let mut rb: Option<GpuReadback> = None;
        let mut prev: Option<CapturedImage> = None;
        let mut last_change: Option<Instant> = None;

        loop {
            if let Some(t) = timeout
                && start.elapsed() >= t
            {
                eprintln!("wlr-peek: no trigger within {}s", t.as_secs());
                std::process::exit(2);
            }

            let step = s.step(&mut client, ROUND);
            let mut changed_pct: Option<f64> = None;
            for frame in step.frames {
                let mut img = stream::decode_frame(&mut rb, frame)?;
                if let Some(c) = crop {
                    img = img.crop(c);
                }
                match &prev {
                    // First frame is the baseline — no trigger, just start the clock.
                    None => {
                        last_change.get_or_insert_with(Instant::now);
                    }
                    Some(p) => {
                        // A real change: some pixels actually differ (frac > 0, past the
                        // per-pixel tolerance) and the changed area meets the threshold.
                        // The `frac > 0` guard matters at the default threshold 0, where
                        // a compositor may still deliver identical damage frames.
                        let frac = diff::changed_fraction(p, &img, diff::DEFAULT_TOLERANCE);
                        if frac > 0.0 && frac * 100.0 >= threshold {
                            changed_pct = Some(frac * 100.0);
                            last_change = Some(Instant::now());
                        }
                    }
                }
                prev = Some(img);
            }

            match args.on {
                Trigger::Change => {
                    if let Some(pct) = changed_pct
                        && fire(&args, &format!("change {pct:.1}%"))?
                    {
                        return Ok(());
                    }
                }
                Trigger::Idle => {
                    if let Some(lc) = last_change
                        && lc.elapsed() >= settle
                    {
                        if fire(&args, "idle")? {
                            return Ok(());
                        }
                        last_change = Some(Instant::now()); // await the next idle period
                    }
                }
            }

            if let Some(end) = step.end {
                match end {
                    stream::End::NeverAppeared => bail!("source did not appear"),
                    stream::End::SourceGone => {
                        eprintln!("wlr-peek: source gone");
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Print the trigger and run `--exec`; returns `true` when the watch should stop.
    fn fire(args: &WatchArgs, msg: &str) -> Result<bool> {
        println!("{msg}");
        std::io::stdout().flush().ok();
        if let Some(cmd) = &args.exec {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .status()
                .with_context(|| format!("running --exec command: {cmd}"))?;
        }
        Ok(!args.repeat)
    }

    /// Resolve the CLI source flags (an exclusive group) to a [`Target`].
    fn resolve_target(client: &mut wl::Client, args: &WatchArgs) -> Result<Target> {
        if args.select {
            let caps = capture::capture_all(client, capture::DEFAULT_BUDGET)?;
            match overlay::select_region(&caps, &crate::tr!("overlay-region-hint"))? {
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
            Ok(Target::Window(
                pick_via_chooser().context("no window picked")?,
            ))
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

    /// The output the region's top-left corner sits on, plus the physical crop within
    /// that output's capture (single output for now).
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
        let crop = capture::logical_to_physical(&output, clipped);
        Ok(Target::Output {
            output,
            crop: Some(crop),
        })
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

    /// Parse a human interval like `2s`, `500ms`, `1m`, `1.5s` into a `Duration`.
    fn parse_interval(s: &str) -> Result<Duration> {
        let s = s.trim();
        let err = || anyhow::anyhow!("invalid duration '{s}' (try e.g. 2s, 500ms, 1m)");
        let (num, mult) = if let Some(n) = s.strip_suffix("ms") {
            (n, 0.001)
        } else if let Some(n) = s.strip_suffix('s') {
            (n, 1.0)
        } else if let Some(n) = s.strip_suffix('m') {
            (n, 60.0)
        } else {
            (s, 1.0)
        };
        let secs: f64 = num.trim().parse().map_err(|_| err())?;
        if !(secs.is_finite() && secs > 0.0) {
            return Err(err());
        }
        Ok(Duration::from_secs_f64(secs * mult))
    }
}

#[cfg(feature = "watch")]
use watch_impl::{WatchArgs, watch};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_color_variants() {
        assert_eq!(format_color(Format::Hex, 0x4d, 0x9a, 0xff), "#4D9AFF");
        assert_eq!(format_color(Format::Rgb, 77, 154, 255), "rgb(77, 154, 255)");
        assert_eq!(format_color(Format::Plain, 0, 128, 255), "0 128 255");
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn clean_ocr_tidies_tesseract_output() {
        // Trailing form-feed + blank lines, double blank lines, trailing spaces.
        let raw = "line one  \n\n\nline two   \n\u{0c}\n";
        assert_eq!(clean_ocr(raw), "line one\n\nline two");
        assert_eq!(clean_ocr("solo\n\u{0c}"), "solo");
        assert_eq!(clean_ocr("   \n\n"), "");
    }
}
