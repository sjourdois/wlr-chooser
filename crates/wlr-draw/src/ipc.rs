//! The control socket: how a second `wlr-draw` invocation talks to the running daemon.
//!
//! A wlroots layer-shell client cannot grab a global hotkey, so — like gromit-mpx —
//! the daemon holds the overlay and further invocations drive it. They connect to a
//! per-user Unix socket in `$XDG_RUNTIME_DIR`, send one [`Cmd`] line, and read a short
//! `ok` / `err …` reply. The daemon's accept loop runs on its own thread and forwards
//! parsed commands into the calloop event loop over a [`channel`], whose write end
//! wakes the loop — so socket commands and Wayland events share one loop.

use crate::proto::Cmd;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

/// Path of the per-user control socket.
pub fn socket_path() -> PathBuf {
    wlr_capture::paths::runtime_dir().join("wlr-draw.sock")
}

/// Whether a daemon is already accepting on the socket.
pub fn daemon_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Client side: connect to the daemon, send one command, and surface its reply. Errors
/// if no daemon is listening or the command was rejected.
pub fn send(cmd: &Cmd) -> anyhow::Result<()> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        anyhow::anyhow!(
            "no wlr-draw daemon listening on {} ({e}); start one with `wlr-draw`",
            path.display()
        )
    })?;
    stream.write_all(cmd.to_line().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    if let Some(reason) = reply.strip_prefix("err ") {
        anyhow::bail!("{}", reason.trim());
    }
    Ok(())
}

/// Daemon side: bind the control socket, refusing to start if another daemon already
/// owns it and clearing a stale socket file otherwise.
pub fn bind() -> anyhow::Result<UnixListener> {
    if daemon_running() {
        anyhow::bail!("a wlr-draw daemon is already running");
    }
    let path = socket_path();
    // A leftover file from a crashed daemon: nothing is listening (checked above), so
    // remove it before re-binding.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)
        .map_err(|e| anyhow::anyhow!("cannot bind {}: {e}", path.display()))?;
    Ok(listener)
}

/// Spawn the accept loop. Each connection delivers one command line; valid commands are
/// acknowledged with `ok` and forwarded on `tx`, invalid ones answered with `err …`.
/// The thread ends when the listener is dropped (daemon exit).
pub fn serve(listener: UnixListener, tx: Sender<Cmd>) {
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(read_half) = stream.try_clone() else {
                continue;
            };
            let mut line = String::new();
            if BufReader::new(read_half).read_line(&mut line).is_err() {
                continue;
            }
            match Cmd::parse(line.trim()) {
                Ok(cmd) => {
                    let _ = stream.write_all(b"ok\n");
                    if tx.send(cmd).is_err() {
                        return; // event loop gone
                    }
                }
                Err(e) => {
                    let _ = stream.write_all(format!("err {e}\n").as_bytes());
                }
            }
        }
    });
}

/// Remove the socket file on daemon shutdown (best effort).
pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
