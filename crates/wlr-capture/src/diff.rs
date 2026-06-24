//! Frame-difference metric for change detection.
//!
//! A dependency-free helper for the change monitor: how much did two captured
//! frames differ? Pixels are compared on RGB only (captures force alpha to 255), and
//! a per-pixel `tolerance` absorbs codec/dither noise so a blinking cursor or a
//! one-bit jitter doesn't read as a change.

use crate::wl::CapturedImage;

/// A sensible default per-channel tolerance (out of 255): below this, two pixels
/// are considered equal.
pub const DEFAULT_TOLERANCE: u8 = 8;

/// Fraction (0.0–1.0) of pixels that differ between `a` and `b` by more than
/// `tolerance` on any RGB channel. Differing dimensions count as fully changed
/// (1.0); two empty frames are unchanged (0.0).
pub fn changed_fraction(a: &CapturedImage, b: &CapturedImage, tolerance: u8) -> f64 {
    if a.width != b.width || a.height != b.height {
        return 1.0;
    }
    let total = (a.width as usize) * (a.height as usize);
    if total == 0 {
        return 0.0;
    }
    let changed = a
        .rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(pa, pb)| {
            pa[0].abs_diff(pb[0]) > tolerance
                || pa[1].abs_diff(pb[1]) > tolerance
                || pa[2].abs_diff(pb[2]) > tolerance
        })
        .count();
    changed as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, fill: [u8; 4]) -> CapturedImage {
        CapturedImage {
            width: w,
            height: h,
            rgba: fill.repeat((w * h) as usize),
        }
    }

    #[test]
    fn identical_is_zero() {
        let a = img(4, 4, [10, 20, 30, 255]);
        let b = img(4, 4, [10, 20, 30, 255]);
        assert_eq!(changed_fraction(&a, &b, DEFAULT_TOLERANCE), 0.0);
    }

    #[test]
    fn within_tolerance_is_zero() {
        let a = img(2, 2, [100, 100, 100, 255]);
        let b = img(2, 2, [105, 100, 100, 255]); // +5 < tolerance 8
        assert_eq!(changed_fraction(&a, &b, DEFAULT_TOLERANCE), 0.0);
    }

    #[test]
    fn counts_changed_pixels() {
        let mut a = img(2, 1, [0, 0, 0, 255]);
        let b = img(2, 1, [0, 0, 0, 255]);
        a.rgba[0] = 200; // one of two pixels differs well past tolerance
        assert_eq!(changed_fraction(&a, &b, DEFAULT_TOLERANCE), 0.5);
    }

    #[test]
    fn alpha_is_ignored() {
        let a = img(1, 1, [0, 0, 0, 255]);
        let b = img(1, 1, [0, 0, 0, 0]); // only alpha differs
        assert_eq!(changed_fraction(&a, &b, DEFAULT_TOLERANCE), 0.0);
    }

    #[test]
    fn size_mismatch_is_full() {
        let a = img(2, 2, [0, 0, 0, 255]);
        let b = img(3, 2, [0, 0, 0, 255]);
        assert_eq!(changed_fraction(&a, &b, DEFAULT_TOLERANCE), 1.0);
    }
}
