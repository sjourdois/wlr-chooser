//! Shared egui → `egui_glow` rendering core on an EGL/GLES context bound to a
//! Wayland surface, plus zero-copy dma-buf → GL texture import.
//!
//! This is the toolkit half of `wlr-capture`: any windowing host (the
//! `wlr-chooser` layer-shell overlay, the `wlr-pip` xdg-toplevel mirror, …) binds
//! a [`Gpu`] to its `wl_surface` and drives one egui frame per repaint with
//! [`Gpu::render`]. The host owns the GL context, so it (via the importer handed
//! to the UI closure) turns capture dma-bufs into drawable textures.

use crate::gl::{
    DmabufEgl, EGL_LINUX_DMA_BUF_EXT, Egl, EglImage, GL_TEXTURE_2D, dmabuf_image_attribs,
    load_dmabuf_egl,
};
use crate::wl;
use edgefirst_egl as egl;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

/// Host-side importer for GPU dma-buf frames. The windowing host owns the GL
/// context, so it (not a toolkit-agnostic UI) turns a dma-buf into a drawable
/// egui texture. Returns the texture id + source pixel size.
pub trait DmabufImporter {
    /// Import `frame` as a GL-backed egui texture, caching it under `key` (the
    /// swapchain slot). Returns the texture id and the source pixel size, or `None`
    /// if the import fails.
    fn import(
        &mut self,
        key: &str,
        frame: wl::DmabufFrame,
    ) -> Option<(egui::TextureId, egui::Vec2)>;
    /// Release any GPU resources cached for a source that went away.
    fn forget(&mut self, key: &str);
}

// dma-buf → GL texture import: the EGL/GL core (entry points, EGLImage creation,
// readback) lives in `crate::gl`; here we only sample the imported texture for
// display. These two GL swizzle constants are display-only.
const GL_TEXTURE_SWIZZLE_A: u32 = 0x8E45;
const GL_ONE: i32 = 1;

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
        let attribs = dmabuf_image_attribs(&frame);
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
            .expect("libEGL not found");
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
            .expect("no EGL config with alpha");

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
            eprintln!("wlr-capture: EGL dma-buf import unavailable (GPU display disabled)");
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
        mut raw_input: egui::RawInput,
        ppp: f32,
        size_px: (u32, u32),
        backdrop: [f32; 4],
        mut run_ui: impl FnMut(&mut egui::Ui, &mut dyn DmabufImporter),
    ) {
        // Lay text out at the same pixels-per-point we tessellate with, or epaint warns
        // ("pixels_per_point have changed between text layout and tessellation") and
        // text shapes can be mis-scaled — happens on fractional/HiDPI outputs where
        // `ppp != 1.0` but the caller left it unset in `raw_input`.
        raw_input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);

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
            // `run_ui` hands the closure a full-screen root `Ui`; paint functions add
            // their panels into it with `show_inside`.
            let full = egui_ctx.run_ui(raw_input, |ui| run_ui(ui, &mut importer));
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
