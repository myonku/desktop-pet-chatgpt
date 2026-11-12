mod behavior;
mod config;
mod launch;

use std::{
    error::Error,
    io,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    platform::windows::WindowAttributesExtWindows,
    window::{Window, WindowId, WindowLevel},
};

use crate::{
    core::{Pet, initial_side_dip},
    media::{
        Variant,
        playback::{PageResult, Playback},
    },
    platform::windows::{Display, Gesture, WindowInput, physical_size},
    render::{Renderer, placeholder::Placeholder},
    ui::Preview,
};

pub(crate) enum AppEvent {
    TrayMenu(PhysicalPosition<i32>),
    TaskbarCreated,
    ResumeDisplay,
    Gesture(Gesture),
    PetDeviceLost,
    PackReady(io::Result<Box<Playback>>),
    PageReady(PageResult),
}

struct App {
    options: launch::LaunchOptions,
    scene: Option<Scene>,
    preview: Option<Preview>,
    proxy: EventLoopProxy<AppEvent>,
    error: Option<Box<dyn Error>>,
}

struct Scene {
    input: WindowInput,
    renderer: Option<Renderer>,
    window: Arc<Window>,
    sample: Placeholder,
    playback: Option<Playback>,
    wants_bc7: bool,
    passthrough: bool,
    visible: bool,
    minimized: bool,
    pet: Pet,
    display: Option<Display>,
    screen_poll: Instant,
    side_dip: f64,
    applied_size: Option<PhysicalSize<u32>>,
    reference_geometry: bool,
    unavailable: std::collections::BTreeSet<String>,
    surface_retried: bool,
    device_rebuilt: bool,
    first_asset_presented: bool,
    trace: Option<PlaybackTrace>,
}

struct PlaybackTrace {
    start: Instant,
    pending: Option<(u128, usize, u128)>,
    action: String,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let options = launch::LaunchOptions::load()?;
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let mut app = App {
        options,
        scene: None,
        preview: None,
        proxy: event_loop.create_proxy(),
        error: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

impl Scene {
    fn open(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<AppEvent>,
        options: &launch::LaunchOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let scale = options.startup.scale_percent;
        let attributes = Window::default_attributes()
            .with_title("Desktop Pet ChatGPT")
            .with_inner_size(LogicalSize::new(256.0, 256.0))
            .with_decorations(false)
            .with_resizable(false)
            .with_active(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_transparent(true)
            .with_no_redirection_bitmap(true);
        #[cfg(feature = "p1b-reference")]
        let (attributes, geometry) = launch::reference_client_geometry(attributes)?;
        #[cfg(not(feature = "p1b-reference"))]
        let geometry = launch::StartupGeometry::Product;
        let window = Arc::new(event_loop.create_window(attributes)?);
        let sample = Placeholder::load()?;
        let mut input = WindowInput::new(&window, proxy.clone(), options.trace_input)?;
        let display = input.display();
        let probe_until = Instant::now() + Duration::from_millis(120);
        let probe = display
            .filter(|_| geometry.probe_edid())
            .and_then(|display| {
                let (sender, receiver) = mpsc::sync_channel(1);
                thread::Builder::new()
                    .name("monitor-edid".into())
                    .spawn(move || {
                        let _ = sender.send(physical_size(display.device));
                    })
                    .ok()?;
                Some(receiver)
            });
        let physical_mm = probe.and_then(|probe| {
            probe
                .recv_timeout(probe_until.saturating_duration_since(Instant::now()))
                .ok()
                .flatten()
        });
        let side_dip = initial_side_dip(
            display.map_or(0, |display| display.bounds.max[1] - display.bounds.min[1]),
            physical_mm.map(|size| size[1]),
            window.scale_factor(),
            scale,
        );
        let applied_size = if geometry.fixed() {
            None
        } else {
            let size = Self::target_size(&window, display, side_dip);
            let _ = window.request_inner_size(size);
            Some(size)
        };
        let mut pet = Pet::new(Instant::now());
        pet.fullscreen = display.is_some_and(|display| input.fullscreen(display));
        eprintln!(
            "Window: DPI scale {:.2}, client size {:?}, EDID mm {physical_mm:?}, initial DIP {side_dip:.2}",
            window.scale_factor(),
            window.inner_size()
        );
        input.apply_hit_region(sample.view())?;
        let variant = options.variant;
        let root = options.startup.pack_path.clone();
        let wants_bc7 = root.is_some() && variant == Variant::Bc7;
        let renderer = pollster::block_on(Renderer::new(
            Arc::clone(&window),
            sample.view(),
            wants_bc7,
            proxy.clone(),
        ))?;
        window.set_visible(pet.visible());
        window.request_redraw();
        if let Some(root) = root {
            let spawn = thread::Builder::new()
                .name("asset-pack".into())
                .spawn(move || {
                    let frames = proxy.clone();
                    let pack = Playback::open(&root, variant, move |page| {
                        frames.send_event(AppEvent::PageReady(page)).is_ok()
                    })
                    .map(Box::new);
                    let _ = proxy.send_event(AppEvent::PackReady(pack));
                });
            if let Err(error) = spawn {
                eprintln!("Could not open asset pack; using placeholder: {error}");
            }
        }
        Ok(Self {
            input,
            renderer: Some(renderer),
            window,
            sample,
            playback: None,
            wants_bc7,
            passthrough: false,
            visible: pet.visible(),
            minimized: false,
            pet,
            display,
            screen_poll: Instant::now(),
            side_dip,
            applied_size,
            reference_geometry: geometry.fixed(),
            unavailable: Default::default(),
            surface_retried: false,
            device_rebuilt: false,
            first_asset_presented: false,
            trace: options.trace_playback.then(|| PlaybackTrace {
                start: Instant::now(),
                pending: None,
                action: String::new(),
            }),
        })
    }

    fn set_passthrough(&mut self, enabled: bool) {
        if self.passthrough == enabled || (enabled && !self.input.has_tray()) {
            return;
        }
        if enabled {
            self.handle_gesture(Gesture::Cancel);
            self.input.cancel_gesture();
        }
        match self.window.set_cursor_hittest(!enabled) {
            Ok(()) => {
                self.passthrough = enabled;
                self.window.request_redraw();
            }
            Err(error) => eprintln!("Could not change mouse passthrough: {error}"),
        }
    }

    fn apply_visibility(&mut self, now: Instant) {
        let visible = self.pet.visible();
        if self.visible == visible {
            return;
        }
        self.input.cancel_gesture();
        self.pet.reset(now);
        self.play_base();
        self.visible = visible;
        if let Some(trace) = &mut self.trace {
            trace.pending = None;
            eprintln!(
                "P1b visibility: {visible} {}",
                trace.start.elapsed().as_micros()
            );
        }
        self.window.set_visible(visible);
        if visible {
            self.window.request_redraw();
        }
    }

    fn refresh_frame(&mut self) -> Result<(), String> {
        let (key, frame) = self
            .playback
            .as_ref()
            .and_then(Playback::frame)
            .map_or((None, self.sample.view()), |(key, frame)| {
                (Some(key), frame)
            });
        let started = self.trace.as_ref().map(|_| Instant::now());
        if let Some(renderer) = &mut self.renderer {
            renderer.show(key, frame)?;
        }
        if let (Some(trace), Some(started)) = (&mut self.trace, started) {
            let prepare_us = started.elapsed().as_micros();
            if let Some(playback) = &self.playback
                && playback.sequence().is_some()
                && playback.active_id() != trace.action
            {
                trace.action = playback.active_id().to_owned();
                eprintln!(
                    "P2 action: {} {}",
                    trace.action,
                    trace.start.elapsed().as_micros()
                );
            }
            trace.pending = self
                .playback
                .as_ref()
                .and_then(Playback::sequence)
                .zip(key)
                .map(|(sequence, page)| (sequence, page, prepare_us));
        }
        self.input.invalidate_hit_region();
        self.input.apply_hit_region(frame)?;
        self.window.request_redraw();
        Ok(())
    }

    fn fallback(&mut self, error: impl std::fmt::Display) -> Result<(), String> {
        eprintln!("Asset playback stopped; using placeholder: {error}");
        self.playback = None;
        if self.pet.activity != crate::core::Activity::Drag {
            self.pet.finish(Instant::now());
        }
        self.refresh_frame()
    }

    fn redraw(&mut self) -> Result<(), wgpu::SurfaceError> {
        if !self.visible || self.window.is_minimized() == Some(true) {
            return Ok(());
        }
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        match renderer.draw(size.width, size.height) {
            Ok(()) => {
                self.surface_retried = false;
                if let Some(trace) = &mut self.trace
                    && let Some((sequence, page, prepare_us)) = trace.pending.take()
                {
                    eprintln!(
                        "P1b frame {sequence} {page} {} {prepare_us}",
                        trace.start.elapsed().as_micros()
                    );
                }
                if !self.first_asset_presented
                    && self.playback.as_ref().and_then(Playback::frame).is_some()
                {
                    self.first_asset_presented = true;
                    eprintln!(
                        "First asset frame presented: {}x{}",
                        size.width, size.height
                    );
                }
            }
            Err(
                wgpu::SurfaceError::Lost
                | wgpu::SurfaceError::Outdated
                | wgpu::SurfaceError::Timeout,
            ) if !self.surface_retried => {
                self.surface_retried = true;
                renderer.invalidate_surface();
                self.window.request_redraw();
            }
            Err(wgpu::SurfaceError::Timeout) => {}
            Err(error) => return Err(error),
        }
        Ok(())
    }
}

impl App {
    fn open_preview(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(preview) = &self.preview {
            preview.window.set_minimized(false);
            preview.window.focus_window();
            preview.window.request_redraw();
            return;
        }
        match Preview::open(event_loop) {
            Ok(preview) => self.preview = Some(preview),
            Err(error) => eprintln!("Could not open Chinese preview: {error}"),
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.scene.is_none() {
            match Scene::open(event_loop, self.proxy.clone(), &self.options) {
                Ok(scene) => self.scene = Some(scene),
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                }
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        let Some(scene) = &mut self.scene else {
            return;
        };
        match event {
            AppEvent::TrayMenu(position) => {
                if scene.menu(event_loop, Some(position)) {
                    self.open_preview(event_loop);
                }
            }
            AppEvent::TaskbarCreated => {
                scene.input.add_tray();
                if !scene.input.has_tray() {
                    scene.set_passthrough(false);
                    scene.pet.manual_hidden = false;
                    scene.apply_visibility(Instant::now());
                }
            }
            AppEvent::ResumeDisplay => {
                scene.surface_retried = false;
                scene.input.cancel_gesture();
                scene.pet.reset(Instant::now());
                scene.play_base();
                scene.update_display(Instant::now());
                if scene.visible {
                    scene.window.request_redraw();
                }
                if let Some(preview) = &self.preview {
                    preview.window.request_redraw();
                }
            }
            AppEvent::Gesture(gesture) => scene.handle_gesture(gesture),
            AppEvent::PageReady(result) => {
                if let Some(playback) = &mut scene.playback {
                    playback.accept(result);
                }
            }
            AppEvent::PackReady(result) => match result {
                Ok(mut playback) => {
                    if scene
                        .renderer
                        .as_ref()
                        .is_some_and(|renderer| !renderer.supports_bc7())
                    {
                        playback.use_compatible_format();
                    }
                    scene.playback = Some(*playback);
                    scene.play_base();
                }
                Err(error) => eprintln!("Could not open asset pack; using placeholder: {error}"),
            },
            AppEvent::PetDeviceLost => {
                if scene.device_rebuilt {
                    self.error = Some("Pet GPU device was lost again after recovery".into());
                    event_loop.exit();
                    return;
                }
                scene.device_rebuilt = true;
                drop(scene.renderer.take());
                match pollster::block_on(Renderer::new(
                    scene.window.clone(),
                    scene.sample.view(),
                    scene.wants_bc7,
                    self.proxy.clone(),
                )) {
                    Ok(renderer) => {
                        if let Some(playback) = &mut scene.playback
                            && !renderer.supports_bc7()
                        {
                            playback.use_compatible_format();
                        }
                        scene.renderer = Some(renderer);
                        scene.pet.reset(Instant::now());
                        scene.play_base();
                        scene.surface_retried = false;
                        if scene.visible
                            && let Err(error) =
                                scene.refresh_frame().or_else(|error| scene.fallback(error))
                        {
                            self.error = Some(error.into());
                            event_loop.exit();
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        event_loop.exit();
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(preview) = &mut self.preview
            && preview.window.id() == id
        {
            if matches!(event, WindowEvent::CloseRequested) {
                self.preview = None;
            } else {
                preview.event(&event);
            }
            return;
        }
        let Some(scene) = &mut self.scene else {
            return;
        };
        if scene.window.id() != id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Moved(_) => {
                // Coalesce geometry and foreground checks into the event-loop deadline.
                scene.screen_poll = Instant::now();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                scene.screen_poll = Instant::now();
                let frame = scene
                    .playback
                    .as_ref()
                    .and_then(Playback::frame)
                    .map_or(scene.sample.view(), |(_, frame)| frame);
                if scene.visible
                    && let Err(error) = scene.input.apply_hit_region(frame)
                {
                    self.error = Some(error.into());
                    event_loop.exit();
                }
                scene.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = scene.redraw() {
                    self.error = Some(error.into());
                    event_loop.exit();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Right,
                ..
            } if !scene.passthrough && scene.menu(event_loop, None) => {
                self.open_preview(event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let preview = self.preview.as_mut().and_then(Preview::repaint_deadline);
        let pet = match self
            .scene
            .as_mut()
            .map(Scene::playback_deadline)
            .transpose()
        {
            Ok(deadline) => deadline.flatten(),
            Err(error) => {
                self.error = Some(error.into());
                event_loop.exit();
                return;
            }
        };
        let deadline = preview.into_iter().chain(pet).min();
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
