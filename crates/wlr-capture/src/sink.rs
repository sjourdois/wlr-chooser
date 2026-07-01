//! Common output seam for capture-consuming tools (screenshot, record, timelapse).
//!
//! A capture round yields a [`Frame`] — CPU pixels (shm) or a GPU dma-buf
//! (zero-copy). A [`FrameSink`] consumes a stream of those frames. Most sinks only
//! want CPU pixels, so the default [`FrameSink::push_dmabuf`] reads the dma-buf back
//! via a [`GpuReadback`] and forwards to [`FrameSink::push`]; a GPU-native sink (a
//! future hardware video encoder, say) can override it to keep the buffer on the
//! GPU. [`pump`] routes a `Frame` to whichever path applies, creating the readback
//! lazily so pure-shm streams never spin up an EGL context.

use crate::error::Result;
use crate::gl::GpuReadback;
use crate::wl::{CapturedImage, DmabufFrame, Frame};
use std::time::Duration;

/// A consumer of a capture stream. `ts` is each frame's capture time relative to a
/// start the sink defines (a screenshot ignores it; a recorder uses it for timing).
pub trait FrameSink {
    /// Consume one CPU-pixel frame.
    fn push(&mut self, img: &CapturedImage, ts: Duration) -> Result<()>;

    /// Feed interleaved PCM for an optional audio track. The default ignores it (sinks
    /// without sound — GIF/WebP — and silent recordings); the video encoder buffers it
    /// and muxes it on the next [`FrameSink::push`].
    fn push_audio(&mut self, _pcm: &[f32]) {}

    /// Consume one GPU dma-buf frame. The default reads it back to CPU pixels via
    /// `rb` and forwards to [`FrameSink::push`]; override to consume it on the GPU.
    fn push_dmabuf(
        &mut self,
        rb: &mut GpuReadback,
        frame: DmabufFrame,
        ts: Duration,
    ) -> Result<()> {
        let img = rb.readback(frame)?;
        self.push(&img, ts)
    }

    /// Flush and finalize (write the file, close the encoder, …). Call once, last.
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Route one [`Frame`] to `sink`, picking the CPU or dma-buf path. The readback
/// context lives in `rb` and is built on first need — a stream that only ever
/// produces shm frames (a no-GPU build) never constructs one. Hold a single
/// `Option<GpuReadback>` across the whole stream so the context is reused.
pub fn pump(
    sink: &mut dyn FrameSink,
    rb: &mut Option<GpuReadback>,
    frame: Frame,
    ts: Duration,
) -> Result<()> {
    match frame {
        Frame::Shm(img) => sink.push(&img, ts),
        Frame::Dmabuf(d) => {
            let rb = match rb {
                Some(rb) => rb,
                None => rb.insert(GpuReadback::new()?),
            };
            sink.push_dmabuf(rb, d, ts)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that records what it received — exercises the shm path of [`pump`]
    /// and the trait's stamping without needing a GPU.
    #[derive(Default)]
    struct Collect {
        frames: Vec<(u32, u32, Duration)>,
        finished: bool,
    }

    impl FrameSink for Collect {
        fn push(&mut self, img: &CapturedImage, ts: Duration) -> Result<()> {
            self.frames.push((img.width, img.height, ts));
            Ok(())
        }
        fn finish(&mut self) -> Result<()> {
            self.finished = true;
            Ok(())
        }
    }

    #[test]
    fn pump_routes_shm_frames() {
        let mut sink = Collect::default();
        let mut rb = None; // never built: no dma-buf frames in this stream
        let img = CapturedImage {
            width: 4,
            height: 2,
            rgba: vec![0; 4 * 2 * 4],
        };
        pump(
            &mut sink,
            &mut rb,
            Frame::Shm(img),
            Duration::from_millis(40),
        )
        .unwrap();
        sink.finish().unwrap();

        assert!(rb.is_none());
        assert_eq!(sink.frames, vec![(4, 2, Duration::from_millis(40))]);
        assert!(sink.finished);
    }
}
