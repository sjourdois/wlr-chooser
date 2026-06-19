//! Native Wayland client: enumerate foreign toplevels and outputs, and capture
//! them via `ext-image-copy-capture-v1`.
//!
//! The whole point of doing this natively (instead of shelling out to `grim -T`)
//! is to create the shm buffer with the *correct* stride (`width * 4`), which is
//! where grim 1.5 trips up ("Invalid stride") on some toplevels (Firefox, …).

use anyhow::{Context, Result, bail};
use std::os::fd::AsFd;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
    event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::WlOutput,
        wl_registry::WlRegistry,
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
    },
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

/// A capturable window.
#[derive(Clone)]
pub struct Toplevel {
    pub handle: ExtForeignToplevelHandleV1,
    pub identifier: String,
    pub title: String,
    pub app_id: String,
}

/// A capturable output.
#[derive(Clone)]
pub struct Output {
    pub wl_output: WlOutput,
    pub name: String,
}

/// Decoded RGBA8 image.
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Byte layout of a wl_shm pixel format (memory order, little-endian), so we can
/// convert to RGBA8 and — crucially — compute the correct stride (`width * bpp`).
struct PixelLayout {
    bpp: usize,
    r: usize,
    g: usize,
    b: usize,
    a: Option<usize>,
}

impl PixelLayout {
    fn of(f: wl_shm::Format) -> Option<Self> {
        use wl_shm::Format::*;
        Some(match f {
            Argb8888 => Self {
                bpp: 4,
                r: 2,
                g: 1,
                b: 0,
                a: Some(3),
            },
            Xrgb8888 => Self {
                bpp: 4,
                r: 2,
                g: 1,
                b: 0,
                a: None,
            },
            Abgr8888 => Self {
                bpp: 4,
                r: 0,
                g: 1,
                b: 2,
                a: Some(3),
            },
            Xbgr8888 => Self {
                bpp: 4,
                r: 0,
                g: 1,
                b: 2,
                a: None,
            },
            Bgr888 => Self {
                bpp: 3,
                r: 0,
                g: 1,
                b: 2,
                a: None,
            },
            Rgb888 => Self {
                bpp: 3,
                r: 2,
                g: 1,
                b: 0,
                a: None,
            },
            _ => return None,
        })
    }
}

#[derive(Default)]
struct PendingToplevel {
    identifier: String,
    title: String,
    app_id: String,
}

/// In-flight capture bookkeeping (one capture at a time).
#[derive(Default)]
struct Cap {
    width: u32,
    height: u32,
    format: Option<wl_shm::Format>,
    constraints_done: bool,
    ready: bool,
    failed: Option<FailureReason>,
}

#[derive(Default)]
struct State {
    toplevels: Vec<Toplevel>,
    pending: Vec<(ExtForeignToplevelHandleV1, PendingToplevel)>,
    outputs: Vec<Output>,
    shm: Option<WlShm>,
    tl_src: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    out_src: Option<ExtOutputImageCaptureSourceManagerV1>,
    copy: Option<ExtImageCopyCaptureManagerV1>,
    cap: Cap,
}

pub struct Client {
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
}

impl Client {
    /// Connect, bind the capture managers, and enumerate windows + outputs.
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("connexion Wayland")?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).context("registre Wayland")?;
        let qh = queue.handle();

        let shm = globals.bind(&qh, 1..=1, ()).context("wl_shm")?;
        let copy = globals
            .bind(&qh, 1..=1, ())
            .context("ext_image_copy_capture_manager_v1 absent")?;
        let tl_src = globals
            .bind(&qh, 1..=1, ())
            .context("ext_foreign_toplevel_image_capture_source_manager_v1 absent")?;
        let out_src = globals
            .bind(&qh, 1..=1, ())
            .context("ext_output_image_capture_source_manager_v1 absent")?;
        let _list: ExtForeignToplevelListV1 = globals
            .bind(&qh, 1..=1, ())
            .context("ext_foreign_toplevel_list_v1 absent")?;

        globals.contents().with_list(|list| {
            for g in list {
                if g.interface == WlOutput::interface().name {
                    let _: WlOutput = globals.registry().bind(g.name, g.version.min(4), &qh, ());
                }
            }
        });

        let mut state = State {
            shm: Some(shm),
            copy: Some(copy),
            tl_src: Some(tl_src),
            out_src: Some(out_src),
            ..Default::default()
        };
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;

        Ok(Self { queue, qh, state })
    }

    pub fn toplevels(&self) -> &[Toplevel] {
        &self.state.toplevels
    }
    pub fn outputs(&self) -> &[Output] {
        &self.state.outputs
    }

    pub fn capture_toplevel(&mut self, t: &Toplevel) -> Result<CapturedImage> {
        let src = self
            .state
            .tl_src
            .as_ref()
            .unwrap()
            .create_source(&t.handle, &self.qh, ());
        self.capture(src)
    }

    pub fn capture_output(&mut self, o: &Output) -> Result<CapturedImage> {
        let src = self
            .state
            .out_src
            .as_ref()
            .unwrap()
            .create_source(&o.wl_output, &self.qh, ());
        self.capture(src)
    }

    fn capture(&mut self, src: ExtImageCaptureSourceV1) -> Result<CapturedImage> {
        self.state.cap = Cap::default();
        let session =
            self.state
                .copy
                .as_ref()
                .unwrap()
                .create_session(&src, Options::empty(), &self.qh, ());

        // Wait for buffer constraints (buffer_size + shm_format + done).
        while !self.state.cap.constraints_done && self.state.cap.failed.is_none() {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        let (w, h) = (self.state.cap.width, self.state.cap.height);
        let format = self
            .state
            .cap
            .format
            .context("le compositeur n'a pas proposé de format shm")?;
        if w == 0 || h == 0 {
            bail!("dimensions de capture nulles");
        }
        let layout =
            PixelLayout::of(format).with_context(|| format!("format shm non géré: {format:?}"))?;
        let stride = w * layout.bpp as u32; // stride correct selon le bpp réel du format
        let size = (stride * h) as usize;

        // shm buffer with the CORRECT stride.
        let fd = rustix::fs::memfd_create("wlr-chooser-shm", rustix::fs::MemfdFlags::CLOEXEC)
            .context("memfd_create")?;
        rustix::fs::ftruncate(&fd, size as u64).context("ftruncate")?;
        let map = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                size,
                rustix::mm::ProtFlags::READ | rustix::mm::ProtFlags::WRITE,
                rustix::mm::MapFlags::SHARED,
                &fd,
                0,
            )
            .context("mmap")?
        };

        let shm = self.state.shm.as_ref().unwrap();
        let pool = shm.create_pool(fd.as_fd(), size as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, &self.qh, ());

        let frame = session.create_frame(&self.qh, ());
        frame.attach_buffer(&buffer);
        frame.capture();

        while !self.state.cap.ready && self.state.cap.failed.is_none() {
            self.queue.blocking_dispatch(&mut self.state)?;
        }

        let result = if let Some(reason) = self.state.cap.failed {
            Err(anyhow::anyhow!("capture échouée: {reason:?}"))
        } else {
            // Copy out and convert the source format to RGBA8.
            let raw = unsafe { std::slice::from_raw_parts(map as *const u8, size) };
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let s = y * stride as usize + x * layout.bpp;
                    let d = (y * w as usize + x) * 4;
                    rgba[d] = raw[s + layout.r];
                    rgba[d + 1] = raw[s + layout.g];
                    rgba[d + 2] = raw[s + layout.b];
                    rgba[d + 3] = match layout.a {
                        Some(a) => raw[s + a],
                        None => 255,
                    };
                }
            }
            Ok(CapturedImage {
                width: w,
                height: h,
                rgba,
            })
        };

        buffer.destroy();
        pool.destroy();
        frame.destroy();
        session.destroy();
        src.destroy();
        unsafe {
            let _ = rustix::mm::munmap(map, size);
        }
        result
    }
}

// --- Dispatch ---

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.pending.push((toplevel, PendingToplevel::default()));
        }
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_foreign_toplevel_handle_v1::Event;
        let Some((_, p)) = state.pending.iter_mut().find(|(h, _)| h == handle) else {
            return;
        };
        match event {
            Event::Identifier { identifier } => p.identifier = identifier,
            Event::Title { title } => p.title = title,
            Event::AppId { app_id } => p.app_id = app_id,
            Event::Done => {
                if let Some(pos) = state.pending.iter().position(|(h, _)| h == handle) {
                    let (h, p) = state.pending.remove(pos);
                    state.toplevels.push(Toplevel {
                        handle: h,
                        identifier: p.identifier,
                        title: p.title,
                        app_id: p.app_id,
                    });
                }
            }
            Event::Closed => {
                state.pending.retain(|(h, _)| h != handle);
                state.toplevels.retain(|t| &t.handle != handle);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: <WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_output::Event;
        if let Event::Name { name } = event {
            if let Some(o) = state.outputs.iter_mut().find(|o| &o.wl_output == output) {
                o.name = name;
            } else {
                state.outputs.push(Output {
                    wl_output: output.clone(),
                    name,
                });
            }
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => {
                state.cap.width = width;
                state.cap.height = height;
            }
            Event::ShmFormat {
                format: WEnum::Value(f),
            } => state.cap.format = Some(f),
            Event::Done => state.cap.constraints_done = true,
            Event::Stopped => state.cap.failed = Some(FailureReason::Stopped),
            // We use the shm path; dmabuf constraints are ignored.
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Ready => state.cap.ready = true,
            Event::Failed { reason } => {
                state.cap.failed = Some(match reason {
                    WEnum::Value(r) => r,
                    _ => FailureReason::Unknown,
                })
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayland_client::protocol::wl_shm::Format;

    /// The heart of the grim-vs-wlr-chooser fix: bytes-per-pixel (hence stride) must
    /// match the advertised format. Bgr888 is 24-bit, so stride = width*3, not *4.
    #[test]
    fn pixel_layout_stride_and_alpha() {
        assert_eq!(PixelLayout::of(Format::Bgr888).unwrap().bpp, 3);
        assert_eq!(PixelLayout::of(Format::Rgb888).unwrap().bpp, 3);
        assert_eq!(PixelLayout::of(Format::Xrgb8888).unwrap().bpp, 4);
        assert_eq!(PixelLayout::of(Format::Argb8888).unwrap().bpp, 4);

        assert!(PixelLayout::of(Format::Bgr888).unwrap().a.is_none());
        assert!(PixelLayout::of(Format::Xrgb8888).unwrap().a.is_none());
        assert_eq!(PixelLayout::of(Format::Argb8888).unwrap().a, Some(3));
        assert_eq!(PixelLayout::of(Format::Abgr8888).unwrap().a, Some(3));
    }

    #[test]
    fn pixel_layout_unknown_format_is_none() {
        // A format we don't decode should be reported, not silently mishandled.
        assert!(PixelLayout::of(Format::C8).is_none());
    }
}

// Objects whose events we don't need.
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore ExtImageCaptureSourceV1);
delegate_noop!(State: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtImageCopyCaptureManagerV1);
