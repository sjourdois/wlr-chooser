//! PiP capture: drive the shared `wlr-capture` engine for a *single* toplevel,
//! at full resolution, and stream its frames to the windowing host.
//!
//! Unlike the chooser (which captures every source and downscales to thumbnails),
//! the mirror wants one source at native size. The GPU dma-buf path hands frames
//! off zero-copy; the shm fallback carries full-resolution pixels.

use std::time::{Duration, Instant};
use wlr_capture::wl;

/// Frame budget per capture round (~30 fps ceiling; capture is damage-driven, so
/// a static window costs ~one syscall and streams nothing).
const ROUND: Duration = Duration::from_millis(33);

/// How long to wait for the target window to appear before giving up (it may not
/// be mapped yet when we launch).
const APPEAR_GRACE: Duration = Duration::from_secs(5);

/// A captured frame (or the source's demise) for the single mirrored window.
pub enum PipMsg {
    /// CPU shm frame at full resolution (RGBA8).
    Shm { w: usize, h: usize, rgba: Vec<u8> },
    /// GPU dma-buf frame to import zero-copy as a GL texture (host-side).
    Dmabuf { frame: wl::DmabufFrame },
    /// The source window is gone (closed, or never appeared): the mirror ends.
    Gone,
}

/// Capture thread body: open one persistent session for the window whose
/// `ext-foreign-toplevel` identifier matches `identifier`, then stream its frames
/// until it closes or the host drops the channel. Reopens the session if the
/// compositor transiently stops it (e.g. on resize).
///
/// `sink` consumes each message and returns `false` once the receiver is gone (so
/// the thread can stop). It is generic so the host (calloop channel) and the
/// headless bench (std mpsc) can both drive it.
pub fn capture_thread(identifier: String, mut sink: impl FnMut(PipMsg) -> bool) {
    let mut client = match wl::Client::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wlr-pip: {e:#}");
            sink(PipMsg::Gone);
            return;
        }
    };

    let mut session: Option<wl::SessionId> = None;
    let appear_deadline = Instant::now() + APPEAR_GRACE;

    loop {
        if client.refresh().is_err() {
            sink(PipMsg::Gone);
            return;
        }

        let present = client
            .toplevels()
            .iter()
            .any(|t| t.identifier == identifier);
        if session.is_none() {
            match client
                .toplevels()
                .iter()
                .find(|t| t.identifier == identifier)
                .cloned()
            {
                Some(t) => {
                    if let Ok(id) = client.open_toplevel_session(&t) {
                        session = Some(id);
                    }
                }
                // Not mapped yet: keep polling until the grace period elapses.
                None if Instant::now() >= appear_deadline => {
                    sink(PipMsg::Gone);
                    return;
                }
                None => {}
            }
        } else if !present {
            // We had a live session and the source vanished: the window closed.
            sink(PipMsg::Gone);
            return;
        }

        let (frames, failed) = client.poll(ROUND);
        // Single source, so every delivered frame is ours.
        for (_id, frame) in frames {
            let msg = match frame {
                wl::Frame::Shm(img) => PipMsg::Shm {
                    w: img.width as usize,
                    h: img.height as usize,
                    rgba: img.rgba,
                },
                wl::Frame::Dmabuf(frame) => PipMsg::Dmabuf { frame },
            };
            if !sink(msg) {
                return; // host gone
            }
        }
        // A stopped session (e.g. resize): drop it; we reopen next round if the
        // window is still listed.
        for id in failed {
            if session.as_ref() == Some(&id) {
                session = None;
            }
            client.close_session(&id);
        }
    }
}
