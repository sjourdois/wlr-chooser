//! Native PipeWire audio capture for recording.
//!
//! Captures the default sink's monitor (system audio) — or a named target node — as
//! 48 kHz stereo F32 interleaved PCM into a shared buffer the video muxer drains. The
//! PipeWire loop runs on its own thread; dropping the [`AudioCapture`] stops it.
//!
//! Capture and encoding are decoupled (the muxer in [`crate::video`] turns this PCM into
//! an AAC stream), so a different backend — a Pulse/ALSA fallback — could feed the same
//! pipeline without touching the encoder.

use anyhow::{Result, anyhow};
use pipewire as pw;
use pw::{properties::properties, spa};
use spa::pod::Pod;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Sample rate we ask PipeWire to deliver (it resamples the graph for us).
pub const RATE: u32 = 48_000;
/// Channel count we ask for (PipeWire down/up-mixes the source).
pub const CHANNELS: u32 = 2;

/// A running PipeWire capture. Interleaved f32 samples accumulate in `pcm`; the encoder
/// drains them with [`AudioCapture::drain`]. Dropping it quits the loop and joins.
pub struct AudioCapture {
    pcm: Arc<Mutex<VecDeque<f32>>>,
    stop: Option<pw::channel::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioCapture {
    /// Start capturing. With `target` `None`, captures the default sink's monitor
    /// (system audio); otherwise the named/numbered node `target`.
    pub fn start(target: Option<String>) -> Result<Self> {
        let pcm: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let pcm_thread = pcm.clone();
        // The loop thread hands back its stop-sender (or a setup error) before run()s.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<pw::channel::Sender<()>, String>>();

        let thread = std::thread::Builder::new()
            .name("wlr-audio-pw".into())
            .spawn(move || {
                if let Err(e) = run_loop(pcm_thread, target, &ready_tx) {
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            })?;

        match ready_rx.recv() {
            Ok(Ok(stop)) => Ok(Self {
                pcm,
                stop: Some(stop),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(anyhow!("PipeWire audio: {e}"))
            }
            Err(_) => Err(anyhow!("PipeWire audio thread exited during setup")),
        }
    }

    /// Take all PCM captured so far (interleaved, [`CHANNELS`] per sample frame).
    pub fn drain(&self) -> Vec<f32> {
        let mut q = self.pcm.lock().unwrap();
        q.drain(..).collect()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The PipeWire loop body, run on the capture thread.
fn run_loop(
    pcm: Arc<Mutex<VecDeque<f32>>>,
    target: Option<String>,
    ready: &mpsc::Sender<Result<pw::channel::Sender<()>, String>>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None).map_err(|e| anyhow!("main loop: {e}"))?;
    let context = pw::context::Context::new(&mainloop).map_err(|e| anyhow!("context: {e}"))?;
    let core = context.connect(None).map_err(|e| anyhow!("connect: {e}"))?;

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
    };
    match &target {
        Some(t) => {
            props.insert(*pw::keys::TARGET_OBJECT, t.clone());
        }
        // No explicit target: capture from a sink's monitor (i.e. system audio).
        None => {
            props.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");
        }
    }

    let stream = pw::stream::Stream::new(&core, "wlr-shot-audio", props)
        .map_err(|e| anyhow!("stream: {e}"))?;

    let pcm_cb = pcm.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .process(move |stream, ()| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else {
                return;
            };
            let n_bytes = d.chunk().size() as usize;
            if let Some(slice) = d.data() {
                let slice = &slice[..n_bytes.min(slice.len())];
                let mut q = pcm_cb.lock().unwrap();
                for s in slice.chunks_exact(4) {
                    q.push_back(f32::from_le_bytes([s[0], s[1], s[2], s[3]]));
                }
            }
        })
        .register()
        .map_err(|e| anyhow!("listener: {e}"))?;

    // Ask for 48 kHz stereo F32LE; PipeWire's adapter resamples/remixes to match.
    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(RATE);
    audio_info.set_channels(CHANNELS);
    let obj = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .map_err(|e| anyhow!("POD serialize: {e}"))?
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).ok_or_else(|| anyhow!("invalid format POD"))?];

    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|e| anyhow!("connect stream: {e}"))?;

    // A message on this channel quits the loop (sent by `AudioCapture::drop`).
    let (stop_tx, stop_rx) = pw::channel::channel::<()>();
    let ml = mainloop.clone();
    let _recv = stop_rx.attach(mainloop.loop_(), move |_| ml.quit());

    ready
        .send(Ok(stop_tx))
        .map_err(|_| anyhow!("handing back the stop channel"))?;
    mainloop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live smoke test (needs a running PipeWire): capture system audio briefly and
    /// confirm samples flow. `cargo test -p wlr-capture --features audio -- --ignored`.
    #[test]
    #[ignore]
    fn captures_system_audio() {
        let cap = AudioCapture::start(None).expect("start capture");
        std::thread::sleep(std::time::Duration::from_millis(500));
        let pcm = cap.drain();
        let peak = pcm.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        eprintln!("captured {} samples, peak {peak:.3}", pcm.len());
        assert!(!pcm.is_empty(), "no PCM captured in 500ms");
        assert_eq!(
            pcm.len() % CHANNELS as usize,
            0,
            "ragged channel interleave"
        );
    }
}
