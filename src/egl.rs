//! Native graphics ownership. All use is single-threaded; Rc keeps the EGL
//! display/context alive until every drawable is gone, and Connection keeps
//! libwayland's native display alive through eglTerminate.
use crate::Result;
use khronos_egl as egl;
use std::rc::Rc;
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};
use wayland_egl::WlEglSurface;

pub struct Gpu {
    api: egl::Instance<egl::Static>,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    _connection: Connection,
}

impl Gpu {
    pub fn new(connection: Connection) -> Result<Rc<Self>> {
        if !wayland_egl::is_available() {
            return Err("libwayland-egl is unavailable".into());
        }
        let api = egl::Instance::new(egl::Static);
        // SAFETY: system-backend Connection owns this live libwayland display.
        let display = unsafe { api.get_display(connection.backend().display_ptr().cast()) }
            .ok_or("EGL could not open the Wayland display")?;
        api.initialize(display)?;
        let setup = (|| -> Result<_> {
            api.bind_api(egl::OPENGL_ES_API)?;
            // XP requests no multisampling. Depth belongs to the retained target;
            // allocating a second window depth buffer wastes large-display memory.
            let config = api
                .choose_first_config(
                    display,
                    &[
                        egl::SURFACE_TYPE,
                        egl::WINDOW_BIT,
                        egl::RENDERABLE_TYPE,
                        egl::OPENGL_ES3_BIT,
                        egl::RED_SIZE,
                        8,
                        egl::GREEN_SIZE,
                        8,
                        egl::BLUE_SIZE,
                        8,
                        egl::ALPHA_SIZE,
                        0,
                        egl::DEPTH_SIZE,
                        0,
                        egl::SAMPLE_BUFFERS,
                        0,
                        egl::SAMPLES,
                        0,
                        egl::NONE,
                    ],
                )?
                .ok_or("no non-multisampled GLES 3 EGL configuration")?;
            if api.get_config_attrib(display, config, egl::SAMPLE_BUFFERS)? != 0 {
                return Err("EGL selected multisampling for a copy-presentation target".into());
            }
            let context = api.create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE],
            )?;
            Ok((config, context))
        })();
        let (config, context) = match setup {
            Ok(value) => value,
            Err(error) => {
                let _ = api.terminate(display);
                return Err(error);
            }
        };
        Ok(Rc::new(Self {
            api,
            display,
            config,
            context,
            _connection: connection,
        }))
    }

    pub fn drawable(self: &Rc<Self>, surface: &WlSurface, size: [i32; 2]) -> Result<Drawable> {
        let window = WlEglSurface::new(surface.id(), size[0], size[1])?;
        // SAFETY: the window is a live wl_egl_window for our display. It remains
        // owned by Drawable until after its EGLSurface is destroyed.
        let egl_surface = unsafe {
            self.api
                .create_window_surface(self.display, self.config, window.ptr().cast_mut(), None)
        }?;
        let drawable = Drawable {
            gpu: self.clone(),
            egl_surface,
            window,
        };
        drawable.make_current()?;
        // wl_surface.frame already paces us. Mesa's separate EGL vsync wait can
        // otherwise block inside swap_buffers after an output is disabled,
        // preventing dispatch of its removal/configure events and our deadline.
        self.api.swap_interval(self.display, 0)?;
        Ok(drawable)
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let _ = self.api.make_current(self.display, None, None, None);
        // GL objects are context-owned: destroying this sole context frees all
        // buffers/programs, even if initialization failed or the last output was
        // unplugged. Retained targets own a drawable so their Drop can first
        // make this context current; context loss still falls back to teardown.
        let _ = self.api.destroy_context(self.display, self.context);
        let _ = self.api.terminate(self.display);
        let _ = self.api.release_thread();
    }
}

pub struct Drawable {
    gpu: Rc<Gpu>,
    egl_surface: egl::Surface,
    window: WlEglSurface,
}

impl Drawable {
    pub fn resize(&self, size: [i32; 2]) {
        self.window.resize(size[0], size[1], 0, 0);
    }

    pub fn make_current(&self) -> Result<()> {
        self.gpu.api.make_current(
            self.gpu.display,
            Some(self.egl_surface),
            Some(self.egl_surface),
            Some(self.gpu.context),
        )?;
        Ok(())
    }

    pub fn load_gl(&self) -> Result<glow::Context> {
        self.make_current()?;
        // SAFETY: this drawable has made a GLES 3 context current. EGL resolves
        // its entry points; Gpu outlives every use of the resulting private GL API.
        Ok(unsafe {
            glow::Context::from_loader_function(|name| {
                self.gpu
                    .api
                    .get_proc_address(name)
                    .map_or(std::ptr::null(), |p| p as *const _)
            })
        })
    }

    pub fn swap(&self) -> Result<()> {
        self.gpu
            .api
            .swap_buffers(self.gpu.display, self.egl_surface)?;
        Ok(())
    }
}

impl crate::renderer::RenderSurface for Drawable {
    fn make_current(&self) -> Result<()> {
        Self::make_current(self)
    }
    fn load_gl(&self) -> Result<glow::Context> {
        Self::load_gl(self)
    }
}

impl Drop for Drawable {
    fn drop(&mut self) {
        let _ = self
            .gpu
            .api
            .make_current(self.gpu.display, None, None, None);
        let _ = self
            .gpu
            .api
            .destroy_surface(self.gpu.display, self.egl_surface);
        // window drops next; the containing surface record drops its shell role
        // (which destroys wl_surface) only after this entire Drawable is gone.
    }
}
