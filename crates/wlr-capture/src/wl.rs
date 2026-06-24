//! Native Wayland client: enumerate foreign toplevels and outputs, and capture
//! them via `ext-image-copy-capture-v1`.
//!
//! The whole point of doing this natively (instead of shelling out to `grim -T`)
//! is to create the shm buffer with the *correct* stride (`width * 4`), which is
//! where grim 1.5 trips up ("Invalid stride") on some toplevels (Firefox, …).

use anyhow::{Context, Result, bail};
#[cfg(feature = "gpu")]
use gbm::{BufferObject, BufferObjectFlags, Device as GbmDevice, Format as GbmFormat, Modifier};
use rustix::event::{PollFd, PollFlags, Timespec};
use std::collections::HashMap;
use std::ffi::c_void;
#[cfg(feature = "gpu")]
use std::fs::File;
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    backend::ObjectId,
    delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::{self, Transform, WlOutput},
        wl_registry::WlRegistry,
        wl_seat::WlSeat,
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
    },
};
#[cfg(feature = "gpu")]
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// DRM "invalid"/"let the driver choose" modifier sentinel — not a real layout.
#[cfg(feature = "gpu")]
const DRM_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
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
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};

/// A capturable window.
#[derive(Clone)]
pub struct Toplevel {
    pub handle: ExtForeignToplevelHandleV1,
    pub identifier: String,
    pub title: String,
    pub app_id: String,
}

/// A capturable output, with its placement in the global logical space.
///
/// Logical position/size come from `xdg-output` (`zxdg_output_manager_v1`) when the
/// compositor exposes it — the only reliable source for multi-monitor positions and
/// fractional-scale logical sizes. Physical pixel size, integer scale and transform
/// come from `wl_output`; if `xdg-output` is absent we fall back to computing the
/// logical size from those.
#[derive(Clone)]
pub struct Output {
    pub wl_output: WlOutput,
    pub name: String,
    /// Top-left position in the global logical coordinate space.
    pub logical_x: i32,
    pub logical_y: i32,
    /// Logical size from xdg-output (0 until received; see [`Output::logical_size`]).
    pub logical_w: i32,
    pub logical_h: i32,
    /// Resolution of the current mode, in physical pixels (pre-transform).
    pub phys_width: i32,
    pub phys_height: i32,
    /// Integer buffer scale (wl_output; may be coarser than the real scale).
    pub scale: i32,
    /// Output transform (rotation/flip); swaps logical width/height for 90/270.
    pub transform: Transform,
    /// Whether xdg-output supplied authoritative logical geometry.
    pub have_xdg: bool,
}

/// Logical dimensions from physical pixels: divide by `scale`, swapping
/// width/height for 90°/270° transforms. Free function so it's unit-testable
/// without a live `WlOutput`.
fn logical_dims(phys_w: i32, phys_h: i32, scale: i32, transform: Transform) -> (i32, i32) {
    let s = scale.max(1);
    let (w, h) = (phys_w / s, phys_h / s);
    if matches!(
        transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    ) {
        (h, w)
    } else {
        (w, h)
    }
}

impl Output {
    /// Logical size (points). Prefers xdg-output's authoritative size (handles
    /// fractional scale); otherwise physical pixels divided by the integer scale,
    /// with width/height swapped for 90°/270° transforms.
    pub fn logical_size(&self) -> (i32, i32) {
        if self.have_xdg && self.logical_w > 0 && self.logical_h > 0 {
            (self.logical_w, self.logical_h)
        } else {
            logical_dims(
                self.phys_width,
                self.phys_height,
                self.scale,
                self.transform,
            )
        }
    }
}

/// An axis-aligned rectangle. Used both for capture cropping (in an image's pixel
/// space) and for selection geometry (in the global logical space), so `x`/`y` may
/// be negative; `w`/`h` are unsigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// The overlapping rectangle of two regions, or `None` if they don't overlap.
    pub fn intersect(&self, o: &Region) -> Option<Region> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w as i32).min(o.x + o.w as i32);
        let y1 = (self.y + self.h as i32).min(o.y + o.h as i32);
        (x1 > x0 && y1 > y0).then_some(Region {
            x: x0,
            y: y0,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        })
    }
}

impl Output {
    /// The output's placement in the global logical space, as a [`Region`].
    pub fn logical_rect(&self) -> Region {
        let (w, h) = self.logical_size();
        Region {
            x: self.logical_x,
            y: self.logical_y,
            w: w.max(0) as u32,
            h: h.max(0) as u32,
        }
    }
}

/// Decoded RGBA8 image.
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CapturedImage {
    /// The RGBA bytes of the pixel at `(x, y)`, or `None` if out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        self.rgba.get(i..i + 4).map(|s| [s[0], s[1], s[2], s[3]])
    }

    /// Crop to `rect` (in this image's pixel space), clamped to the bounds. Returns
    /// the overlapping sub-image; empty (0×0) if there is no overlap.
    pub fn crop(&self, rect: Region) -> CapturedImage {
        let bounds = Region {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        };
        let Some(r) = rect.intersect(&bounds) else {
            return CapturedImage {
                width: 0,
                height: 0,
                rgba: Vec::new(),
            };
        };
        let row_bytes = (r.w * 4) as usize;
        let mut out = vec![0u8; row_bytes * r.h as usize];
        for row in 0..r.h {
            let sy = r.y as u32 + row;
            let src = ((sy * self.width + r.x as u32) * 4) as usize;
            let dst = row as usize * row_bytes;
            out[dst..dst + row_bytes].copy_from_slice(&self.rgba[src..src + row_bytes]);
        }
        CapturedImage {
            width: r.w,
            height: r.h,
            rgba: out,
        }
    }

    /// Composite this image into a `dst_w × dst_h` RGBA8 buffer at `(at_x, at_y)`,
    /// clipping to the destination. Used to stitch per-output captures into one
    /// multi-output region.
    pub fn blit_into(&self, dst: &mut [u8], dst_w: u32, dst_h: u32, at_x: i32, at_y: i32) {
        let dst_rect = Region {
            x: 0,
            y: 0,
            w: dst_w,
            h: dst_h,
        };
        let src_rect = Region {
            x: at_x,
            y: at_y,
            w: self.width,
            h: self.height,
        };
        let Some(r) = src_rect.intersect(&dst_rect) else {
            return;
        };
        let row_bytes = (r.w * 4) as usize;
        for row in 0..r.h {
            let dy = r.y as u32 + row;
            let sy = (r.y - at_y) as u32 + row;
            let sx = (r.x - at_x) as u32;
            let src = ((sy * self.width + sx) * 4) as usize;
            let dpos = ((dy * dst_w + r.x as u32) * 4) as usize;
            dst[dpos..dpos + row_bytes].copy_from_slice(&self.rgba[src..src + row_bytes]);
        }
    }
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

    /// Same, keyed by DRM fourcc (for dma-buf). DRM 32-bit codes use the same
    /// little-endian memory order as their wl_shm counterparts.
    #[cfg(feature = "gpu")]
    fn of_fourcc(f: u32) -> Option<Self> {
        Some(match f {
            // XR24 / AR24: little-endian B,G,R,(X|A)
            f if f == fourcc(b'X', b'R', b'2', b'4') => Self::of(wl_shm::Format::Xrgb8888)?,
            f if f == fourcc(b'A', b'R', b'2', b'4') => Self::of(wl_shm::Format::Argb8888)?,
            // XB24 / AB24: little-endian R,G,B,(X|A)
            f if f == fourcc(b'X', b'B', b'2', b'4') => Self::of(wl_shm::Format::Xbgr8888)?,
            f if f == fourcc(b'A', b'B', b'2', b'4') => Self::of(wl_shm::Format::Abgr8888)?,
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

/// Opaque handle to a persistent capture session (the session object's id).
pub type SessionId = ObjectId;

/// Per-session bookkeeping, updated by the session/frame Dispatch impls and keyed
/// by the session object id so multiple live sessions never clobber each other.
#[derive(Default)]
struct SessionData {
    /// Latest buffer constraints advertised by the compositor.
    width: u32,
    height: u32,
    format: Option<wl_shm::Format>,
    /// dma-buf device the compositor wants buffers allocated on (raw dev_t).
    #[cfg(feature = "gpu")]
    dmabuf_dev: Option<u64>,
    /// dma-buf formats advertised: (drm fourcc, supported modifiers).
    #[cfg(feature = "gpu")]
    dmabuf_formats: Vec<(u32, Vec<u64>)>,
    /// Set once a constraints group (`done`) has been received.
    constraints_done: bool,
    /// Constraints changed since the buffer was last (re)allocated (e.g. resize).
    dirty: bool,
    /// Set when the current in-flight frame is ready to read.
    ready: bool,
    /// A transient per-frame failure (retry next round); `buffer_constraints`
    /// additionally triggers a reallocation. Not terminal.
    frame_failed: Option<FailureReason>,
    /// Terminal: the session/source stopped and won't produce more frames.
    stopped: bool,
}

/// A reusable buffer backing one session, kept alive between frames. Either a
/// CPU shm buffer (fallback) or a GPU dma-buf swapchain allocated through gbm.
enum Buf {
    Shm(ShmBuf),
    #[cfg(feature = "gpu")]
    Dmabuf(DmaBuf),
}

impl Buf {
    fn wl_buffer(&self) -> &WlBuffer {
        match self {
            Buf::Shm(b) => &b.buffer,
            #[cfg(feature = "gpu")]
            Buf::Dmabuf(b) => &b.buffer,
        }
    }
    /// Did the advertised constraints (size) change vs this buffer?
    fn matches(&self, w: u32, h: u32) -> bool {
        match self {
            Buf::Shm(b) => b.width == w && b.height == h,
            #[cfg(feature = "gpu")]
            Buf::Dmabuf(b) => b.width == w && b.height == h,
        }
    }
}

/// CPU shm buffer with a correct, format-specific stride.
struct ShmBuf {
    pool: WlShmPool,
    buffer: WlBuffer,
    _fd: OwnedFd,
    map: *mut c_void,
    size: usize,
    width: u32,
    height: u32,
    stride: usize,
    format: wl_shm::Format,
}

impl Drop for ShmBuf {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
        unsafe {
            let _ = rustix::mm::munmap(self.map, self.size);
        }
    }
}

/// One dma-buf allocated via gbm: the compositor captures into it, and it is
/// imported zero-copy as a GL texture for display.
///
/// Single-buffered on purpose: `ext-image-copy-capture` captures *incrementally*
/// by damage, assuming the buffer it's given already holds the previous frame.
/// Reusing one buffer lets it accumulate the full image; alternating buffers
/// would leave undamaged regions of the "other" buffer empty (black) for static
/// windows. Sampling the buffer while the compositor updates a small damage
/// region is imperceptible at thumbnail scale.
#[cfg(feature = "gpu")]
struct DmaBuf {
    buffer: WlBuffer,
    bo: BufferObject<()>,
    width: u32,
    height: u32,
    fourcc: u32,
    modifier: u64,
    stride: u32,
    offset: u32,
}

#[cfg(feature = "gpu")]
impl Drop for DmaBuf {
    fn drop(&mut self) {
        self.buffer.destroy();
        // `bo` drops here, releasing the underlying dma-buf.
    }
}

/// A captured frame handed to the UI: either CPU pixels (shm) or a dma-buf
/// descriptor to import as a GL texture (GPU, zero-copy).
pub enum Frame {
    Shm(CapturedImage),
    // Constructed only with the `gpu` feature; the display side (EGL import) is
    // always built since it needs no gbm.
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    Dmabuf(DmabufFrame),
}

/// dma-buf descriptor for zero-copy GL import on the UI thread. `fd` is owned by
/// the receiver; `buf_id` identifies the swapchain slot so the importer can cache
/// one GL texture per slot (their backing memory is stable).
pub struct DmabufFrame {
    pub fd: OwnedFd,
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub modifier: u64,
    pub stride: u32,
    pub offset: u32,
}

/// A persistent capture session: source + session objects plus the reusable
/// buffer. Re-armed each frame instead of being torn down (the one-shot model).
/// `frame` holds the in-flight capture (a frame object captures exactly one
/// frame), pending until the source produces new content (damage).
struct OpenSession {
    frame: Option<ExtImageCopyCaptureFrameV1>, // in-flight capture, if armed
    buf: Option<Buf>,                          // dropped after the frame, before the session
    session: ExtImageCopyCaptureSessionV1,
    src: ExtImageCaptureSourceV1,
}

impl Drop for OpenSession {
    fn drop(&mut self) {
        if let Some(frame) = &self.frame {
            frame.destroy();
        }
        self.session.destroy();
        self.src.destroy();
    }
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
    /// linux-dmabuf manager, if the compositor exposes it (enables the GPU path).
    #[cfg(feature = "gpu")]
    dmabuf: Option<ZwpLinuxDmabufV1>,
    /// Event bookkeeping for every live session, keyed by session object id.
    sessions: HashMap<ObjectId, SessionData>,
}

pub struct Client {
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    /// Session-owned Wayland objects + buffers, keyed by session object id.
    open: HashMap<ObjectId, OpenSession>,
    /// gbm device for dma-buf allocation, opened lazily on the first dma-buf
    /// session (matching the compositor's advertised device). `None` until then,
    /// or if the GPU path is unavailable (we then fall back to shm).
    #[cfg(feature = "gpu")]
    gbm: Option<GbmDevice<File>>,
}

impl Client {
    /// Connect, bind the capture managers, and enumerate windows + outputs.
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("Wayland connection")?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).context("registre Wayland")?;
        let qh = queue.handle();

        let shm = globals.bind(&qh, 1..=1, ()).context("wl_shm")?;
        let copy = globals
            .bind(&qh, 1..=1, ())
            .context("ext_image_copy_capture_manager_v1 missing")?;
        let tl_src = globals
            .bind(&qh, 1..=1, ())
            .context("ext_foreign_toplevel_image_capture_source_manager_v1 missing")?;
        let out_src = globals
            .bind(&qh, 1..=1, ())
            .context("ext_output_image_capture_source_manager_v1 missing")?;
        let _list: ExtForeignToplevelListV1 = globals
            .bind(&qh, 1..=1, ())
            .context("ext_foreign_toplevel_list_v1 missing")?;

        // Optional: authoritative logical geometry (multi-monitor positions,
        // fractional scale). Absent on a few compositors — we then fall back to
        // wl_output-derived sizes.
        let xdg_mgr: Option<ZxdgOutputManagerV1> = globals.bind(&qh, 1..=3, ()).ok();

        globals.contents().with_list(|list| {
            for g in list {
                if g.interface == WlOutput::interface().name {
                    let out: WlOutput = globals.registry().bind(g.name, g.version.min(4), &qh, ());
                    if let Some(mgr) = &xdg_mgr {
                        // udata = the wl_output, so the xdg_output's logical-geometry
                        // events update the matching Output.
                        mgr.get_xdg_output(&out, &qh, out.clone());
                    }
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
        // Optional: enables the GPU dma-buf path. Absence just means shm-only.
        #[cfg(feature = "gpu")]
        {
            state.dmabuf = globals.bind(&qh, 3..=4, ()).ok();
        }
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;

        Ok(Self {
            queue,
            qh,
            state,
            open: HashMap::new(),
            #[cfg(feature = "gpu")]
            gbm: None,
        })
    }

    pub fn toplevels(&self) -> &[Toplevel] {
        &self.state.toplevels
    }
    pub fn outputs(&self) -> &[Output] {
        &self.state.outputs
    }

    /// Drain pending Wayland events (new/closed toplevels, etc.) without blocking
    /// on a capture, so the source list stays current between capture rounds.
    pub fn refresh(&mut self) -> Result<()> {
        self.queue.roundtrip(&mut self.state)?;
        Ok(())
    }

    /// Open a persistent capture session for a window. The session and its buffer
    /// live until [`Client::close_session`] (or the source disappears); re-arm a
    /// frame each cycle with [`Client::capture`].
    pub fn open_toplevel_session(&mut self, t: &Toplevel) -> Result<SessionId> {
        let src = self
            .state
            .tl_src
            .as_ref()
            .unwrap()
            .create_source(&t.handle, &self.qh, ());
        self.open_session(src)
    }

    /// Open a persistent capture session for an output. See [`Client::open_toplevel_session`].
    pub fn open_output_session(&mut self, o: &Output) -> Result<SessionId> {
        let src = self
            .state
            .out_src
            .as_ref()
            .unwrap()
            .create_source(&o.wl_output, &self.qh, ());
        self.open_session(src)
    }

    fn open_session(&mut self, src: ExtImageCaptureSourceV1) -> Result<SessionId> {
        let session =
            self.state
                .copy
                .as_ref()
                .unwrap()
                .create_session(&src, Options::empty(), &self.qh, ());
        let id = session.id();
        self.state
            .sessions
            .insert(id.clone(), SessionData::default());

        // Wait for the first buffer-constraints group (buffer_size + shm_format + done).
        loop {
            self.queue.blocking_dispatch(&mut self.state)?;
            let d = self.state.sessions.get(&id).unwrap();
            if d.constraints_done || d.stopped {
                break;
            }
        }
        if self.state.sessions.get(&id).unwrap().stopped {
            self.state.sessions.remove(&id);
            session.destroy();
            src.destroy();
            bail!("capture session stopped before first frame");
        }

        self.open.insert(
            id.clone(),
            OpenSession {
                frame: None,
                buf: None,
                session,
                src,
            },
        );
        Ok(id)
    }

    /// Tear down a session (e.g. its window closed).
    pub fn close_session(&mut self, id: &SessionId) {
        self.open.remove(id); // Drop releases frame + buffer + session + source
        self.state.sessions.remove(id);
    }

    /// One-shot: capture a single frame of `output`, then tear the session down.
    /// Blocks up to `budget`. For screenshots / timelapse ticks.
    pub fn capture_output_once(&mut self, output: &Output, budget: Duration) -> Result<Frame> {
        let id = self.open_output_session(output)?;
        let r = self.poll_one(&id, budget);
        self.close_session(&id);
        r
    }

    /// One-shot: capture a single frame of `toplevel`, then tear the session down.
    pub fn capture_toplevel_once(
        &mut self,
        toplevel: &Toplevel,
        budget: Duration,
    ) -> Result<Frame> {
        let id = self.open_toplevel_session(toplevel)?;
        let r = self.poll_one(&id, budget);
        self.close_session(&id);
        r
    }

    /// Poll until session `id` yields a frame, it stops, or `budget` elapses.
    /// Frames from other open sessions in this round are discarded.
    fn poll_one(&mut self, id: &SessionId, budget: Duration) -> Result<Frame> {
        let deadline = Instant::now() + budget;
        loop {
            let now = Instant::now();
            if now >= deadline {
                bail!("capture: timed out");
            }
            let step = Duration::from_millis(50).min(deadline - now);
            let (frames, stopped) = self.poll(step);
            for (sid, frame) in frames {
                if &sid == id {
                    return Ok(frame);
                }
            }
            if stopped.iter().any(|s| s == id) {
                bail!("capture: session stopped before first frame");
            }
        }
    }

    /// Drive all open sessions for up to `budget`: arm a frame on every idle
    /// session, wait for events, and return the frames that became ready (the
    /// sources that produced new content). Sessions whose source is static simply
    /// keep their frame armed and deliver nothing — which is exactly right, there
    /// is nothing new to show.
    ///
    /// Also returns the ids of sessions the compositor stopped (e.g. their window
    /// closed), so the caller can drop and (if still listed) reopen them.
    pub fn poll(&mut self, budget: Duration) -> (Vec<(SessionId, Frame)>, Vec<SessionId>) {
        // 1. Arm every session that has no frame in flight.
        let ids: Vec<ObjectId> = self.open.keys().cloned().collect();
        for id in &ids {
            let armed = self.open.get(id).is_some_and(|o| o.frame.is_some());
            let dead = self.state.sessions.get(id).is_some_and(|d| d.stopped);
            if armed || dead {
                continue;
            }
            if self.ensure_buffer(id).is_err() {
                continue;
            }
            if let Some(d) = self.state.sessions.get_mut(id) {
                d.ready = false;
            }
            let os = self.open.get_mut(id).unwrap();
            let wl_buffer = os.buf.as_ref().unwrap().wl_buffer().clone();
            let frame = os.session.create_frame(&self.qh, id.clone());
            frame.attach_buffer(&wl_buffer);
            frame.capture();
            os.frame = Some(frame);
        }

        // 2. Wait for frame events, but never longer than the budget.
        let _ = self.dispatch_timeout(budget);

        // 3. Harvest ready frames; retry transient frame failures; surface stops.
        let mut frames = Vec::new();
        let mut stopped = Vec::new();
        for id in self.open.keys().cloned().collect::<Vec<_>>() {
            let (ready, is_stopped, frame_failed) = self
                .state
                .sessions
                .get(&id)
                .map(|d| (d.ready, d.stopped, d.frame_failed))
                .unwrap_or((false, false, None));

            // Terminal: source gone. Drop the in-flight frame and report it.
            if is_stopped {
                if let Some(os) = self.open.get_mut(&id) {
                    if let Some(frame) = os.frame.take() {
                        frame.destroy();
                    }
                }
                stopped.push(id);
                continue;
            }

            if ready {
                let frame = harvest(self.open[&id].buf.as_ref().unwrap());
                if let Some(os) = self.open.get_mut(&id) {
                    if let Some(f) = os.frame.take() {
                        f.destroy();
                    }
                }
                if let Some(d) = self.state.sessions.get_mut(&id) {
                    d.ready = false;
                    d.frame_failed = None;
                }
                if let Some(frame) = frame {
                    frames.push((id, frame));
                }
            } else if let Some(reason) = frame_failed {
                // Transient: drop the failed frame and re-arm next round. A
                // buffer_constraints failure also means our buffer is stale.
                if let Some(os) = self.open.get_mut(&id) {
                    if let Some(f) = os.frame.take() {
                        f.destroy();
                    }
                }
                if let Some(d) = self.state.sessions.get_mut(&id) {
                    d.frame_failed = None;
                    if matches!(reason, FailureReason::BufferConstraints) {
                        d.dirty = true; // size/format changed → reallocate
                    }
                }
            }
        }
        (frames, stopped)
    }

    /// Dispatch Wayland events for at most `budget`, returning early once the fd
    /// goes quiet. Mirrors the crate's `blocking_read` but with a `poll` timeout
    /// so a desktop with no damage doesn't block us forever.
    fn dispatch_timeout(&mut self, budget: Duration) -> Result<()> {
        self.queue.dispatch_pending(&mut self.state)?;
        self.queue.flush()?;
        let deadline = Instant::now() + budget;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Some(guard) = self.queue.prepare_read() else {
                // Events already queued: dispatch them and re-check.
                self.queue.dispatch_pending(&mut self.state)?;
                continue;
            };
            let ts = Timespec {
                tv_sec: remaining.as_secs() as _,
                tv_nsec: remaining.subsec_nanos() as _,
            };
            // Scope the borrowed fd so it is released before `guard.read()` (which
            // consumes the guard).
            let poll_res = {
                let fd = guard.connection_fd();
                let mut fds = [PollFd::new(&fd, PollFlags::IN | PollFlags::ERR)];
                rustix::event::poll(&mut fds, Some(&ts))
            };
            match poll_res {
                Ok(0) => break, // timeout: no events within the budget
                Ok(_) => {
                    guard.read().context("reading Wayland events")?;
                    self.queue.dispatch_pending(&mut self.state)?;
                }
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(anyhow::anyhow!("poll: {e}")),
            }
        }
        Ok(())
    }

    /// Ensure the session has a usable buffer, (re)allocating only when it is
    /// missing or the size changed (window resized). Prefers the GPU dma-buf path
    /// and falls back to shm. The buffer is reused across frames otherwise.
    fn ensure_buffer(&mut self, id: &SessionId) -> Result<()> {
        let (w, h, dirty) = {
            let d = self.state.sessions.get(id).context("session inconnue")?;
            (d.width, d.height, d.dirty)
        };
        let fits = self
            .open
            .get(id)
            .and_then(|o| o.buf.as_ref())
            .is_some_and(|b| b.matches(w, h));
        // Reuse unless the size changed or a buffer_constraints failure marked it
        // dirty (then reallocate even at the same size).
        if fits && !dirty {
            return Ok(());
        }
        if w == 0 || h == 0 {
            bail!("dimensions de capture nulles");
        }

        // Prefer dma-buf (GPU); fall back to shm if it isn't available/usable.
        #[cfg(feature = "gpu")]
        let buf = match self.alloc_dmabuf(id, w, h) {
            Some(b) => b,
            None => self.alloc_shm(id, w, h)?,
        };
        #[cfg(not(feature = "gpu"))]
        let buf = self.alloc_shm(id, w, h)?;
        // Install the new buffer; the old one (if any) drops here, releasing it.
        self.open.get_mut(id).context("session non ouverte")?.buf = Some(buf);
        self.state.sessions.get_mut(id).unwrap().dirty = false;
        Ok(())
    }

    /// Allocate a CPU shm buffer with the format-correct stride.
    fn alloc_shm(&mut self, id: &SessionId, w: u32, h: u32) -> Result<Buf> {
        let format = self
            .state
            .sessions
            .get(id)
            .and_then(|d| d.format)
            .context("compositor offered no shm format")?;
        let layout = PixelLayout::of(format)
            .with_context(|| format!("unsupported shm format: {format:?}"))?;
        let stride = w as usize * layout.bpp; // stride from the format's actual bpp
        let size = stride * h as usize;

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
        let pool =
            self.state
                .shm
                .as_ref()
                .unwrap()
                .create_pool(fd.as_fd(), size as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, &self.qh, ());
        Ok(Buf::Shm(ShmBuf {
            pool,
            buffer,
            _fd: fd,
            map,
            size,
            width: w,
            height: h,
            stride,
            format,
        }))
    }

    /// Try to allocate a dma-buf (via gbm) and wrap it as a wl_buffer. Returns
    /// `None` whenever the GPU path isn't usable (no manager, no suitable format,
    /// gbm/allocation failure) so the caller falls back to shm.
    #[cfg(feature = "gpu")]
    fn alloc_dmabuf(&mut self, id: &SessionId, w: u32, h: u32) -> Option<Buf> {
        let dmabuf_mgr = self.state.dmabuf.as_ref().cloned()?;
        let (formats, dev) = {
            let d = self.state.sessions.get(id)?;
            (d.dmabuf_formats.clone(), d.dmabuf_dev)
        };
        let Some((fourcc, mods)) = pick_dmabuf_format(&formats) else {
            if debug() {
                eprintln!("wlr-capture: no usable dma-buf format");
            }
            return None;
        };
        self.ensure_gbm(dev)?;
        let gbm = self.gbm.as_ref()?;
        let gfmt = GbmFormat::try_from(fourcc).ok()?;
        let qh = &self.qh;

        // Allocate one swapchain slot: a gbm bo wrapped as a dma-buf wl_buffer.
        let alloc_slot = || -> Option<DmaBuf> {
            let bo = gbm
                .create_buffer_object_with_modifiers2::<()>(
                    w,
                    h,
                    gfmt,
                    mods.iter().map(|&m| Modifier::from(m)),
                    BufferObjectFlags::RENDERING,
                )
                .ok()?;
            let stride = bo.stride();
            let offset = bo.offset(0);
            let modifier: u64 = bo.modifier().into();
            let fd = bo.fd().ok()?;

            let params = dmabuf_mgr.create_params(qh, ());
            params.add(
                fd.as_fd(),
                0,
                offset,
                stride,
                (modifier >> 32) as u32,
                (modifier & 0xffff_ffff) as u32,
            );
            let buffer = params.create_immed(
                w as i32,
                h as i32,
                fourcc,
                zwp_linux_buffer_params_v1::Flags::empty(),
                qh,
                (),
            );
            params.destroy();
            Some(DmaBuf {
                buffer,
                bo,
                width: w,
                height: h,
                fourcc,
                modifier,
                stride,
                offset,
            })
        };

        let buf = alloc_slot()?;
        if debug() {
            eprintln!(
                "wlr-chooser: dma-buf {w}x{h} fourcc={fourcc:#010x} modifier={}",
                buf.modifier
            );
        }
        Some(Buf::Dmabuf(buf))
    }

    /// Open the gbm device for the compositor's advertised dma-buf device, once.
    /// Returns `None` (so callers fall back to shm) if it can't be opened.
    #[cfg(feature = "gpu")]
    fn ensure_gbm(&mut self, dev: Option<u64>) -> Option<()> {
        if self.gbm.is_some() {
            return Some(());
        }
        let path = render_node_for(dev);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .ok()?;
        let device = GbmDevice::new(file).ok()?;
        if debug() {
            eprintln!("wlr-chooser: gbm device {}", path.display());
        }
        self.gbm = Some(device);
        Some(())
    }
}

/// Build a DRM fourcc code from its four ASCII bytes.
#[cfg(feature = "gpu")]
const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

/// Pick a dma-buf format we can both allocate and decode, plus its usable
/// modifiers (dropping `INVALID`). Prefers the common 32-bit RGB layouts.
#[cfg(feature = "gpu")]
fn pick_dmabuf_format(formats: &[(u32, Vec<u64>)]) -> Option<(u32, Vec<u64>)> {
    let preferred = [
        fourcc(b'X', b'R', b'2', b'4'), // XRGB8888
        fourcc(b'A', b'R', b'2', b'4'), // ARGB8888
        fourcc(b'X', b'B', b'2', b'4'), // XBGR8888
        fourcc(b'A', b'B', b'2', b'4'), // ABGR8888
    ];
    for want in preferred {
        if PixelLayout::of_fourcc(want).is_none() {
            continue;
        }
        if let Some((_, mods)) = formats.iter().find(|(f, _)| *f == want) {
            let usable: Vec<u64> = mods
                .iter()
                .copied()
                .filter(|&m| m != DRM_MOD_INVALID)
                .collect();
            if !usable.is_empty() {
                return Some((want, usable));
            }
        }
    }
    None
}

/// Resolve the DRM render node to allocate on. Best effort: match the advertised
/// dev_t against `/dev/dri/renderD*`, else the first render node, else renderD128.
#[cfg(feature = "gpu")]
fn render_node_for(dev: Option<u64>) -> std::path::PathBuf {
    use std::path::PathBuf;
    let render_nodes = || -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("renderD"))
            })
            .collect();
        v.sort();
        v
    };
    let nodes = render_nodes();
    if let Some(dev) = dev {
        for p in &nodes {
            if rustix::fs::stat(p).is_ok_and(|st| st.st_rdev == dev) {
                return p.clone();
            }
        }
    }
    nodes
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("/dev/dri/renderD128"))
}

/// Whether verbose capture diagnostics are enabled (`WLR_CHOOSER_DEBUG`).
#[cfg(feature = "gpu")]
fn debug() -> bool {
    std::env::var_os("WLR_CHOOSER_DEBUG").is_some()
}

/// Turn a ready capture into a [`Frame`] for the UI. shm is read back + converted
/// to RGBA on the CPU; dma-buf is handed off zero-copy as an fd to import as a GL
/// texture (re-exporting an fd for the buffer the compositor just wrote).
fn harvest(buf: &Buf) -> Option<Frame> {
    match buf {
        Buf::Shm(b) => {
            let layout = PixelLayout::of(b.format).expect("format validated at alloc time");
            let raw = unsafe { std::slice::from_raw_parts(b.map as *const u8, b.size) };
            Some(Frame::Shm(convert(
                raw, b.width, b.height, b.stride, &layout,
            )))
        }
        #[cfg(feature = "gpu")]
        Buf::Dmabuf(b) => {
            let fd = b.bo.fd().ok()?;
            Some(Frame::Dmabuf(DmabufFrame {
                fd,
                width: b.width,
                height: b.height,
                fourcc: b.fourcc,
                modifier: b.modifier,
                stride: b.stride,
                offset: b.offset,
            }))
        }
    }
}

/// Pixel-format conversion to RGBA8 shared by the shm and dma-buf paths.
fn convert(raw: &[u8], w: u32, h: u32, stride: usize, layout: &PixelLayout) -> CapturedImage {
    let (w, h) = (w as usize, h as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = y * stride + x * layout.bpp;
            let d = (y * w + x) * 4;
            rgba[d] = raw[s + layout.r];
            rgba[d + 1] = raw[s + layout.g];
            rgba[d + 2] = raw[s + layout.b];
            rgba[d + 3] = match layout.a {
                Some(a) => raw[s + a],
                None => 255,
            };
        }
    }
    CapturedImage {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

// --- Window activation (zwlr-foreign-toplevel-management) ---
//
// Capture uses ext-foreign-toplevel-list (stable `identifier`), but activation
// needs zwlr handles, a separate object namespace. We correlate the two by
// app_id + title — the only key both expose. This is a self-contained, one-shot
// path on its own connection, run after the picker closes (so our overlay's
// keyboard grab is already gone and focus can move to the target).

/// Enumeration state for [`activate_window`].
#[derive(Default)]
struct ActState {
    /// (handle, app_id, title) for every advertised toplevel.
    toplevels: Vec<(ZwlrForeignToplevelHandleV1, String, String)>,
}

/// Focus the window matching `app_id` + `title` via zwlr-foreign-toplevel-manager.
/// `dup_index` selects among identical (app_id, title) windows by creation order
/// (both ext-foreign-toplevel-list and zwlr enumerate in that order on wlroots),
/// so the right one is focused even with duplicates.
pub fn activate_window(app_id: &str, title: &str, dup_index: usize) -> Result<()> {
    let conn = Connection::connect_to_env().context("Wayland connection")?;
    let (globals, mut queue) =
        registry_queue_init::<ActState>(&conn).context("registre Wayland")?;
    let qh = queue.handle();
    let _mgr: ZwlrForeignToplevelManagerV1 = globals
        .bind(&qh, 1..=3, ())
        .context("zwlr_foreign_toplevel_manager_v1 missing (unsupported compositor)")?;
    let seat: WlSeat = globals.bind(&qh, 1..=8, ()).context("wl_seat missing")?;

    // Binding the manager makes the compositor advertise current toplevels.
    let mut st = ActState::default();
    queue.roundtrip(&mut st)?;
    queue.roundtrip(&mut st)?;

    let handle = st
        .toplevels
        .iter()
        .filter(|(_, a, t)| a == app_id && t == title)
        .nth(dup_index)
        .or_else(|| {
            st.toplevels
                .iter()
                .find(|(_, a, t)| a == app_id && t == title)
        })
        .map(|(h, _, _)| h.clone())
        .with_context(|| format!("window to activate not found: {app_id} / {title}"))?;
    handle.activate(&seat);
    queue.roundtrip(&mut st)?; // flush the activate request
    Ok(())
}

/// List the Wayland globals the current compositor advertises, as
/// `(interface, version)`. Used by `wlr-peek doctor` to report which capture
/// protocols (and therefore which features) are available.
pub fn advertised_globals() -> Result<Vec<(String, u32)>> {
    let conn = Connection::connect_to_env().context("Wayland connection")?;
    let (globals, _queue) = registry_queue_init::<ActState>(&conn).context("registre Wayland")?;
    let mut list = Vec::new();
    globals.contents().with_list(|globals| {
        for g in globals {
            list.push((g.interface.clone(), g.version));
        }
    });
    Ok(list)
}

impl Dispatch<WlRegistry, GlobalListContents> for ActState {
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

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for ActState {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state
                .toplevels
                .push((toplevel, String::new(), String::new()));
        }
    }

    event_created_child!(ActState, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for ActState {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Event;
        let Some(e) = state.toplevels.iter_mut().find(|(h, _, _)| h == handle) else {
            return;
        };
        match event {
            Event::AppId { app_id } => e.1 = app_id,
            Event::Title { title } => e.2 = title,
            _ => {}
        }
    }
}

delegate_noop!(ActState: ignore WlSeat);

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

impl State {
    /// The `Output` for `wl_output`, created (with neutral geometry) on first sight
    /// so `geometry`/`mode`/`scale` can land before `name`.
    fn output_entry(&mut self, output: &WlOutput) -> &mut Output {
        if let Some(i) = self.outputs.iter().position(|o| &o.wl_output == output) {
            return &mut self.outputs[i];
        }
        self.outputs.push(Output {
            wl_output: output.clone(),
            name: String::new(),
            logical_x: 0,
            logical_y: 0,
            logical_w: 0,
            logical_h: 0,
            phys_width: 0,
            phys_height: 0,
            scale: 1,
            transform: Transform::Normal,
            have_xdg: false,
        });
        self.outputs.last_mut().unwrap()
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
        match event {
            Event::Geometry {
                x, y, transform, ..
            } => {
                let o = state.output_entry(output);
                o.transform = transform.into_result().unwrap_or(Transform::Normal);
                // wl_output position is only a fallback; xdg-output is authoritative.
                if !o.have_xdg {
                    o.logical_x = x;
                    o.logical_y = y;
                }
            }
            // Keep only the active mode's resolution (physical pixels).
            Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                if flags
                    .into_result()
                    .is_ok_and(|f| f.contains(wl_output::Mode::Current))
                {
                    let o = state.output_entry(output);
                    o.phys_width = width;
                    o.phys_height = height;
                }
            }
            Event::Scale { factor } => {
                state.output_entry(output).scale = factor.max(1);
            }
            Event::Name { name } => {
                state.output_entry(output).name = name;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, WlOutput> for State {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: <ZxdgOutputV1 as Proxy>::Event,
        wl_output: &WlOutput,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zxdg_output_v1::Event;
        match event {
            Event::LogicalPosition { x, y } => {
                let o = state.output_entry(wl_output);
                o.logical_x = x;
                o.logical_y = y;
                o.have_xdg = true;
            }
            Event::LogicalSize { width, height } => {
                let o = state.output_entry(wl_output);
                o.logical_w = width;
                o.logical_h = height;
                o.have_xdg = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        session: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        let Some(d) = state.sessions.get_mut(&session.id()) else {
            return;
        };
        match event {
            Event::BufferSize { width, height } => {
                d.width = width;
                d.height = height;
            }
            Event::ShmFormat {
                format: WEnum::Value(f),
            } => d.format = Some(f),
            #[cfg(feature = "gpu")]
            Event::DmabufDevice { device } => {
                // dev_t as a native-endian byte array.
                if device.len() == 8 {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&device);
                    d.dmabuf_dev = Some(u64::from_ne_bytes(b));
                }
            }
            #[cfg(feature = "gpu")]
            Event::DmabufFormat { format, modifiers } => {
                // modifiers: array of native-endian u64.
                let mods = modifiers
                    .chunks_exact(8)
                    .map(|c| u64::from_ne_bytes(c.try_into().unwrap()))
                    .collect();
                d.dmabuf_formats.push((format, mods));
            }
            // A constraints group ends with `done`; flag a (re)allocation so a
            // resize between frames grows the buffer.
            Event::Done => {
                d.constraints_done = true;
                d.dirty = true;
            }
            Event::Stopped => d.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ObjectId> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        session_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        let Some(d) = state.sessions.get_mut(session_id) else {
            return;
        };
        match event {
            Event::Ready => d.ready = true,
            Event::Failed { reason } => {
                let reason = match reason {
                    WEnum::Value(r) => r,
                    _ => FailureReason::Unknown,
                };
                // Per the protocol, a frame failure means destroy the frame, not
                // the session. Only `stopped` is terminal; the rest are transient.
                if matches!(reason, FailureReason::Stopped) {
                    d.stopped = true;
                } else {
                    d.frame_failed = Some(reason);
                }
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

    #[test]
    fn region_intersect() {
        let a = Region {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let b = Region {
            x: 5,
            y: 5,
            w: 10,
            h: 10,
        };
        assert_eq!(
            a.intersect(&b),
            Some(Region {
                x: 5,
                y: 5,
                w: 5,
                h: 5
            })
        );
        // Negative origin (selection partly off the image) clamps correctly.
        let c = Region {
            x: -3,
            y: -3,
            w: 6,
            h: 6,
        };
        assert_eq!(
            a.intersect(&c),
            Some(Region {
                x: 0,
                y: 0,
                w: 3,
                h: 3
            })
        );
        // Disjoint → None.
        let d = Region {
            x: 100,
            y: 100,
            w: 1,
            h: 1,
        };
        assert_eq!(a.intersect(&d), None);
    }

    /// A 2×2 RGBA image: four distinct pixels, to verify pixel/crop addressing.
    fn img_2x2() -> CapturedImage {
        CapturedImage {
            width: 2,
            height: 2,
            rgba: vec![
                1, 1, 1, 255, 2, 2, 2, 255, // row 0: (0,0)=1, (1,0)=2
                3, 3, 3, 255, 4, 4, 4, 255, // row 1: (0,1)=3, (1,1)=4
            ],
        }
    }

    #[test]
    fn captured_pixel_and_crop() {
        let img = img_2x2();
        assert_eq!(img.pixel(0, 0), Some([1, 1, 1, 255]));
        assert_eq!(img.pixel(1, 1), Some([4, 4, 4, 255]));
        assert_eq!(img.pixel(2, 0), None); // out of bounds

        // Crop the bottom-right 1×1 pixel.
        let c = img.crop(Region {
            x: 1,
            y: 1,
            w: 1,
            h: 1,
        });
        assert_eq!((c.width, c.height), (1, 1));
        assert_eq!(c.rgba, vec![4, 4, 4, 255]);

        // Crop overrunning the bounds clamps to the overlap.
        let c2 = img.crop(Region {
            x: 1,
            y: 0,
            w: 5,
            h: 5,
        });
        assert_eq!((c2.width, c2.height), (1, 2));
        assert_eq!(c2.rgba, vec![2, 2, 2, 255, 4, 4, 4, 255]);
    }

    #[test]
    fn captured_blit_into() {
        // Blit the 2×2 image into a 3×2 black canvas at x=1, clipping the overflow.
        let img = img_2x2();
        let (dw, dh) = (3u32, 2u32);
        let mut dst = vec![0u8; (dw * dh * 4) as usize];
        img.blit_into(&mut dst, dw, dh, 1, 0);
        // Column 0 stays black; columns 1..3 get the image's two columns.
        assert_eq!(&dst[0..4], &[0, 0, 0, 0]); // (0,0)
        assert_eq!(&dst[4..8], &[1, 1, 1, 255]); // (1,0) = img (0,0)
        assert_eq!(&dst[8..12], &[2, 2, 2, 255]); // (2,0) = img (1,0)
        assert_eq!(&dst[12..16], &[0, 0, 0, 0]); // (0,1)
        assert_eq!(&dst[16..20], &[3, 3, 3, 255]); // (1,1) = img (0,1)
    }

    #[test]
    fn output_logical_dims_transform() {
        // 4K at scale 2 → 1920×1080 logical.
        assert_eq!(logical_dims(3840, 2160, 2, Transform::Normal), (1920, 1080));
        // 90°/270° swap width and height.
        assert_eq!(logical_dims(3840, 2160, 2, Transform::_90), (1080, 1920));
        assert_eq!(
            logical_dims(3840, 2160, 2, Transform::Flipped270),
            (1080, 1920)
        );
        // 180° keeps orientation; scale 0 is treated as 1.
        assert_eq!(logical_dims(1000, 500, 0, Transform::_180), (1000, 500));
    }
}

// Objects whose events we don't need.
delegate_noop!(State: ignore ZxdgOutputManagerV1);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore ExtImageCaptureSourceV1);
delegate_noop!(State: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtImageCopyCaptureManagerV1);
// dma-buf: we drive allocation ourselves (gbm) and create buffers with
// `create_immed`, so the manager's format/modifier and the params' created/failed
// events carry nothing we need.
#[cfg(feature = "gpu")]
delegate_noop!(State: ignore ZwpLinuxDmabufV1);
#[cfg(feature = "gpu")]
delegate_noop!(State: ignore ZwpLinuxBufferParamsV1);
