mod assets;
mod behavior;
mod config;
mod launch;
mod migration;
mod settings;

use std::{
    error::Error,
    path::PathBuf,
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
    core::{Pet, initial_side_dip, preferences::INITIAL_SCALE},
    media::{
        Variant,
        playback::{PageResult, Playback},
    },
    platform::windows::{
        Autostart, Display, Gesture, Instance, Sound, WindowInput, physical_size, user_profile,
    },
    render::{Renderer, placeholder::Placeholder},
    ui::SettingsWindow,
};

use self::{
    assets::{CharacterAssets, LoadedPack},
    settings::RuntimeSettings,
};

#[cfg(feature = "window-diagnostics")]
use crate::ui::InputPreview;

pub(crate) enum AppEvent {
    TrayMenu(PhysicalPosition<i32>),
    TaskbarCreated,
    ResumeDisplay,
    Gesture(Gesture),
    PetDeviceLost,
    OpenSettings,
    PageReady(PageResult),
}

struct App {
    options: launch::LaunchOptions,
    scene: Option<Scene>,
    #[cfg(feature = "window-diagnostics")]
    preview: Option<InputPreview>,
    settings: Option<SettingsWindow>,
    committed: Option<RuntimeSettings>,
    autostart: Option<Autostart>,
    instance: Instance,
    proxy: EventLoopProxy<AppEvent>,
    error: Option<Box<dyn Error>>,
}

struct Scene {
    input: WindowInput,
    renderer: Option<Renderer>,
    window: Arc<Window>,
    sample: Placeholder,
    playback: Option<Playback>,
    pack_path: PathBuf,
    pack_status: String,
    assets: CharacterAssets,
    sound: Sound,
    runtime: RuntimeSettings,
    wants_bc7: bool,
    passthrough: bool,
    visible: bool,
    minimized: bool,
    pet: Pet,
    display: Option<Display>,
    screen_poll: Instant,
    applied_size: Option<PhysicalSize<u32>>,
    fixed_geometry: bool,
    probe_edid: bool,
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
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !args.is_empty() {
        return migration::run(&args);
    }
    let (root, login_registration) = if cfg!(feature = "development") {
        (std::env::current_dir()?.canonicalize()?, false)
    } else {
        let profile = user_profile()?;
        (profile.root, profile.login_registration)
    };
    let Some(instance) = Instance::acquire(&root)? else {
        return Ok(());
    };
    let options = launch::LaunchOptions::load(root)?;
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let mut app = App {
        options,
        scene: None,
        #[cfg(feature = "window-diagnostics")]
        preview: None,
        settings: None,
        committed: None,
        autostart: login_registration.then(Autostart::default),
        instance,
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
        instance: &Instance,
    ) -> Result<Self, Box<dyn Error>> {
        let config = &options.startup.config;
        let scale = config.initial_scale_percent.unwrap_or(INITIAL_SCALE);
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
        let (attributes, geometry) = launch::window_geometry(attributes)?;
        let window = Arc::new(event_loop.create_window(attributes)?);
        let sample = Placeholder::load()?;
        let wants_bc7 = options.variant == Variant::Bc7;
        let mut renderer = pollster::block_on(Renderer::new(
            Arc::clone(&window),
            sample.view(),
            wants_bc7,
            proxy.clone(),
        ))?;
        let variant = if wants_bc7 && !renderer.supports_bc7() {
            eprintln!("BC7 unavailable on this device; using RGBA8");
            Variant::Rgba8
        } else {
            options.variant
        };
        let (pack_path, pack, pack_status) = assets::load_startup_pack(
            options.startup.pack_path.as_deref(),
            options.startup.default_pack()?,
            variant,
            &proxy,
            &mut renderer,
        )?;
        if options.trace_playback {
            pack.trace_resources();
        }
        let LoadedPack {
            playback,
            assets,
            click_wave,
        } = pack;
        let input = WindowInput::new(
            &window,
            proxy.clone(),
            options.trace_input,
            instance.marker(),
        )?;
        let display = input.display();
        let physical_mm = if config.saved_size.is_none() && geometry.probe_edid() {
            probe_dimensions(display)
        } else {
            None
        };
        let runtime = RuntimeSettings {
            preferences: config.preferences,
            size: config
                .saved_size
                .unwrap_or_else(|| crate::core::preferences::SavedSize {
                    base_dip: base_dip(display, physical_mm, window.scale_factor()),
                    percent: scale,
                }),
        };
        let side_dip = runtime.size.dip();
        let applied_size = if geometry.fixed() {
            None
        } else {
            let size = Self::target_size(&window, display, side_dip);
            let _ = window.request_inner_size(size);
            Some(size)
        };
        let mut pet = Pet::new(Instant::now());
        pet.auto_enabled = runtime.preferences.auto_move;
        pet.manual_hidden = runtime.preferences.start_hidden && input.has_tray();
        pet.fullscreen = runtime.preferences.fullscreen_hide
            && input.has_tray()
            && display.is_some_and(|display| input.fullscreen(display));
        let passthrough = runtime.preferences.passthrough
            && input.has_tray()
            && match window.set_cursor_hittest(false) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("Could not restore mouse passthrough: {error}");
                    false
                }
            };
        eprintln!(
            "Window: DPI scale {:.2}, client size {:?}, EDID mm {physical_mm:?}, initial DIP {side_dip:.2}",
            window.scale_factor(),
            window.inner_size()
        );
        let mut sound = Sound::default();
        sound.replace(click_wave);
        let mut scene = Self {
            input,
            renderer: Some(renderer),
            window,
            sample,
            playback: Some(playback),
            pack_path,
            pack_status,
            assets,
            sound,
            runtime,
            wants_bc7,
            passthrough,
            visible: pet.visible(),
            minimized: false,
            pet,
            display,
            screen_poll: Instant::now(),
            applied_size,
            fixed_geometry: geometry.fixed(),
            probe_edid: geometry.probe_edid(),
            unavailable: Default::default(),
            surface_retried: false,
            device_rebuilt: false,
            first_asset_presented: false,
            trace: options.trace_playback.then(|| PlaybackTrace {
                start: Instant::now(),
                pending: None,
                action: String::new(),
            }),
        };
        scene.refresh_frame()?;
        scene.window.set_visible(scene.visible);
        Ok(scene)
    }

    fn set_passthrough(&mut self, enabled: bool) -> Result<(), String> {
        if self.passthrough == enabled {
            return Ok(());
        }
        if enabled {
            if !self.input.has_tray() {
                return Err("托盘不可用，无法开启鼠标穿透。".into());
            }
            self.handle_gesture(Gesture::Cancel);
            self.input.cancel_gesture();
        }
        self.window
            .set_cursor_hittest(!enabled)
            .map_err(|error| error.to_string())?;
        self.passthrough = enabled;
        self.window.request_redraw();
        Ok(())
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
        if !visible {
            self.sound.stop();
        }
        if let Some(trace) = &mut self.trace {
            trace.pending = None;
            eprintln!(
                "Window visibility: {visible} {}",
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
                    "Playback action: {} {}",
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
        self.pack_status = format!("素材播放失败，正在使用内置占位图。{error}");
        self.sound.replace(None);
        self.assets = CharacterAssets::default();
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
                        "Playback frame {sequence} {page} {} {prepare_us}",
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
    #[cfg(feature = "window-diagnostics")]
    fn open_preview(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(preview) = &self.preview {
            preview.window().set_minimized(false);
            preview.window().focus_window();
            preview.window().request_redraw();
            return;
        }
        match InputPreview::open(event_loop) {
            Ok(preview) => self.preview = Some(preview),
            Err(error) => eprintln!("Could not open Chinese preview: {error}"),
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.scene.is_none() {
            match Scene::open(
                event_loop,
                self.proxy.clone(),
                &self.options,
                &self.instance,
            ) {
                Ok(scene) => {
                    let no_tray = !scene.input.has_tray();
                    self.committed = Some(scene.runtime);
                    self.scene = Some(scene);
                    if no_tray {
                        self.open_settings(
                            event_loop,
                            Some("系统托盘不可用。请从此窗口或角色右键菜单退出。".into()),
                        );
                    }
                }
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
                let command = scene.menu(Some(position));
                self.handle_menu(event_loop, command);
            }
            AppEvent::OpenSettings => self.open_settings(event_loop, None),
            AppEvent::TaskbarCreated => {
                scene.input.add_tray();
                if !scene.input.has_tray() {
                    if let Err(error) = scene.set_passthrough(false) {
                        eprintln!("Could not restore hit testing: {error}");
                    }
                    scene.pet.manual_hidden = false;
                    scene.update_display(Instant::now());
                    self.open_settings(
                        event_loop,
                        Some("系统托盘不可用。请从设置窗口退出。".into()),
                    );
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
                #[cfg(feature = "window-diagnostics")]
                if let Some(preview) = &self.preview {
                    preview.window().request_redraw();
                }
            }
            AppEvent::Gesture(gesture) => scene.handle_gesture(gesture),
            AppEvent::PageReady(result) => {
                if let Some(playback) = &mut scene.playback {
                    playback.accept(result);
                }
            }
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
        if self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.window().id() == id)
        {
            self.settings_event(event_loop, &event);
            return;
        }
        #[cfg(feature = "window-diagnostics")]
        if let Some(preview) = &mut self.preview
            && preview.window().id() == id
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
            } if !scene.passthrough => {
                let command = scene.menu(None);
                self.handle_menu(event_loop, command);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(feature = "window-diagnostics")]
        let preview = self
            .preview
            .as_mut()
            .and_then(InputPreview::repaint_deadline);
        let had_playback = self
            .scene
            .as_ref()
            .is_some_and(|scene| scene.playback.is_some());
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
        if had_playback
            && self
                .scene
                .as_ref()
                .is_some_and(|scene| scene.playback.is_none())
        {
            self.refresh_settings_character();
        }
        let settings = self
            .settings
            .as_mut()
            .and_then(SettingsWindow::repaint_deadline);
        let deadline = settings.into_iter().chain(pet).min();
        #[cfg(feature = "window-diagnostics")]
        let deadline = deadline.into_iter().chain(preview).min();
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        self.sound.stop();
        drop(self.playback.take());
    }
}

fn probe_dimensions(display: Option<Display>) -> Option<[u16; 2]> {
    let display = display?;
    let until = Instant::now() + Duration::from_millis(120);
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("monitor-edid".into())
        .spawn(move || {
            let _ = sender.send(physical_size(display.device));
        })
        .ok()?;
    receiver
        .recv_timeout(until.saturating_duration_since(Instant::now()))
        .ok()
        .flatten()
}

fn base_dip(display: Option<Display>, physical_mm: Option<[u16; 2]>, dpi_scale: f64) -> f64 {
    initial_side_dip(
        display.map_or(0, |display| display.bounds.max[1] - display.bounds.min[1]),
        physical_mm.map(|size| size[1]),
        dpi_scale,
        100,
    )
}
