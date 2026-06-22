//! wlr-pip — floating, always-on-top live mirror (picture-in-picture) of a
//! wlroots window.
//!
//! Usage: `wlr-pip <identifier>` mirrors the window with that
//! `ext-foreign-toplevel` identifier (as printed by `wlr-chooser`). With no
//! argument it launches the chooser to pick one.

mod host;
mod pip;

use clap::Parser;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Floating always-on-top live mirror of a wlroots window.
#[derive(Parser)]
#[command(name = "wlr-pip", version, about)]
struct Cli {
    /// The `ext-foreign-toplevel` identifier of the window to mirror
    /// (as printed by `wlr-chooser`). If omitted, the chooser is launched to pick.
    identifier: Option<String>,

    /// Headless capture benchmark: stream the source for SECS seconds and print
    /// frame stats to stderr (debug; no window).
    #[arg(long, value_name = "SECS", hide = true)]
    bench: Option<u64>,

    /// List candidate windows (identifier — app_id — title) and exit (debug).
    #[arg(long, hide = true)]
    list: bool,
}

fn main() {
    let cli = Cli::parse();
    wlr_capture::i18n::init();

    if cli.list {
        list_windows();
        return;
    }

    // Either a given identifier, or one picked interactively via the chooser.
    let identifier = match cli.identifier {
        Some(id) => id,
        None => match pick_via_chooser() {
            Some(id) => id,
            None => std::process::exit(1), // cancelled or chooser unavailable
        },
    };

    if let Some(secs) = cli.bench {
        bench(identifier, secs);
        return;
    }

    // One mirror per window: a second launch for the same identifier is a no-op
    // (different windows each get their own mirror). Held for our lifetime.
    let _lock = match acquire_pip_lock(&identifier) {
        Some(lock) => lock,
        None => return, // already mirroring this window
    };

    // Resolve a human label + app icon for the window (and validate it exists).
    let (label, icon) = resolve_window(&identifier);
    if let Err(e) = host::run(identifier, label, icon) {
        eprintln!("wlr-pip: {e:#}");
        std::process::exit(2);
    }
}

/// Launch `wlr-chooser --windows` to pick a window, and parse its
/// `Window: <identifier>` stdout contract. Prefers a `wlr-chooser` next to our own
/// binary (so the workspace build / install stays self-consistent), else `$PATH`.
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

/// Acquire the single-instance advisory lock for this window's mirror. Returns the
/// held lock file (keep it alive), or `None` if another mirror already owns it.
fn acquire_pip_lock(identifier: &str) -> Option<std::fs::File> {
    use rustix::fs::{FlockOperation, flock};
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    // The identifier is opaque (hex on wlroots); sanitise it for a filename anyway.
    let safe: String = identifier
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join(format!("wlr-pip-{safe}.lock")))
        .ok()?;
    flock(&f, FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(f)
}

/// Look up the target window's app-id/title (for the label) and app icon. Falls
/// back to a generic label if it can't be found right now (it may map shortly).
fn resolve_window(identifier: &str) -> (String, Option<(u32, u32, Vec<u8>)>) {
    let mut client = match wlr_capture::wl::Client::connect() {
        Ok(c) => c,
        Err(_) => return ("wlr-pip".to_string(), None),
    };
    let _ = client.refresh();
    let Some(t) = client
        .toplevels()
        .iter()
        .find(|t| t.identifier == identifier)
    else {
        return ("wlr-pip".to_string(), None);
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

/// List capturable windows with their identifiers (debug helper).
fn list_windows() {
    let mut client = match wlr_capture::wl::Client::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wlr-pip: {e:#}");
            std::process::exit(2);
        }
    };
    let _ = client.refresh();
    for t in client.toplevels() {
        println!("{}\t{}\t{}", t.identifier, t.app_id, t.title);
    }
}

/// Headless smoke test: count frames received for `secs` seconds.
fn bench(identifier: String, secs: u64) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || pip::capture_thread(identifier, move |m| tx.send(m).is_ok()));
    let deadline = Instant::now() + Duration::from_secs(secs);
    let (mut shm, mut dmabuf) = (0u32, 0u32);
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(pip::PipMsg::Shm { w, h, .. }) => {
                shm += 1;
                if shm == 1 {
                    eprintln!("bench: first shm frame {w}x{h}");
                }
            }
            Ok(pip::PipMsg::Dmabuf { frame }) => {
                dmabuf += 1;
                if dmabuf == 1 {
                    eprintln!(
                        "bench: first dma-buf frame {}x{}",
                        frame.width, frame.height
                    );
                }
            }
            Ok(pip::PipMsg::Gone) => {
                eprintln!("bench: source gone");
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    eprintln!("bench: {shm} shm frame(s), {dmabuf} dma-buf frame(s) in {secs}s");
}
