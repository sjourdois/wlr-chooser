//! `wlr-capture` — the reusable bricks behind the wlr-utils tools.
//!
//! - [`wl`]: native Wayland client that enumerates foreign toplevels + outputs and
//!   captures them (full-resolution, zero-copy GPU dma-buf path) via
//!   `ext-image-copy-capture`.
//! - [`icons`] / [`theme`] / [`i18n`]: shared overlay UI helpers.
//!
//! Consumers (`wlr-chooser`, `wlr-pip`, …) build their own windowing host on top
//! and reuse this engine for the heavy lifting.

pub mod i18n;
pub mod icons;
pub mod render;
pub mod theme;
pub mod wl;
