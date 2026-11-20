mod assets;
mod behavior;
mod clients;
mod config;
mod launch;
mod migration;
mod settings;
mod winui;

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
    core::{Activity, Pet, initial_side_dip, preferences::INITIAL_SCALE},
    media::playback::{PageResult, Playback},
    platform::windows::{
        Autostart, Command, Display, Gesture, Instance, Sound, WindowInput, physical_size,
        user_profile,
    },
    render::Renderer,
};

use self::{
    assets::{CharacterAssets, LoadedPack, load_ready_pack},
    settings::RuntimeSettings,
};

pub(crate) enum AppEvent {
    TrayMenu(PhysicalPosition<i32>),
    MenuSelected(Command),
    ModalTick,
    TaskbarCreated,
    ResumeDisplay,
    Gesture(Gesture),
    PetDeviceLost,
    OpenSettings,
    PageReady(PageResult),
    SettingsPackChecked(settings::PackCheckResult),
    Winui(winui::PanelEvent),
    Client(clients::Event),
}

struct App {
    options: launch::LaunchOptions,
    scene: Option<Scene>,
    settings: Option<winui::Panel>,
    quick: Option<winui::Panel>,
    hub: Option<winui::Panel>,
    hub_position: Option<[i32; 2]>,
    hub_client: clients::ClientId,
    clients: clients::Clients,
    pack_check: Option<settings::PackCheck>,
    next_panel_session: u64,
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
    playback: Playback,
    pack_path: PathBuf,
    default_pack_fallback: Option<PathBuf>,
    pack_status: Option<String>,
    assets: CharacterAssets,
    sound: Sound,
    runtime: RuntimeSettings,
    diagonal_angle: f64,
    passthrough: bool,
    visible: bool,
    minimized: bool,
    pet: Pet,
    display: Option<Display>,
    screen_poll: Instant,
    applied_size: Option<PhysicalSize<u32>>,
    fixed_geometry: bool,
    probe_edid: bool,
    unavailable: std::collections::BTreeSet<usize>,
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
    if args.len() == 1 && args[0] == "--panel-child" {
        return winui::run_child();
    }
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
        clients: clients::Clients::new(options.startup.config.preferences.chatgpt_connection),
        hub_client: clients::ClientId::ChatGpt,
        options,
        scene: None,
        settings: None,
        quick: None,
        hub: None,
        hub_position: None,
        pack_check: None,
        next_panel_session: 0,
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
        let mut renderer = pollster::block_on(Renderer::new(Arc::clone(&window), proxy.clone()))?;
        let default_pack = options.startup.default_pack()?;
        let (pack_path, pack, pack_status) = assets::load_startup_pack(
            options.startup.pack_path.as_deref(),
            default_pack.clone(),
            &proxy,
            &mut renderer,
        )?;
        if options.trace_playback {
            pack.trace_resources();
        }
        let LoadedPack {
            playback,
            mut assets,
            click_wave,
            tray_icon,
        } = pack;
        assets.apply_window_icon(&window);
        let input = WindowInput::new(
            &window,
            proxy.clone(),
            options.trace_input,
            instance.marker(),
            tray_icon,
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
            playback,
            default_pack_fallback: (pack_path != default_pack).then_some(default_pack),
            pack_path,
            pack_status,
            assets,
            sound,
            runtime,
            diagonal_angle: options.diagonal_angle,
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
        let (key, frame) = self.playback.frame().ok_or("素材包未提供可显示的帧。")?;
        let started = self.trace.as_ref().map(|_| Instant::now());
        if let Some(renderer) = &mut self.renderer {
            renderer.show(key, frame)?;
        }
        if let (Some(trace), Some(started)) = (&mut self.trace, started) {
            let prepare_us = started.elapsed().as_micros();
            if self.playback.sequence().is_some() && self.playback.active_id() != trace.action {
                trace.action = self.playback.active_id().to_owned();
                eprintln!(
                    "Playback action: {} {}",
                    trace.action,
                    trace.start.elapsed().as_micros()
                );
            }
            trace.pending = self
                .playback
                .sequence()
                .map(|sequence| (sequence, key.index, prepare_us));
        }
        self.input.invalidate_hit_region();
        self.input.apply_hit_region(frame)?;
        self.window.request_redraw();
        Ok(())
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
                let edge = self.playback.selected_is_edge_pose();
                if let Some(trace) = &mut self.trace
                    && let Some((sequence, page, prepare_us)) = trace.pending.take()
                {
                    eprintln!(
                        "Playback frame {sequence} {page} {} {prepare_us}",
                        trace.start.elapsed().as_micros()
                    );
                }
                if !self.first_asset_presented {
                    self.first_asset_presented = true;
                    eprintln!(
                        "First asset frame presented: {}x{}",
                        size.width, size.height
                    );
                }
                self.input
                    .frame_presented(edge, self.pet.activity == Activity::Drag && !edge);
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
    fn recover_default_pack(&mut self, error: String) -> Result<Instant, String> {
        let scene = self.scene.as_mut().unwrap();
        let Some(path) = scene.default_pack_fallback.take() else {
            return Err(error);
        };
        eprintln!("{error}");
        let pack = load_ready_pack(&path, &self.proxy, scene.renderer.as_mut())
            .map_err(|fallback| format!("{error}；默认素材包加载失败：{fallback}"))?;
        let LoadedPack {
            playback,
            mut assets,
            click_wave,
            tray_icon,
        } = pack;
        assets.apply_window_icon(&scene.window);
        scene.input.replace_tray_icon(tray_icon.as_ref());
        scene.assets = assets;
        scene.sound.replace(click_wave);
        scene.playback = playback;
        scene.pack_path = path;
        let status = "指定素材包的待机动作无法播放，已切换到默认素材包。";
        scene.pack_status = Some(status.into());
        scene.unavailable.clear();
        scene.input.cancel_gesture();
        scene.pet.edge = None;
        scene.pet.peek = false;
        let now = Instant::now();
        scene.pet.reset(now);
        scene.update_display(now);
        if scene.visible {
            scene.refresh_frame()?;
        }
        eprintln!("{status}");
        self.close_quick();
        self.notify_panel_pack();
        Ok(now)
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
                        self.open_settings(Some(
                            "系统托盘不可用。请从此窗口或角色右键菜单退出。".into(),
                        ));
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
            AppEvent::SettingsPackChecked(result) => self.finish_pack_check(result),
            AppEvent::Winui(event) => self.handle_winui(event_loop, event),
            AppEvent::Client(event) => {
                let now = Instant::now();
                self.clients.complete(event, now);
                self.refresh_client_panels(now);
            }
            AppEvent::TrayMenu(position) => {
                self.close_quick();
                self.scene.as_ref().unwrap().tray_menu(position);
            }
            AppEvent::MenuSelected(command) => {
                let command = scene.menu_selected(command);
                self.handle_menu(event_loop, command);
            }
            AppEvent::ModalTick => self.about_to_wait(event_loop),
            AppEvent::OpenSettings => self.open_settings(None),
            AppEvent::TaskbarCreated => {
                scene.input.add_tray();
                if !scene.input.has_tray() {
                    if let Err(error) = scene.set_passthrough(false) {
                        eprintln!("Could not restore hit testing: {error}");
                    }
                    scene.pet.manual_hidden = false;
                    scene.update_display(Instant::now());
                    self.open_settings(Some("系统托盘不可用。请从设置窗口退出。".into()));
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
                self.anchor_quick();
            }
            AppEvent::Gesture(gesture) => {
                let was_docked = scene.pet.edge.is_some();
                let overview = scene.handle_gesture(gesture);
                let dock_changed = was_docked != scene.pet.edge.is_some();
                if dock_changed {
                    self.refresh_quick_values();
                }
                if overview {
                    self.toggle_overview();
                }
            }
            AppEvent::PageReady(result) => {
                scene.playback.accept(result);
            }
            AppEvent::PetDeviceLost => {
                if scene.device_rebuilt {
                    self.error = Some("Pet GPU device was lost again after recovery".into());
                    event_loop.exit();
                    return;
                }
                scene.device_rebuilt = true;
                drop(scene.renderer.take());
                match pollster::block_on(Renderer::new(scene.window.clone(), self.proxy.clone())) {
                    Ok(renderer) => {
                        scene.renderer = Some(renderer);
                        scene.pet.reset(Instant::now());
                        scene.play_base();
                        scene.surface_retried = false;
                        if scene.visible
                            && let Err(error) = scene.refresh_frame()
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
                self.anchor_quick();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                scene.screen_poll = Instant::now();
                if scene.visible
                    && let Some((_, frame)) = scene.playback.frame()
                    && let Err(error) = scene.input.apply_hit_region(frame)
                {
                    self.error = Some(error.into());
                    event_loop.exit();
                }
                scene.window.request_redraw();
                self.anchor_quick();
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
            } if !scene.passthrough && scene.visible => {
                self.toggle_quick();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let previous_display = self.scene.as_ref().and_then(|scene| scene.display);
        let was_docked = self
            .scene
            .as_ref()
            .is_some_and(|scene| scene.pet.edge.is_some());
        let pet = match self
            .scene
            .as_mut()
            .map(Scene::playback_deadline)
            .transpose()
        {
            Ok(deadline) => deadline.flatten(),
            Err(error) => match self.recover_default_pack(error) {
                Ok(now) => Some(now),
                Err(error) => {
                    self.error = Some(error.into());
                    event_loop.exit();
                    return;
                }
            },
        };
        if self
            .scene
            .as_ref()
            .is_some_and(|scene| !scene.visible || scene.minimized || scene.passthrough)
        {
            self.close_quick();
        } else if previous_display != self.scene.as_ref().and_then(|scene| scene.display) {
            self.anchor_quick();
        }
        if self.quick.is_some()
            && self
                .scene
                .as_ref()
                .is_some_and(|scene| scene.pet.edge.is_some())
                != was_docked
        {
            self.refresh_quick_values();
        }
        let now = Instant::now();
        let (client, changed) = self.clients.tick(now, self.overview_open(), &self.proxy);
        if changed {
            self.refresh_client_panels(now);
        }
        let deadline = pet
            .into_iter()
            .chain(self.panel_deadline())
            .chain(client)
            .min();
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        self.sound.stop();
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
