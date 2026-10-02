//! Wayland presentation with per-output scenes and process-wide random state. Shell roles own their
//! `wl_surfaces`; records drop EGL drawables and scaling objects before those roles.
use crate::{
    Result,
    animation::Animation,
    config::Config,
    egl::{Drawable, Gpu},
    geometry::{self, Scene},
    renderer::{self, History, Renderer},
    rng::Random,
    texture::Texture,
    timing::FadeClock,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_dispatch2, delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    reexports::{calloop::EventLoop, calloop_wayland_source::WaylandSource},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState},
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Proxy, QueueHandle, WEnum,
    globals::registry_queue_init,
    protocol::{
        wl_callback, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface::WlSurface, wl_touch,
    },
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, wp_fractional_scale_v1,
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};

const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(10);
const MOTION_SETTLE: Duration = Duration::from_millis(300);

pub fn run(config: Config) -> Result<()> {
    let texture = Texture::load(config.texture.as_deref(), config.default_texture.as_deref())?;
    let connection = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<App>(&connection)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    // An overlay must not require xdg-shell: swaylock-plugin deliberately omits it.
    let shell = if config.windowed {
        Shell::Window(XdgShell::bind(&globals, &qh)?)
    } else {
        Shell::Overlay(LayerShell::bind(&globals, &qh)?)
    };
    let viewporter = globals
        .bind::<WpViewporter, _, _>(&qh, 1..=1, ProtocolData)
        .ok();
    let fractional = if viewporter.is_some() {
        globals
            .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ProtocolData)
            .ok()
    } else {
        None
    };
    let gpu = Gpu::new(connection.clone())?;
    let start = Instant::now();
    let mut app = App {
        records: Vec::new(),
        renderer: None,
        gpu,
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats: SeatState::new(&globals, &qh),
        devices: Vec::new(),
        compositor,
        shell,
        viewporter,
        fractional,
        random: Random::new(config.seed),
        fade: FadeClock::default(),
        texture,
        config,
        start,
        exit: false,
        drew_frame: false,
    };
    if app.config.windowed {
        app.add_surface(None, &qh);
    } else {
        let outputs = app.outputs.outputs().collect::<Vec<_>>();
        for output in outputs {
            app.add_surface(Some(output), &qh);
        }
    }
    let mut event_loop = EventLoop::<App>::try_new()?;
    WaylandSource::new(connection, queue).insert(event_loop.handle())?;
    while !app.exit {
        let now = Instant::now();
        if app
            .config
            .lifetime
            .is_some_and(|limit| now.duration_since(start) >= limit)
        {
            break;
        }
        app.paint(&qh)?;
        event_loop.dispatch(app.wait(now)?, &mut app)?;
    }
    Ok(())
}

enum Shell {
    Overlay(LayerShell),
    Window(XdgShell),
}
enum Role {
    Overlay(LayerSurface),
    Window(Window),
}
impl Role {
    fn surface(&self) -> &WlSurface {
        match self {
            Self::Overlay(role) => role.wl_surface(),
            Self::Window(role) => role.wl_surface(),
        }
    }
}

#[derive(Default)]
struct FrameState {
    serial: u64,
    ready: bool,
}
impl FrameState {
    fn configure(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        self.ready = true;
    }
    fn request(&mut self) -> u64 {
        debug_assert!(self.ready);
        self.serial = self.serial.wrapping_add(1);
        self.ready = false;
        self.serial
    }
    fn complete(&mut self, serial: u64) {
        if serial == self.serial {
            self.ready = true;
        }
    }
}

struct Surface {
    history: Option<History>,
    animation: Option<Animation>,
    drawable: Option<Rc<Drawable>>,
    scaling: Scaling,
    role: Role,
    output: Option<wl_output::WlOutput>,
    logical: [u32; 2],
    integer_scale: i32,
    fractional_scale: Option<u32>,
    configured: bool,
    frame: FrameState,
    dirty: bool,
    delayed_present: Option<Instant>,
    created: Instant,
}
struct Scaling {
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
}
impl Drop for Scaling {
    fn drop(&mut self) {
        if let Some(fractional) = &self.fractional {
            fractional.destroy();
        }
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
    }
}

impl Surface {
    fn configure(&mut self, size: [u32; 2]) {
        self.logical = size;
        self.configured = true;
        self.dirty = true;
        self.frame.configure();
        self.delayed_present = None;
    }

    fn buffer_size(&self) -> Result<[i32; 2]> {
        buffer_size(
            self.logical,
            self.fractional_scale.unwrap_or(
                u32::try_from(self.integer_scale.max(1))?
                    .checked_mul(120)
                    .ok_or("scale overflow")?,
            ),
        )
    }

    fn prepare(&mut self, gpu: &Rc<Gpu>) -> Result<[i32; 2]> {
        let size = self.buffer_size()?;
        if let Some(viewport) = &self.scaling.viewport {
            if self.role.surface().version() >= 3 {
                self.role.surface().set_buffer_scale(1);
            }
            viewport.set_destination(
                i32::try_from(self.logical[0])?,
                i32::try_from(self.logical[1])?,
            );
        } else if self.role.surface().version() >= 3 {
            self.role
                .surface()
                .set_buffer_scale(self.integer_scale.max(1));
        }
        if let Some(drawable) = &self.drawable {
            drawable.resize(size);
        } else {
            self.drawable = Some(Rc::new(gpu.drawable(self.role.surface(), size)?));
        }
        Ok(size)
    }
}

// Bounds checked before EGL allocation and before conversion to GL dimensions.
fn buffer_size(logical: [u32; 2], scale: u32) -> Result<[i32; 2]> {
    if scale == 0 {
        return Err("zero output scale".into());
    }
    let mut size = [0; 2];
    for (out, dimension) in size.iter_mut().zip(logical) {
        let pixels = (u64::from(dimension) * u64::from(scale)).div_ceil(120);
        if !(1..=16_384).contains(&pixels) {
            return Err("output buffer dimension outside 1..16384".into());
        }
        *out = i32::try_from(pixels)?;
    }
    if i64::from(size[0]) * i64::from(size[1]) > 64 * 1024 * 1024 {
        return Err("output buffer exceeds 64 megapixels".into());
    }
    Ok(size)
}

fn check_budgets(model: u64, gpu: u64) -> Result<()> {
    if model > 128 * 1024 * 1024 {
        return Err("combined output models exceed 128 MiB".into());
    }
    if gpu > 512 * 1024 * 1024 {
        return Err("combined histories/meshes/textures exceed 512 MiB".into());
    }
    Ok(())
}

struct App {
    // Field order matters: drawables/scaling/roles are retired before Gpu and
    // before global protocol objects; the event loop also retains Connection.
    records: Vec<Surface>,
    renderer: Option<Renderer>,
    gpu: Rc<Gpu>,
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    devices: Vec<Devices>,
    compositor: CompositorState,
    shell: Shell,
    viewporter: Option<WpViewporter>,
    fractional: Option<WpFractionalScaleManagerV1>,
    config: Config,
    random: Random,
    fade: FadeClock,
    texture: Option<Texture>,
    start: Instant,
    exit: bool,
    drew_frame: bool,
}

impl App {
    fn add_surface(&mut self, output: Option<wl_output::WlOutput>, qh: &QueueHandle<Self>) {
        if self.records.iter().any(|r| r.output == output) {
            return;
        }
        let surface = self.compositor.create_surface(qh);
        let scaling = Scaling {
            fractional: self
                .fractional
                .as_ref()
                .map(|m| m.get_fractional_scale(&surface, qh, FractionalData(surface.clone()))),
            viewport: self
                .viewporter
                .as_ref()
                .map(|v| v.get_viewport(&surface, qh, ProtocolData)),
        };
        let role = match &self.shell {
            Shell::Overlay(shell) => {
                let role = shell.create_layer_surface(
                    qh,
                    surface,
                    Layer::Overlay,
                    Some("wlpipes"),
                    output.as_ref(),
                );
                role.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
                role.set_exclusive_zone(-1);
                // Under swaylock-plugin there is no seat; this request is ignored
                // by the private compositor. On a desktop it prevents key leakage.
                role.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
                role.commit();
                Role::Overlay(role)
            }
            Shell::Window(shell) => {
                let role = shell.create_window(surface, WindowDecorations::RequestServer, qh);
                role.set_title("wlpipes");
                role.set_app_id("wlpipes");
                role.commit();
                Role::Window(role)
            }
        };
        let integer_scale = output
            .as_ref()
            .and_then(|o| self.outputs.info(o))
            .map_or(1, |o| o.scale_factor.max(1));
        self.records.push(Surface {
            history: None,
            animation: None,
            drawable: None,
            scaling,
            role,
            output,
            integer_scale,
            logical: [960, 720],
            fractional_scale: None,
            configured: false,
            frame: FrameState::default(),
            dirty: true,
            delayed_present: None,
            created: Instant::now(),
        });
    }

    fn surface(&mut self, surface: &WlSurface) -> Option<&mut Surface> {
        self.records
            .iter_mut()
            .find(|record| record.role.surface() == surface)
    }

    fn prepare_histories(&mut self) -> Result<()> {
        let mut model_bytes =
            u64::try_from(self.renderer.as_ref().map_or(0, Renderer::retained_bytes))?;
        let mut history_bytes = self.renderer.as_ref().map_or_else(
            || {
                self.texture
                    .as_ref()
                    .map_or(geometry::mesh_bytes(self.config.tessellation, false), |t| {
                        u64::from(t.size[0]) * u64::from(t.size[1]) * 4
                    })
            },
            Renderer::fixed_gpu_bytes,
        );
        let textured_mesh_bytes = if self.config.textured() {
            geometry::mesh_bytes(self.config.tessellation, true)
        } else {
            0
        };
        for record in self.records.iter_mut().filter(|r| r.configured) {
            let dims = geometry::layout(record.logical)?;
            let retained = u64::try_from(
                record
                    .animation
                    .as_ref()
                    .map_or(0, |a| a.scene.retained_bytes()),
            )?;
            model_bytes = model_bytes
                .checked_add(Scene::model_bytes(dims).max(retained))
                .ok_or("model size overflow")?;
            let size = record.buffer_size()?;
            history_bytes = history_bytes
                .checked_add(renderer::history_bytes(size)? + textured_mesh_bytes)
                .ok_or("history size overflow")?;
            // Retire ALL changed targets before allocating ANY replacements;
            // swapping a large/small display cannot temporarily double the budget.
            if record.history.as_ref().is_some_and(|h| h.size != size) {
                record.history = None;
            }
        }
        check_budgets(model_bytes, history_bytes)
    }

    fn paint(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        self.prepare_histories()?;
        for (index, record) in self.records.iter_mut().enumerate() {
            if !record.configured || !record.frame.ready {
                continue;
            }
            match &mut record.animation {
                None => record.animation = Some(Animation::new(&self.config, record.logical)?),
                Some(animation) if record.dirty => {
                    animation.resize(&self.config, record.logical)?;
                }
                Some(_) => {}
            }
            let now = Instant::now();
            if let Some(deadline) = record.delayed_present {
                if now < deadline {
                    continue;
                }
                // A rejected Render already happened: present its unchanged
                // history after the nonblocking equivalent of Sleep(10).
                record.delayed_present = None;
            } else {
                let accepted = record.animation.as_mut().unwrap().render(
                    now.duration_since(self.start).as_secs_f64(),
                    &mut self.random,
                    &mut self.fade,
                )?;
                if !accepted && index == 0 {
                    record.delayed_present = Some(now + Duration::from_millis(10));
                    continue;
                }
            }
            let size = record.prepare(&self.gpu)?;
            let animation = record.animation.as_ref().unwrap();
            let target = record
                .drawable
                .as_ref()
                .expect("prepare created the drawable");
            if self.renderer.is_none() {
                self.renderer = Some(Renderer::new(
                    target.as_ref(),
                    self.config.tessellation,
                    self.texture.as_ref(),
                )?);
                self.texture = None; // GPU owns the immutable image after upload.
            }
            self.renderer.as_mut().unwrap().draw(
                target.clone(),
                &mut record.history,
                size,
                &animation.scene,
            )?;
            let serial = record.frame.request();
            record.role.surface().frame(
                qh,
                PaintFrame {
                    surface: record.role.surface().clone(),
                    serial,
                },
            );
            target.swap()?;
            record.dirty = false;
            self.drew_frame = true;
        }
        Ok(())
    }

    fn wait(&self, now: Instant) -> Result<Option<Duration>> {
        let mut deadline = self.config.lifetime.map(|limit| self.start + limit);
        for record in self
            .records
            .iter()
            .filter(|r| r.configured && r.frame.ready)
        {
            let retry = record.delayed_present.unwrap_or(now);
            deadline = Some(deadline.map_or(retry, |d| d.min(retry)));
        }
        let configure_deadline = self
            .records
            .iter()
            .filter(|r| !r.configured)
            .map(|r| r.created + CONFIGURE_TIMEOUT)
            .min()
            .or_else(|| (!self.drew_frame).then_some(self.start + CONFIGURE_TIMEOUT));
        if let Some(configure) = configure_deadline {
            if now >= configure {
                return Err("compositor did not configure a drawable within 10 seconds".into());
            }
            deadline = Some(deadline.map_or(configure, |d| d.min(configure)));
        }
        Ok(deadline.map(|d| d.saturating_duration_since(Instant::now())))
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}
delegate_registry!(App);
delegate_dispatch2!(App);

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &WlSurface,
        factor: i32,
    ) {
        if let Some(record) = self.surface(surface) {
            record.integer_scale = factor.max(1);
            record.dirty = true;
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
        // Buffers stay in the surface's logical orientation (Normal). The
        // compositor applies output transforms; rotating here would double it.
    }
    // Actual requests use PaintFrame tokens, not the unversioned SCTK callback.
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}
impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if !self.config.windowed {
            self.add_surface(Some(output), qh);
        }
    }
    fn update_output(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let factor = self
            .outputs
            .info(&output)
            .map_or(1, |o| o.scale_factor.max(1));
        for record in &mut self.records {
            if record.output.as_ref() == Some(&output) {
                record.integer_scale = factor;
                record.dirty = true;
            }
        }
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.records.retain(|r| r.output.as_ref() != Some(&output));
    }
}
impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.records
            .retain(|r| r.role.surface() != layer.wl_surface());
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        cfg: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(record) = self.surface(layer.wl_surface()) {
            record.configure([cfg.new_size.0, cfg.new_size.1]);
        }
    }
}
impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        window: &Window,
        cfg: WindowConfigure,
        _: u32,
    ) {
        if let Some(record) = self.surface(window.wl_surface()) {
            record.configure([
                cfg.new_size
                    .0
                    .map_or(record.logical[0], std::num::NonZeroU32::get),
                cfg.new_size
                    .1
                    .map_or(record.logical[1], std::num::NonZeroU32::get),
            ]);
        }
    }
}

struct PaintFrame {
    surface: WlSurface,
    serial: u64,
}
impl Dispatch2<wl_callback::WlCallback, App> for PaintFrame {
    fn event(
        &self,
        app: &mut App,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wl_callback::Event::Done { .. } = event
            && let Some(record) = app.surface(&self.surface)
        {
            record.frame.complete(self.serial);
        }
    }
}

struct ProtocolData;
struct FractionalData(WlSurface);
impl Dispatch2<WpViewporter, App> for ProtocolData {
    fn event(
        &self,
        _: &mut App,
        _: &WpViewporter,
        _: <WpViewporter as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}
impl Dispatch2<WpViewport, App> for ProtocolData {
    fn event(
        &self,
        _: &mut App,
        _: &WpViewport,
        _: <WpViewport as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}
impl Dispatch2<WpFractionalScaleManagerV1, App> for ProtocolData {
    fn event(
        &self,
        _: &mut App,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}
impl Dispatch2<wp_fractional_scale_v1::WpFractionalScaleV1, App> for FractionalData {
    fn event(
        &self,
        app: &mut App,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
            && let Some(record) = app.surface(&self.0)
        {
            record.fractional_scale = Some(scale);
            record.dirty = true;
        }
    }
}

struct Devices {
    seat: wl_seat::WlSeat,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
}
impl Devices {
    fn remove(&mut self, capability: Capability) {
        match capability {
            Capability::Keyboard => {
                if let Some(device) = self.keyboard.take()
                    && device.version() >= 3
                {
                    device.release();
                }
            }
            Capability::Pointer => {
                if let Some(device) = self.pointer.take()
                    && device.version() >= 3
                {
                    device.release();
                }
            }
            Capability::Touch => {
                if let Some(device) = self.touch.take()
                    && device.version() >= 3
                {
                    device.release();
                }
            }
            _ => {}
        }
    }
}
impl Drop for Devices {
    fn drop(&mut self) {
        for cap in [Capability::Keyboard, Capability::Pointer, Capability::Touch] {
            self.remove(cap);
        }
    }
}
impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if self.config.windowed {
            return;
        }
        // Initial globals receive capability events without new_seat; create
        // device state here for both startup seats and later hotplugged seats.
        let index = self
            .devices
            .iter()
            .position(|d| d.seat == seat)
            .unwrap_or_else(|| {
                self.devices.push(Devices {
                    seat: seat.clone(),
                    keyboard: None,
                    pointer: None,
                    touch: None,
                });
                self.devices.len() - 1
            });
        let devices = &mut self.devices[index];
        match capability {
            Capability::Keyboard if devices.keyboard.is_none() => {
                devices.keyboard = Some(seat.get_keyboard(qh, ProtocolData));
            }
            Capability::Pointer if devices.pointer.is_none() => {
                devices.pointer = Some(seat.get_pointer(qh, ProtocolData));
            }
            Capability::Touch if devices.touch.is_none() => {
                devices.touch = Some(seat.get_touch(qh, ProtocolData));
            }
            _ => {}
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if let Some(devices) = self.devices.iter_mut().find(|d| d.seat == seat) {
            devices.remove(capability);
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.devices.retain(|d| d.seat != seat);
    }
}
impl Dispatch2<wl_keyboard::WlKeyboard, App> for ProtocolData {
    fn event(
        &self,
        app: &mut App,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if matches!(
            event,
            wl_keyboard::Event::Key {
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            }
        ) {
            app.exit = true;
        }
        // Keymap events own an OwnedFd, which is closed on drop without parsing.
    }
}
impl Dispatch2<wl_pointer::WlPointer, App> for ProtocolData {
    fn event(
        &self,
        app: &mut App,
        pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            wl_pointer::Event::Enter { serial, .. } => pointer.set_cursor(serial, None, 0, 0),
            wl_pointer::Event::Motion { .. } if app.start.elapsed() >= MOTION_SETTLE => {
                app.exit = true;
            }
            wl_pointer::Event::Button { .. } | wl_pointer::Event::Axis { .. } => app.exit = true,
            _ => {}
        }
    }
}
impl Dispatch2<wl_touch::WlTouch, App> for ProtocolData {
    fn event(
        &self,
        app: &mut App,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if matches!(
            event,
            wl_touch::Event::Down { .. } | wl_touch::Event::Motion { .. }
        ) {
            app.exit = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_output_script_replays_shared_random_fade_and_resize_inputs() {
        let cfg = Config {
            pipes: crate::config::Pipes::Multiple,
            joints: crate::config::Joints::Cycle,
            texture: Some("diagnostic.png".into()),
            seed: Some(42),
            ..Config::default()
        };
        let mut a = [[640, 480], [5120, 2160]].map(|size| Animation::new(&cfg, size).unwrap());
        let mut b = [[640, 480], [5120, 2160]].map(|size| Animation::new(&cfg, size).unwrap());
        let (mut ra, mut rb) = (Random::new(cfg.seed), Random::new(cfg.seed));
        let (mut fa, mut fb) = (FadeClock::default(), FadeClock::default());
        for frame in 0..4000 {
            if frame == 200 {
                a[0].resize(&cfg, [480, 640]).unwrap();
                b[0].resize(&cfg, [480, 640]).unwrap();
            }
            for output in 0..2 {
                if output == 1 && frame % 5 == 0 {
                    continue;
                }
                let now = f64::from(frame) * 0.025;
                assert_eq!(
                    a[output].render(now, &mut ra, &mut fa).unwrap(),
                    b[output].render(now, &mut rb, &mut fb).unwrap()
                );
                assert_eq!(ra.checkpoint(), rb.checkpoint());
                assert_eq!(
                    (a[output].scene.generation, a[output].scene.fades),
                    (b[output].scene.generation, b[output].scene.fades)
                );
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&a[output].scene.instances),
                    bytemuck::cast_slice::<_, u8>(&b[output].scene.instances)
                );
            }
        }
        assert!(a.iter().all(|a| a.scene.generation > 2));
    }

    #[test]
    fn stale_frame_callbacks_cannot_release_a_new_configure_generation() {
        let mut frame = FrameState::default();
        frame.configure();
        let old = frame.request();
        assert!(!frame.ready);
        frame.configure();
        let current = frame.request();
        frame.complete(old);
        assert!(!frame.ready, "stale pre-resize callback bypassed pacing");
        frame.complete(current);
        assert!(frame.ready);
        let next = frame.request();
        frame.complete(current);
        assert!(!frame.ready);
        frame.complete(next);
        assert!(frame.ready);
    }

    #[test]
    fn render_admission_never_replays_a_clock_backlog() {
        let config = Config {
            seed: Some(42),
            ..Config::default()
        };
        let mut animation = Animation::new(&config, [1920, 1080]).unwrap();
        let mut random = Random::new(config.seed);
        let mut fade = FadeClock::default();
        let initial = random.checkpoint();
        assert!(!animation.render(0.0, &mut random, &mut fade).unwrap());
        assert!(!animation.render(0.020, &mut random, &mut fade).unwrap());
        assert_eq!(random.checkpoint(), initial);
        assert!(
            animation
                .render(0.020_000_01, &mut random, &mut fade)
                .unwrap()
        );
        let count = animation.scene.instances.len();
        assert!(animation.render(100.0, &mut random, &mut fade).unwrap());
        assert!(animation.scene.instances.len() <= count + 2);
        let checkpoint = random.checkpoint();
        assert!(!animation.render(100.0, &mut random, &mut fade).unwrap());
        assert_eq!(random.checkpoint(), checkpoint);
    }

    #[test]
    fn completion_retains_geometry_until_the_terminal_fade_clear() {
        let config = Config {
            speed: 20,
            seed: Some(42),
            ..Config::default()
        };
        let mut animation = Animation::new(&config, [1920, 1080]).unwrap();
        let mut random = Random::new(config.seed);
        let mut fade = FadeClock::default();
        let mut now = 0.0;
        for _ in 0..10000 {
            animation.render(now, &mut random, &mut fade).unwrap();
            now += 0.020;
            if animation.simulation.ended() {
                break;
            }
        }
        assert!(animation.simulation.ended());
        let checkpoint = random.checkpoint();
        let count = animation.scene.instances.len();
        animation.render(now, &mut random, &mut fade).unwrap();
        for pass in 1..=29 {
            now += 0.020;
            animation.render(now, &mut random, &mut fade).unwrap();
            assert_eq!(animation.scene.fades, pass);
            assert_eq!(animation.scene.instances.len(), count);
            assert_eq!(animation.scene.generation, 0);
            assert_eq!(random.checkpoint(), checkpoint);
        }
        now += 0.020;
        animation.render(now, &mut random, &mut fade).unwrap();
        assert_eq!(animation.scene.generation, 1);
        assert_eq!(animation.scene.fades, 0);
        assert!(!animation.scene.instances.is_empty());
    }

    #[test]
    fn resizing_preserves_scale_only_growth_and_resets_changed_layout_once() {
        let config = Config {
            speed: 20,
            seed: Some(42),
            ..Config::default()
        };
        let mut animation = Animation::new(&config, [1280, 540]).unwrap();
        let mut random = Random::new(config.seed);
        let mut fade = FadeClock::default();
        animation.render(0.0, &mut random, &mut fade).unwrap();
        let first = bytemuck::cast_slice::<_, u8>(&animation.scene.instances).to_vec();
        animation.resize(&config, [2560, 1080]).unwrap();
        assert_eq!(animation.scene.generation, 0);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&animation.scene.instances),
            first
        );
        let checkpoint = random.checkpoint();
        animation.resize(&config, [540, 1280]).unwrap();
        assert_eq!(random.checkpoint(), checkpoint);
        assert!(animation.scene.instances.is_empty());
        assert_eq!(animation.scene.generation, 1);
        assert_eq!(animation.dims, [6, 15, 15]);
        animation.render(1.0, &mut random, &mut fade).unwrap();
        assert!((2..=4).contains(&animation.scene.instances.len()));
        animation.resize(&config, [540, 1280]).unwrap();
        assert_eq!(animation.scene.generation, 1);
    }

    #[test]
    fn outputs_own_scenes_and_share_process_randomness() {
        let config = Config {
            speed: 20,
            seed: Some(42),
            ..Config::default()
        };
        let mut a = Animation::new(&config, [1280, 800]).unwrap();
        let mut b = Animation::new(&config, [1280, 800]).unwrap();
        let mut random = Random::new(config.seed);
        let mut fade = FadeClock::default();
        a.render(0.0, &mut random, &mut fade).unwrap();
        let first = bytemuck::cast_slice::<_, u8>(&a.scene.instances).to_vec();
        b.render(0.0, &mut random, &mut fade).unwrap();
        assert_eq!(bytemuck::cast_slice::<_, u8>(&a.scene.instances), first);
        assert_ne!(bytemuck::cast_slice::<_, u8>(&b.scene.instances), first);
        drop(a);
        let count = b.scene.instances.len();
        assert!(b.render(0.0, &mut random, &mut fade).unwrap());
        assert!(b.scene.instances.len() > count);
    }

    #[test]
    fn aggregate_budget_includes_each_target_not_only_each_valid_dimension() {
        let large = renderer::history_bytes([7680, 4320]).unwrap();
        assert!(check_budgets(0, large * 3).is_err());
        assert!(check_budgets(128 * 1024 * 1024 + 1, 0).is_err());
        let scout = renderer::history_bytes([5120, 2160]).unwrap()
            + renderer::history_bytes([2880, 1800]).unwrap()
            + 2 * geometry::mesh_bytes(4, true)
            + 64 * 64 * 4;
        assert!(check_budgets(4 * 1024 * 1024, scout).is_ok());
    }

    #[test]
    fn scale_arithmetic_is_bounded_and_rounds_up() {
        assert_eq!(buffer_size([1920, 1080], 120).unwrap(), [1920, 1080]);
        assert_eq!(buffer_size([1001, 701], 150).unwrap(), [1252, 877]);
        assert_eq!(buffer_size([100, 100], 240).unwrap(), [200, 200]);
        for (size, scale) in [
            ([0, 10], 120),
            ([u32::MAX, 1], u32::MAX),
            ([16_384, 16_384], 120),
            ([1, 1], 0),
        ] {
            assert!(buffer_size(size, scale).is_err());
        }
    }
}
