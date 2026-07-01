//! EGL/GLES core: dma-buf → GL texture import and headless readback.
//!
//! This is the windowing- and egui-free half of the GPU path. It loads the
//! `EGL_EXT_image_dma_buf_import` entry points and wraps a capture dma-buf as an
//! `EGLImage`, and provides [`GpuReadback`] — an offscreen context that reads such a
//! dma-buf back to CPU RGBA8. The egui display toolkit ([`crate::render`], gated by
//! the `toolkit` feature) builds on these primitives; this module stays available in
//! headless builds (record/timelapse) that need readback but no UI.
//!
//! Extension function pointers are loaded at runtime via `eglGetProcAddress`
//! (edgefirst-egl has no typed bindings for these).

use crate::wl;
use crate::error::{CaptureError, Context, Result};
use edgefirst_egl as egl;
use std::ffi::c_void;
use std::os::fd::AsRawFd;
use std::sync::Arc;
use wayland_client::Connection;

pub(crate) type Egl = egl::Instance<egl::Dynamic<libloading::Library, egl::EGL1_4>>;

pub(crate) type EglImage = *mut c_void;
pub(crate) const EGL_LINUX_DMA_BUF_EXT: u32 = 0x3270;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: i32 = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: i32 = 0x3444;
const EGL_ATTRIB_NONE: i32 = 0x3038;
pub(crate) const GL_TEXTURE_2D: u32 = 0x0DE1;

type EglCreateImageKhr =
    unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *const i32) -> EglImage;
type EglDestroyImageKhr = unsafe extern "system" fn(*mut c_void, EglImage) -> u32;
type GlEglImageTargetTexture2dOes = unsafe extern "system" fn(u32, EglImage);

/// Resolved EGL/GL extension entry points + the EGL display, for dma-buf import.
#[derive(Clone, Copy)]
pub(crate) struct DmabufEgl {
    pub(crate) display: *mut c_void,
    pub(crate) create_image: EglCreateImageKhr,
    pub(crate) destroy_image: EglDestroyImageKhr,
    pub(crate) image_target: GlEglImageTargetTexture2dOes,
}

/// Load the dma-buf import entry points. `None` if the driver lacks them (then there
/// is no GPU import path and callers fall back to whatever shm provided).
pub(crate) fn load_dmabuf_egl(egl: &Egl, display: egl::Display) -> Option<DmabufEgl> {
    let create = egl.get_proc_address("eglCreateImageKHR")?;
    let destroy = egl.get_proc_address("eglDestroyImageKHR")?;
    let target = egl.get_proc_address("glEGLImageTargetTexture2DOES")?;
    // Same calling convention (extern "system"), just typed signatures.
    Some(unsafe {
        DmabufEgl {
            display: display.as_ptr(),
            create_image: std::mem::transmute::<extern "system" fn(), EglCreateImageKhr>(create),
            destroy_image: std::mem::transmute::<extern "system" fn(), EglDestroyImageKhr>(destroy),
            image_target: std::mem::transmute::<extern "system" fn(), GlEglImageTargetTexture2dOes>(
                target,
            ),
        }
    })
}

/// Build the `EGL_LINUX_DMA_BUF_EXT` attribute list for a single-plane frame. The
/// fd is only borrowed (EGL dups it in `eglCreateImageKHR`), so `frame` must outlive
/// the import call.
pub(crate) fn dmabuf_image_attribs(frame: &wl::DmabufFrame) -> [i32; 17] {
    [
        EGL_WIDTH,
        frame.width as i32,
        EGL_HEIGHT,
        frame.height as i32,
        EGL_LINUX_DRM_FOURCC_EXT,
        frame.fourcc as i32,
        EGL_DMA_BUF_PLANE0_FD_EXT,
        frame.fd.as_raw_fd(),
        EGL_DMA_BUF_PLANE0_OFFSET_EXT,
        frame.offset as i32,
        EGL_DMA_BUF_PLANE0_PITCH_EXT,
        frame.stride as i32,
        EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
        (frame.modifier & 0xffff_ffff) as i32,
        EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
        (frame.modifier >> 32) as i32,
        EGL_ATTRIB_NONE,
    ]
}

/// Headless EGL/GLES context for reading a capture dma-buf back to CPU RGBA pixels.
///
/// The GPU capture path hands out a [`wl::DmabufFrame`] (zero-copy, meant for
/// display). Tools that ultimately need CPU pixels — screenshot encoding today,
/// video/timelapse on the roadmap — use this to import that dma-buf as a GL texture
/// and `glReadPixels` it into RGBA8. It runs without a window: a 1×1 pbuffer keeps
/// it portable across drivers that lack surfaceless contexts. Build one and reuse it
/// across frames (the EGL setup is not free).
pub struct GpuReadback {
    // Keeps the Wayland display (whose ptr backs the EGL display) alive.
    _conn: Connection,
    egl: Egl,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
    gl: Arc<glow::Context>,
    dmabuf_egl: Option<DmabufEgl>,
}

impl GpuReadback {
    /// Create the offscreen context. Errors (rather than panicking, unlike
    /// `Gpu::new`) since callers can fall back to the shm capture path.
    pub fn new() -> Result<Self> {
        let conn = Connection::connect_to_env().context("Wayland connection")?;
        let lib = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required() }
            .context("libEGL not found")?;
        let egl: Egl = lib;

        let display_ptr = conn.backend().display_ptr() as *mut c_void;
        let display = unsafe { egl.get_display(display_ptr) }.context("eglGetDisplay")?;
        egl.initialize(display).context("eglInitialize")?;
        egl.bind_api(egl::OPENGL_ES_API).context("eglBindAPI")?;

        let attribs = [
            egl::SURFACE_TYPE,
            egl::PBUFFER_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attribs)
            .context("eglChooseConfig")?
            .context("no EGL pbuffer config")?;

        let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
        let context = egl
            .create_context(display, config, None, &ctx_attribs)
            .or_else(|_| {
                let a = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
                egl.create_context(display, config, None, &a)
            })
            .context("eglCreateContext")?;

        let pb_attribs = [egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE];
        let surface = egl
            .create_pbuffer_surface(display, config, &pb_attribs)
            .context("eglCreatePbufferSurface")?;
        egl.make_current(display, Some(surface), Some(surface), Some(context))
            .context("eglMakeCurrent")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                egl.get_proc_address(s)
                    .map_or(std::ptr::null(), |p| p as *const _)
            })
        };
        let dmabuf_egl = load_dmabuf_egl(&egl, display);

        Ok(GpuReadback {
            _conn: conn,
            egl,
            display,
            surface,
            context,
            gl: Arc::new(gl),
            dmabuf_egl,
        })
    }

    /// Import `frame`'s dma-buf as a GL texture and read it back to RGBA8. Alpha is
    /// forced opaque: captures are opaque and XRGB dma-bufs leave the X byte
    /// undefined (mirrors the shm path's handling of alpha-less formats).
    pub fn readback(&mut self, frame: wl::DmabufFrame) -> Result<wl::CapturedImage> {
        let egl = self
            .dmabuf_egl
            .context("EGL dma-buf import unavailable (driver)")?;
        let (w, h) = (frame.width, frame.height);
        if w == 0 || h == 0 {
            return Err(CaptureError::msg("dimensions de readback nulles"));
        }

        self.egl
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .context("eglMakeCurrent")?;

        let attribs = dmabuf_image_attribs(&frame);
        let image = unsafe {
            (egl.create_image)(
                egl.display,
                std::ptr::null_mut(),
                EGL_LINUX_DMA_BUF_EXT,
                std::ptr::null_mut(),
                attribs.as_ptr(),
            )
        };
        if image.is_null() {
            return Err(CaptureError::msg("eglCreateImageKHR failed"));
        }

        // Import → bind to a texture → attach to an FBO → glReadPixels. Always
        // destroy the EGLImage afterwards, success or not.
        let read = self.read_image_to_rgba(&egl, image, w, h);
        unsafe { (egl.destroy_image)(egl.display, image) };
        let mut rgba = read?;

        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(wl::CapturedImage {
            width: w,
            height: h,
            rgba,
        })
    }

    /// Bind `image` to a fresh texture + FBO and read it back, cleaning up the GL
    /// objects before returning. Split out so [`Self::readback`] can destroy the
    /// EGLImage on every path.
    fn read_image_to_rgba(
        &self,
        egl: &DmabufEgl,
        image: EglImage,
        w: u32,
        h: u32,
    ) -> Result<Vec<u8>> {
        use glow::HasContext as _;
        unsafe {
            let tex = self
                .gl
                .create_texture()
                .map_err(|e| CaptureError::msg(format!("glGenTextures: {e}")))?;
            self.gl.bind_texture(GL_TEXTURE_2D, Some(tex));
            (egl.image_target)(GL_TEXTURE_2D, image);

            let fbo = self.gl.create_framebuffer().map_err(|e| {
                self.gl.delete_texture(tex);
                CaptureError::msg(format!("glGenFramebuffers: {e}"))
            })?;
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                Some(tex),
                0,
            );

            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            let result = if status == glow::FRAMEBUFFER_COMPLETE {
                let mut buf = vec![0u8; w as usize * h as usize * 4];
                self.gl.read_pixels(
                    0,
                    0,
                    w as i32,
                    h as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut buf)),
                );
                Ok(buf)
            } else {
                Err(CaptureError::msg(format!("FBO de readback incomplet (0x{status:x})")))
            };

            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            self.gl.delete_framebuffer(fbo);
            self.gl.bind_texture(GL_TEXTURE_2D, None);
            self.gl.delete_texture(tex);
            result
        }
    }
}
