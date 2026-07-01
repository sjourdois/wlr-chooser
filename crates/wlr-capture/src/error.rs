//! The capture engine's typed error surface.
//!
//! The crate depends on `thiserror` only — no `anyhow` in its public API. Conditions a
//! caller may want to react to (present a message, pick a fallback, translate) get their
//! own variant; every lower-level failure (EGL, FFmpeg, PipeWire, Wayland, GL readback, IO)
//! flows through [`CaptureError::Backend`], which keeps a human context **and** the
//! underlying cause via `#[source]`, so the error chain is preserved. Variant text is plain
//! technical English, deliberately **not** localised — this is a library, so callers
//! translate by matching the variant.

use std::error::Error as StdError;
use thiserror::Error;

/// An error from the capture engine.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// The compositor can't capture individual windows: no foreign-toplevel image-capture
    /// source (wlroots < 0.20 / Sway < 1.12). Screen capture may still work.
    #[error(
        "this compositor cannot capture individual windows (needs wlroots >= 0.20 / Sway >= 1.12)"
    )]
    WindowsUnsupported,

    /// No outputs are available to capture.
    #[error("no outputs available")]
    NoOutputs,

    /// The requested region has zero area.
    #[error("empty region")]
    EmptyRegion,

    /// The requested region lies outside every output.
    #[error("region covers no output")]
    RegionOffscreen,

    /// No output matches the requested name.
    #[error("output '{0}' not found")]
    OutputNotFound(String),

    /// No window matches the requested id.
    #[error("window '{0}' not found")]
    WindowNotFound(String),

    /// A geometry string was not in `X,Y WxH` form.
    #[error("invalid geometry '{0}' (expected 'X,Y WxH')")]
    InvalidGeometry(String),

    /// The capture produced no frame before the deadline.
    #[error("capture timed out")]
    CaptureTimeout,

    /// No usable H.264 encoder is available (NVENC, VAAPI or libx264).
    #[cfg(feature = "video")]
    #[error("no H.264 encoder available (need NVENC, VAAPI or libx264)")]
    NoVideoEncoder,

    /// The source is too small to encode.
    #[cfg(feature = "video")]
    #[error("source too small to encode ({w}x{h})")]
    SourceTooSmall {
        /// Source width in pixels.
        w: u32,
        /// Source height in pixels.
        h: u32,
    },

    /// No audio capture backend is available.
    #[cfg(feature = "audio")]
    #[error("no audio backend available")]
    NoAudioBackend,

    /// A lower-level failure, with a human context and (when there is one) the underlying
    /// cause preserved so the error chain survives.
    #[error("{context}")]
    Backend {
        /// What was being attempted.
        context: String,
        /// The underlying cause, if any.
        #[source]
        source: Option<Box<dyn StdError + Send + Sync + 'static>>,
    },
}

/// The crate's result type: `Result<T, CaptureError>`. Mirrors `anyhow::Result` so a call
/// site keeps its `Result<T>` signatures and only swaps the import.
pub type Result<T, E = CaptureError> = std::result::Result<T, E>;

impl CaptureError {
    /// A message-only backend error (no underlying cause) — the typed replacement for a
    /// bare `anyhow!("…")` / `bail!("…")`.
    pub fn msg(context: impl Into<String>) -> Self {
        CaptureError::Backend {
            context: context.into(),
            source: None,
        }
    }
}

/// Attach context to a fallible value, mapping its error into [`CaptureError::Backend`]
/// while preserving the original cause. Mirrors `anyhow::Context`, so a call site only has
/// to import `crate::error::Context` instead of `anyhow::Context`.
pub trait Context<T> {
    /// Wrap the error with a fixed context message.
    fn context(self, context: impl Into<String>) -> Result<T, CaptureError>;
    /// Wrap the error with a lazily-built context message (only computed on error).
    fn with_context<S: Into<String>>(self, f: impl FnOnce() -> S) -> Result<T, CaptureError>;
}

impl<T, E: StdError + Send + Sync + 'static> Context<T> for Result<T, E> {
    fn context(self, context: impl Into<String>) -> Result<T, CaptureError> {
        self.map_err(|e| CaptureError::Backend {
            context: context.into(),
            source: Some(Box::new(e)),
        })
    }
    fn with_context<S: Into<String>>(self, f: impl FnOnce() -> S) -> Result<T, CaptureError> {
        self.map_err(|e| CaptureError::Backend {
            context: f().into(),
            source: Some(Box::new(e)),
        })
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, context: impl Into<String>) -> Result<T, CaptureError> {
        self.ok_or_else(|| CaptureError::msg(context))
    }
    fn with_context<S: Into<String>>(self, f: impl FnOnce() -> S) -> Result<T, CaptureError> {
        self.ok_or_else(|| CaptureError::msg(f()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(e: &CaptureError) -> Option<&(dyn StdError + 'static)> {
        StdError::source(e)
    }

    #[test]
    fn variant_display_text_is_stable() {
        assert_eq!(
            CaptureError::OutputNotFound("HDMI-1".into()).to_string(),
            "output 'HDMI-1' not found"
        );
        assert_eq!(
            CaptureError::WindowNotFound("42".into()).to_string(),
            "window '42' not found"
        );
        assert_eq!(
            CaptureError::InvalidGeometry("bad".into()).to_string(),
            "invalid geometry 'bad' (expected 'X,Y WxH')"
        );
        assert!(
            CaptureError::WindowsUnsupported
                .to_string()
                .contains("wlroots >= 0.20")
        );
    }

    #[cfg(feature = "video")]
    #[test]
    fn source_too_small_interpolates_dimensions() {
        assert_eq!(
            CaptureError::SourceTooSmall { w: 4, h: 2 }.to_string(),
            "source too small to encode (4x2)"
        );
    }

    #[test]
    fn msg_is_a_backend_error_without_a_cause() {
        let e = CaptureError::msg("boom");
        assert_eq!(e.to_string(), "boom");
        assert!(source(&e).is_none());
    }

    #[test]
    fn context_on_result_wraps_and_preserves_the_cause() {
        // A ParseIntError is a real StdError, so it flows into `Backend { source }`.
        let e: CaptureError = "x".parse::<i32>().context("parsing width").unwrap_err();
        assert_eq!(e.to_string(), "parsing width");
        let cause = source(&e).expect("cause preserved");
        assert!(cause.to_string().contains("invalid digit"));
    }

    #[test]
    fn with_context_is_lazy_and_only_builds_on_error() {
        let ok: Result<i32> = "1"
            .parse::<i32>()
            .with_context(|| -> String { unreachable!("not called on Ok") });
        assert_eq!(ok.unwrap(), 1);

        let e = "x"
            .parse::<i32>()
            .with_context(|| format!("ctx {}", 7))
            .unwrap_err();
        assert_eq!(e.to_string(), "ctx 7");
    }

    #[test]
    fn context_on_option_maps_none_and_passes_some_through() {
        let e = None::<i32>.context("missing thing").unwrap_err();
        assert_eq!(e.to_string(), "missing thing");
        assert!(source(&e).is_none());

        assert_eq!(Some(5).context("unused").unwrap(), 5);
    }
}
