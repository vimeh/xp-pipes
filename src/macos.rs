//! Native macOS bridge. `AppKit` calls this interface on its main thread only.
use crate::{
    Result,
    animation::Animation,
    config::{Config, Joints, Pipes},
    geometry::Scene,
    renderer::{History, RenderSurface, Renderer, history_bytes},
    rng::Random,
    texture::Texture,
    timing::FadeClock,
};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char, c_int, c_void},
    os::unix::ffi::OsStrExt,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    rc::{Rc, Weak},
    time::Instant,
};

#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {
    fn CGLRetainContext(context: *mut c_void) -> *mut c_void;
    fn CGLReleaseContext(context: *mut c_void);
    fn CGLSetCurrentContext(context: *mut c_void) -> c_int;
    fn CGLGetCurrentContext() -> *mut c_void;
}
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn pthread_main_np() -> c_int;
}

pub(crate) struct Cgl {
    raw: *mut c_void,
    library: *mut c_void,
}
impl Cgl {
    #[cfg(test)]
    pub(crate) fn new() -> Result<Self> {
        unsafe extern "C" {
            fn CGLChoosePixelFormat(
                attributes: *const u32,
                format: *mut *mut c_void,
                count: *mut c_int,
            ) -> c_int;
            fn CGLCreateContext(
                format: *mut c_void,
                share: *mut c_void,
                context: *mut *mut c_void,
            ) -> c_int;
            fn CGLDestroyPixelFormat(format: *mut c_void) -> c_int;
        }
        // SDK CGLTypes.h: OpenGLProfile, GL4_Core, Accelerated, AllowOfflineRenderers.
        let attributes = [99, 0x4100, 73, 96, 0];
        let mut format = std::ptr::null_mut();
        let mut raw = std::ptr::null_mut();
        let mut count = 0;
        // SAFETY: bounded attribute array and out-pointers; owned on this test thread.
        unsafe {
            let error = CGLChoosePixelFormat(attributes.as_ptr(), &raw mut format, &raw mut count);
            if error != 0 || format.is_null() {
                return Err(format!("CGL test format: {error}").into());
            }
            let error = CGLCreateContext(format, std::ptr::null_mut(), &raw mut raw);
            CGLDestroyPixelFormat(format);
            if error != 0 {
                return Err(format!("CGL test context: {error}").into());
            }
            let result = Self::retain(raw);
            CGLReleaseContext(raw);
            result
        }
    }

    // SAFETY: raw is a live CGLContextObj supplied by the native wrapper.
    unsafe fn retain(raw: *mut c_void) -> Result<Self> {
        if raw.is_null() {
            return Err("missing native GL context".into());
        }
        unsafe {
            let library = dlopen(
                c"/System/Library/Frameworks/OpenGL.framework/OpenGL".as_ptr(),
                1,
            );
            if library.is_null() {
                return Err("OpenGL framework could not be loaded".into());
            }
            Ok(Self {
                raw: CGLRetainContext(raw),
                library,
            })
        }
    }
}
impl RenderSurface for Cgl {
    fn make_current(&self) -> Result<()> {
        // SAFETY: this object retains the live context and never leaves the main thread.
        let error = unsafe { CGLSetCurrentContext(self.raw) };
        if error != 0 {
            return Err(format!("CGL context error {error}").into());
        }
        Ok(())
    }
    fn load_gl(&self) -> Result<glow::Context> {
        self.make_current()?;
        // SAFETY: current GL 4.1 context; the framework remains loaded through cleanup.
        Ok(unsafe {
            glow::Context::from_loader_function(|name| {
                let name = CString::new(name).expect("GL function name has no NUL");
                dlsym(self.library, name.as_ptr()).cast_const()
            })
        })
    }
}
impl Drop for Cgl {
    fn drop(&mut self) {
        // SAFETY: releases precisely our retain after all owned histories are gone.
        unsafe {
            if CGLGetCurrentContext() == self.raw {
                CGLSetCurrentContext(std::ptr::null_mut());
            }
            CGLReleaseContext(self.raw);
            dlclose(self.library);
        }
    }
}

struct Process {
    random: Random,
    fade: FadeClock,
    start: Instant,
    model_bytes: u64,
    gpu_bytes: u64,
}
thread_local! { static PROCESS: RefCell<Weak<RefCell<Process>>> = RefCell::default(); }

#[repr(C)]
pub struct Options {
    version: u32,
    multiple: u32,
    joints: u32,
    speed: u32,
    tessellation: u32,
    use_seed: u32,
    seed: u32,
    texture_path: *const c_char,
}

struct Saver {
    // Drop histories while the retained CGL context is still alive.
    history: Option<History>,
    renderer: Renderer,
    surface: Rc<Cgl>,
    animation: Animation,
    config: Config,
    process: Rc<RefCell<Process>>,
    model_bytes: u64,
    gpu_bytes: u64,
}
impl Saver {
    fn budget(&self, logical: [u32; 2], pixels: [i32; 2]) -> Result<(u64, u64)> {
        let dims = crate::geometry::layout(logical)?;
        let model = Scene::model_bytes(dims)
            .max(u64::try_from(self.animation.scene.retained_bytes())?)
            + u64::try_from(self.renderer.retained_bytes())?;
        let gpu = history_bytes(pixels)?
            + self.renderer.fixed_gpu_bytes()
            + if self.config.textured() {
                crate::geometry::mesh_bytes(self.config.tessellation, true)
            } else {
                0
            };
        let shared = self.process.borrow();
        check_budget(
            shared.model_bytes - self.model_bytes + model,
            shared.gpu_bytes - self.gpu_bytes + gpu,
        )?;
        Ok((model, gpu))
    }
    fn frame(&mut self, logical: [u32; 2], pixels: [i32; 2]) -> Result<()> {
        let (model, gpu) = self.budget(logical, pixels)?;
        self.animation.resize(&self.config, logical)?;
        {
            let mut shared = self.process.borrow_mut();
            shared.model_bytes = shared.model_bytes - self.model_bytes + model;
            shared.gpu_bytes = shared.gpu_bytes - self.gpu_bytes + gpu;
            self.model_bytes = model;
            self.gpu_bytes = gpu;
            let now = shared.start.elapsed().as_secs_f64();
            let Process { random, fade, .. } = &mut *shared;
            self.animation.render(now, random, fade)?;
        }
        self.renderer.draw(
            self.surface.clone(),
            &mut self.history,
            pixels,
            &self.animation.scene,
        )
    }
}
impl Drop for Saver {
    fn drop(&mut self) {
        self.history = None;
        if self.surface.make_current().is_ok() {
            // SAFETY: native context is current, histories are retired, and this
            // renderer cannot be used again after the enclosing engine is dropped.
            unsafe {
                self.renderer.delete_fixed();
            }
        }
        let mut process = self.process.borrow_mut();
        process.model_bytes -= self.model_bytes;
        process.gpu_bytes -= self.gpu_bytes;
    }
}

#[test]
fn resource_limits_include_all_live_views() {
    assert!(check_budget(128 * 1024 * 1024, 512 * 1024 * 1024).is_ok());
    assert!(check_budget(128 * 1024 * 1024 + 1, 0).is_err());
    assert!(check_budget(0, 512 * 1024 * 1024 + 1).is_err());
    let full = history_bytes([7680, 4320]).unwrap();
    assert!(check_budget(0, full * 3).is_err());
}

fn check_budget(model: u64, gpu: u64) -> Result<()> {
    if model > 128 * 1024 * 1024 || gpu > 512 * 1024 * 1024 {
        return Err("screen saver views exceed the shared resource budget".into());
    }
    Ok(())
}

fn boundary<T>(fallback: T, operation: impl FnOnce() -> Result<T>) -> T {
    // A Rust unwind must never cross the Objective-C/C ABI.
    if unsafe { pthread_main_np() } == 0 {
        eprintln!("XP Pipes: native calls require the main thread");
        return fallback;
    }
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            eprintln!("XP Pipes: {error}");
            fallback
        }
        Err(_) => {
            eprintln!("XP Pipes: rendering panic contained at native interface");
            fallback
        }
    }
}

/// Create a native view's engine without changing any system screen-saver policy.
/// # Safety
/// `context` must be a live `CGLContextObj`; `options` and its optional NUL-terminated
/// path must be readable for this call. All calls use the `AppKit` main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xp_pipes_create(
    context: *mut c_void,
    options: *const Options,
    width: u32,
    height: u32,
) -> *mut c_void {
    boundary(std::ptr::null_mut(), || {
        if options.is_null() {
            return Err("missing saver options".into());
        }
        // SAFETY: native wrapper owns this validated C-layout structure for this call.
        let options = unsafe { &*options };
        if options.version != 1
            || options.multiple > 1
            || options.joints > 3
            || options.speed > 20
            || options.tessellation > 4
            || options.use_seed > 1
        {
            return Err("invalid saver settings".into());
        }
        let mut config = Config {
            pipes: if options.multiple != 0 {
                Pipes::Multiple
            } else {
                Pipes::Fixed(1)
            },
            joints: [Joints::Elbow, Joints::Ball, Joints::Mixed, Joints::Cycle]
                [usize::try_from(options.joints)?],
            speed: u8::try_from(options.speed)?,
            tessellation: u8::try_from(options.tessellation)?,
            seed: (options.use_seed != 0).then_some(options.seed),
            ..Config::default()
        };
        if !options.texture_path.is_null() {
            // SAFETY: the wrapper supplies a live NUL-terminated filesystem path.
            let bytes = unsafe { CStr::from_ptr(options.texture_path) }.to_bytes();
            if !bytes.is_empty() {
                config.texture = Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)));
            }
        }
        let animation = Animation::new(&config, [width, height])?;
        let texture = Texture::load(config.texture.as_deref(), None)?;
        let process = PROCESS.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(shared) = slot.upgrade() {
                shared
            } else {
                let shared = Rc::new(RefCell::new(Process {
                    random: Random::new(config.seed),
                    fade: FadeClock::default(),
                    start: Instant::now(),
                    model_bytes: 0,
                    gpu_bytes: 0,
                }));
                *slot = Rc::downgrade(&shared);
                shared
            }
        });
        let model_bytes = Scene::model_bytes(animation.dims);
        let gpu_bytes = texture.as_ref().map_or_else(
            || crate::geometry::mesh_bytes(config.tessellation, false),
            |texture| u64::from(texture.size[0]) * u64::from(texture.size[1]) * 4,
        );
        {
            let shared = process.borrow();
            check_budget(
                shared.model_bytes + model_bytes,
                shared.gpu_bytes + gpu_bytes,
            )?;
        }
        // SAFETY: the caller supplies a live native context for this thread.
        let surface = Rc::new(unsafe { Cgl::retain(context)? });
        let renderer = Renderer::new(surface.as_ref(), config.tessellation, texture.as_ref())?;
        {
            let mut shared = process.borrow_mut();
            shared.model_bytes += model_bytes;
            shared.gpu_bytes += gpu_bytes;
        }
        let saver = Saver {
            history: None,
            renderer,
            surface,
            animation,
            config,
            process,
            model_bytes,
            gpu_bytes,
        };
        Ok(Box::into_raw(Box::new(saver)).cast())
    })
}

/// Render/present one host-scheduled frame. Zero reports a contained failure.
/// # Safety
/// `handle` must be the live unique engine returned by create, on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xp_pipes_frame(
    handle: *mut c_void,
    width: u32,
    height: u32,
    pixel_width: u32,
    pixel_height: u32,
) -> c_int {
    boundary(0, || {
        if handle.is_null() {
            return Err("missing saver engine".into());
        }
        // SAFETY: caller preserves the unique engine handle until destroy.
        let saver = unsafe { &mut *handle.cast::<Saver>() };
        saver.frame(
            [width, height],
            [i32::try_from(pixel_width)?, i32::try_from(pixel_height)?],
        )?;
        Ok(1)
    })
}

/// Release the engine before releasing the native view/context.
/// # Safety
/// `handle` is null or a live create result not previously destroyed; main thread only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xp_pipes_destroy(handle: *mut c_void) {
    boundary((), || {
        if !handle.is_null() {
            // SAFETY: exclusive ownership returns from the native wrapper exactly once.
            drop(unsafe { Box::from_raw(handle.cast::<Saver>()) });
        }
        Ok(())
    });
}
