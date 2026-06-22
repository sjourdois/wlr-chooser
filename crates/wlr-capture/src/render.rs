//! Shared egui → `egui_glow` rendering core on an EGL/GLES context bound to a
//! Wayland surface, plus zero-copy dma-buf → GL texture import.
//!
//! This is the toolkit half of `wlr-capture`: any windowing host (the
//! `wlr-chooser` layer-shell overlay, the `wlr-pip` xdg-toplevel mirror, …) binds
//! a [`Gpu`] to its `wl_surface` and drives one egui frame per repaint with
//! [`Gpu::render`]. The host owns the GL context, so it (via the importer handed
//! to the UI closure) turns capture dma-bufs into drawable textures.

use crate::wl;
use khronos_egl as egl;
use std::collections::HashMap;
use std::ffi::c_void;
use std::os::fd::AsRawFd;
use std::sync::Arc;
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

type Egl = egl::Instance<egl::Dynamic<libloading::Library, egl::EGL1_4>>;

/// Host-side importer for GPU dma-buf frames. The windowing host owns the GL
/// context, so it (not a toolkit-agnostic UI) turns a dma-buf into a drawable
/// egui texture. Returns the texture id + source pixel size.
pub trait DmabufImporter {
    fn import(
        &mut self,
        key: &str,
        frame: wl::DmabufFrame,
    ) -> Option<(egui::TextureId, egui::Vec2)>;
    /// Release any GPU resources cached for a source that went away.
    fn forget(&mut self, key: &str);
}

// --- dma-buf → GL texture import (EGL_EXT_image_dma_buf_import) ---
//
// The capture thread hands us a dma-buf fd; we wrap it in an EGLImage and bind it
// to a GL texture egui can sample — zero copy, no readback. Function pointers are
// loaded at runtime via eglGetProcAddress (khronos-egl has no typed bindings for
// these extensions).

type EglImage = *mut c_void;
const EGL_LINUX_DMA_BUF_EXT: u32 = 0x3270;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: i32 = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: i32 = 0x3444;
const EGL_ATTRIB_NONE: i32 = 0x3038;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE_SWIZZLE_A: u32 = 0x8E45;
const GL_ONE: i32 = 1;

type EglCreateImageKhr =
    unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *const i32) -> EglImage;
type EglDestroyImageKhr = unsafe extern "system" fn(*mut c_void, EglImage) -> u32;
type GlEglImageTargetTexture2dOes = unsafe extern "system" fn(u32, EglImage);

/// Resolved EGL/GL extension entry points + the EGL display, for dma-buf import.
#[derive(Clone, Copy)]
struct DmabufEgl {
    display: *mut c_void,
    create_image: EglCreateImageKhr,
    destroy_image: EglDestroyImageKhr,
    image_target: GlEglImageTargetTexture2dOes,
}

/// Load the dma-buf import entry points. `None` if the driver lacks them (then we
/// have no GPU display path and tiles fall back to whatever shm provided).
fn load_dmabuf_egl(egl: &Egl, display: egl::Display) -> Option<DmabufEgl> {
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

/// A dma-buf imported as a GL texture, cached per source key.
struct NativeTex {
    image: EglImage,
    tex: glow::Texture,
    id: egui::TextureId,
    size: egui::Vec2,
}

/// Host-side [`DmabufImporter`]: turns a dma-buf fd into a GL texture egui can
/// sample. Borrows the painter (to register native textures) and the persistent
/// texture cache; `egl` is `None` if the driver can't import dma-bufs.
struct HostImporter<'a> {
    egl: Option<DmabufEgl>,
    gl: Arc<glow::Context>,
    painter: &'a mut egui_glow::Painter,
    cache: &'a mut HashMap<String, NativeTex>,
}

impl DmabufImporter for HostImporter<'_> {
    fn import(
        &mut self,
        key: &str,
        frame: wl::DmabufFrame,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        use glow::HasContext as _;
        let egl = self.egl?;
        let size = egui::vec2(frame.width as f32, frame.height as f32);
        let attribs: [i32; 17] = [
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
        ];
        // EGL_NO_CONTEXT for dma-buf import; EGL dups the fd, so we may close ours.
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
            return None;
        }

        let ckey = key.to_string();
        // Refresh the existing texture in place (the dma-buf is the same kernel
        // object; just rebind the fresh image), keeping a stable egui texture id.
        if let Some(nt) = self.cache.get_mut(&ckey) {
            unsafe {
                self.gl.bind_texture(GL_TEXTURE_2D, Some(nt.tex));
                (egl.image_target)(GL_TEXTURE_2D, image);
                self.gl.bind_texture(GL_TEXTURE_2D, None);
                (egl.destroy_image)(egl.display, nt.image);
            }
            nt.image = image;
            nt.size = size;
            return Some((nt.id, nt.size));
        }

        // First frame for this slot: create the GL texture and register it.
        let tex = unsafe {
            let t = self.gl.create_texture().ok()?;
            self.gl.bind_texture(GL_TEXTURE_2D, Some(t));
            let lin = glow::LINEAR as i32;
            let clamp = glow::CLAMP_TO_EDGE as i32;
            self.gl
                .tex_parameter_i32(GL_TEXTURE_2D, glow::TEXTURE_MIN_FILTER, lin);
            self.gl
                .tex_parameter_i32(GL_TEXTURE_2D, glow::TEXTURE_MAG_FILTER, lin);
            self.gl
                .tex_parameter_i32(GL_TEXTURE_2D, glow::TEXTURE_WRAP_S, clamp);
            self.gl
                .tex_parameter_i32(GL_TEXTURE_2D, glow::TEXTURE_WRAP_T, clamp);
            // Captured buffers are XRGB (no real alpha): the X byte is undefined,
            // so force sampled alpha to 1, else egui blends with garbage alpha.
            self.gl
                .tex_parameter_i32(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_A, GL_ONE);
            (egl.image_target)(GL_TEXTURE_2D, image);
            self.gl.bind_texture(GL_TEXTURE_2D, None);
            t
        };
        let id = self.painter.register_native_texture(tex);
        self.cache.insert(
            ckey,
            NativeTex {
                image,
                tex,
                id,
                size,
            },
        );
        Some((id, size))
    }

    fn forget(&mut self, key: &str) {
        use glow::HasContext as _;
        let Some(egl) = self.egl else { return };
        if let Some(nt) = self.cache.remove(key) {
            self.painter.free_texture(nt.id);
            unsafe {
                self.gl.delete_texture(nt.tex);
                (egl.destroy_image)(egl.display, nt.image);
            }
        }
    }
}

/// EGL/GL state bound to a `wl_surface`, created once the surface has its first
/// size. Owns the egui_glow painter and the dma-buf texture cache.
pub struct Gpu {
    egl: Egl,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
    egl_window: wayland_egl::WlEglSurface,
    painter: egui_glow::Painter,
    /// dma-buf import entry points, if the driver supports them.
    dmabuf_egl: Option<DmabufEgl>,
    /// dma-buf textures imported for display, keyed by source key.
    dmabuf_tex: HashMap<String, NativeTex>,
}

impl Gpu {
    /// Build the EGL/GLES context for `surface` at physical size `pw`×`ph`.
    /// Panics on EGL setup failure (the host can't render without it).
    pub fn new(conn: &Connection, surface: &WlSurface, pw: i32, ph: i32) -> Gpu {
        let lib = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required() }
            .expect("libEGL introuvable");
        let egl: Egl = lib;

        let display_ptr = conn.backend().display_ptr() as *mut c_void;
        let display = unsafe { egl.get_display(display_ptr).expect("eglGetDisplay") };
        egl.initialize(display).expect("eglInitialize");
        egl.bind_api(egl::OPENGL_ES_API).expect("eglBindAPI");

        let attribs = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
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
            .expect("eglChooseConfig")
            .expect("aucune config EGL avec alpha");

        let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
        let context = egl
            .create_context(display, config, None, &ctx_attribs)
            .or_else(|_| {
                let a = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
                egl.create_context(display, config, None, &a)
            })
            .expect("eglCreateContext");

        let egl_window = wayland_egl::WlEglSurface::new(surface.id(), pw, ph).expect("wl_egl");
        let egl_surface = unsafe {
            egl.create_window_surface(
                display,
                config,
                egl_window.ptr() as egl::NativeWindowType,
                None,
            )
            .expect("eglCreateWindowSurface")
        };
        egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
            .expect("eglMakeCurrent");

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                egl.get_proc_address(s)
                    .map_or(std::ptr::null(), |p| p as *const _)
            })
        };
        let painter = egui_glow::Painter::new(Arc::new(gl), "", None, false).expect("egui_glow");
        let dmabuf_egl = load_dmabuf_egl(&egl, display);
        if dmabuf_egl.is_none() {
            eprintln!("wlr-capture: import dma-buf EGL indisponible (affichage GPU désactivé)");
        }

        Gpu {
            egl,
            display,
            surface: egl_surface,
            context,
            egl_window,
            painter,
            dmabuf_egl,
            dmabuf_tex: HashMap::new(),
        }
    }

    /// Resize the EGL window to a new physical size (after a surface configure /
    /// scale change).
    pub fn resize(&self, pw: i32, ph: i32) {
        self.egl_window.resize(pw, ph, 0, 0);
    }

    /// Run one egui frame and present it. `run_ui` builds the UI; it is handed the
    /// dma-buf importer (this owns the GL context) so capture frames become
    /// drawable textures. `backdrop` is the GL clear colour (premultiplied gamma).
    pub fn render(
        &mut self,
        egui_ctx: &egui::Context,
        raw_input: egui::RawInput,
        ppp: f32,
        size_px: (u32, u32),
        backdrop: [f32; 4],
        mut run_ui: impl FnMut(&egui::Context, &mut dyn DmabufImporter),
    ) {
        let (pw, ph) = size_px;
        self.egl
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .ok();

        // Run the UI. GPU dma-buf frames are imported here via the host importer,
        // since that needs the painter + GL context.
        let (prims, textures_delta) = {
            let gl = self.painter.gl().clone();
            let mut importer = HostImporter {
                egl: self.dmabuf_egl,
                gl,
                painter: &mut self.painter,
                cache: &mut self.dmabuf_tex,
            };
            let full = egui_ctx.run(raw_input, |ctx| run_ui(ctx, &mut importer));
            (egui_ctx.tessellate(full.shapes, ppp), full.textures_delta)
        };

        unsafe {
            use glow::HasContext as _;
            let gl = self.painter.gl();
            gl.viewport(0, 0, pw as i32, ph as i32);
            let [r, g, b, a] = backdrop;
            gl.clear_color(r, g, b, a);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.painter
            .paint_and_update_textures([pw, ph], ppp, &prims, &textures_delta);
        self.egl.swap_buffers(self.display, self.surface).ok();
    }
}
