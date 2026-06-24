//! Shared driver for a live capture session over one source.
//!
//! The capture engine delivers frames one at a time per session, re-armed each
//! round, and a session can stop transiently (e.g. on resize) or for good (the
//! window closed). Every streaming consumer — the live mirror, the recorder, the
//! change monitor — needs the same arm / poll / reopen / give-up loop. [`Stream`]
//! is that loop, factored out: hold one, call [`Stream::step`] each round, and act
//! on the [`Step`] it returns.
//!
//! It deliberately yields raw [`Frame`]s (not decoded pixels): the mirror imports
//! dma-buf frames straight into a GL texture (zero-copy), while the recorder and
//! monitor read them back to CPU pixels. The driver stays in the dependency-free
//! core so any tool can use it.

use crate::gl::GpuReadback;
use crate::wl::{CapturedImage, Client, Frame, SessionId};
use anyhow::Result;
use std::time::{Duration, Instant};

/// Default time to wait for the source to appear before giving up.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(5);

/// What to stream. Both variants are resolved by name/identifier each round, so a
/// source that reappears (or an output that comes back) is picked up again.
#[derive(Clone, Debug)]
pub enum Source {
    /// The output with this name (e.g. `DP-4`).
    Output(String),
    /// The toplevel with this `ext-foreign-toplevel` identifier.
    Toplevel(String),
}

/// Why a stream ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The source was live and then vanished (window closed, output unplugged), or
    /// the connection dropped.
    SourceGone,
    /// The source never appeared within the grace period.
    NeverAppeared,
}

/// The outcome of one [`Stream::step`]: the frames that arrived this round, and
/// whether the stream has ended.
pub struct Step {
    /// Frames delivered this round (every one is the single source's).
    pub frames: Vec<Frame>,
    /// `Some` once the stream is over — stop calling `step`.
    pub end: Option<End>,
}

/// A live capture session for one [`Source`], reopened as needed.
pub struct Stream {
    source: Source,
    session: Option<SessionId>,
    appear_deadline: Instant,
    /// Whether a session was ever successfully opened (distinguishes "gone" from
    /// "never appeared" when the source is absent).
    had_session: bool,
}

impl Stream {
    /// Start a stream for `source`, giving it `grace` to first appear.
    pub fn new(source: Source, grace: Duration) -> Self {
        Self {
            source,
            session: None,
            appear_deadline: Instant::now() + grace,
            had_session: false,
        }
    }

    /// Run one round: refresh state, (re)open the session, poll up to `budget`, and
    /// return the frames that arrived. Returns a [`Step`] whose `end` is set once the
    /// source is gone or never showed up.
    pub fn step(&mut self, client: &mut Client, budget: Duration) -> Step {
        if client.refresh().is_err() {
            return self.ended(End::SourceGone);
        }

        if self.session.is_none() {
            match self.open(client) {
                Some(id) => {
                    self.session = Some(id);
                    self.had_session = true;
                }
                None if Instant::now() >= self.appear_deadline => {
                    let why = if self.had_session {
                        End::SourceGone
                    } else {
                        End::NeverAppeared
                    };
                    return self.ended(why);
                }
                // Not present yet: keep dispatching below so it can show up.
                None => {}
            }
        } else if self.is_gone(client) {
            return self.ended(End::SourceGone);
        }

        let (got, failed) = client.poll(budget);
        // A stopped session (e.g. on resize): drop it and reopen next round.
        for id in failed {
            if self.session.as_ref() == Some(&id) {
                self.session = None;
            }
            client.close_session(&id);
        }
        Step {
            frames: got.into_iter().map(|(_id, f)| f).collect(),
            end: None,
        }
    }

    /// Open a session for the source if it is currently present.
    fn open(&self, client: &mut Client) -> Option<SessionId> {
        match &self.source {
            Source::Output(name) => {
                let out = client.outputs().iter().find(|o| o.name == *name).cloned()?;
                client.open_output_session(&out).ok()
            }
            Source::Toplevel(id) => {
                let tl = client
                    .toplevels()
                    .iter()
                    .find(|t| t.identifier == *id)
                    .cloned()?;
                client.open_toplevel_session(&tl).ok()
            }
        }
    }

    /// Whether a source we had a session for has since disappeared.
    fn is_gone(&self, client: &Client) -> bool {
        match &self.source {
            Source::Output(name) => !client.outputs().iter().any(|o| o.name == *name),
            Source::Toplevel(id) => !client.toplevels().iter().any(|t| t.identifier == *id),
        }
    }

    fn ended(&self, why: End) -> Step {
        Step {
            frames: Vec::new(),
            end: Some(why),
        }
    }
}

/// Decode a streamed [`Frame`] to CPU pixels: shm frames pass through; a dma-buf
/// frame is read back via `rb`, which is built on first need (a pure-shm stream never
/// spins up a GL context). Hold one `Option<GpuReadback>` across the whole stream so
/// the readback context is reused.
pub fn decode_frame(rb: &mut Option<GpuReadback>, frame: Frame) -> Result<CapturedImage> {
    match frame {
        Frame::Shm(img) => Ok(img),
        Frame::Dmabuf(d) => {
            let rb = match rb {
                Some(rb) => rb,
                None => rb.insert(GpuReadback::new()?),
            };
            rb.readback(d)
        }
    }
}
