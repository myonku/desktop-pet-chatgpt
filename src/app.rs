use std::{error::Error, path::Path, sync::Arc, time::Instant};

use desktop_pet_chatgpt::media::Variant;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    platform::windows::WindowAttributesExtWindows,
    window::{Window, WindowId, WindowLevel},
};

use crate::{
    platform::windows::{Command, InputEvent, WindowInput},
    playback::Playback,
    render::Renderer,
    sample::Sample,
    ui::Preview,
};

struct App {
    scene: Option<Scene>,
    preview: Option<Preview>,
    proxy: EventLoopProxy<InputEvent>,
    error: Option<Box<dyn Error>>,
}

struct Scene {
    input: WindowInput,
    renderer: Option<Renderer>,
    window: Arc<Window>,
    sample: Sample,
    playback: Option<Playback>,
    wants_bc7: bool,
    passthrough: bool,
    visible: bool,
    surface_retried: bool,
    device_rebuilt: bool,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::<InputEvent>::with_user_event().build()?;
    let mut app = App {
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
        proxy: EventLoopProxy<InputEvent>,
    ) -> Result<Self, Box<dyn Error>> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Desktop Pet ChatGPT — P1")
                    .with_inner_size(LogicalSize::new(256, 256))
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_active(false)
                    .with_visible(false)
                    .with_window_level(WindowLevel::AlwaysOnTop)
                    .with_transparent(true)
                    .with_no_redirection_bitmap(true),
            )?,
        );
        eprintln!(
            "Window: DPI scale {:.2}, client size {:?}",
            window.scale_factor(),
            window.inner_size()
        );
        let sample = Sample::load()?;
        let mut input = WindowInput::new(&window, proxy.clone())?;
        input.apply_hit_region(sample.view())?;
        let variant = match std::env::var("DESKTOP_PET_FORMAT").as_deref() {
            Ok("rgba8") => Variant::Rgba8,
            Ok("bc7") => Variant::Bc7,
            Ok("png") | Err(_) => Variant::Png,
            Ok(other) => {
                eprintln!("Unknown asset format {other}; using PNG");
                Variant::Png
            }
        };
        let mut playback =
            std::env::var_os("DESKTOP_PET_PACK").and_then(|root| {
                match Playback::open(Path::new(&root), variant, proxy.clone()) {
                    Ok(playback) => Some(playback),
                    Err(error) => {
                        eprintln!("Could not open asset pack; using placeholder: {error}");
                        None
                    }
                }
            });
        let wants_bc7 = playback.is_some() && variant == Variant::Bc7;
        let renderer = pollster::block_on(Renderer::new(
            Arc::clone(&window),
            sample.view(),
            wants_bc7,
            proxy,
        ))?;
        if !renderer.supports_bc7()
            && let Some(playback) = &mut playback
        {
            playback.use_compatible_format();
        }
        window.set_visible(true);
        window.request_redraw();
        Ok(Self {
            input,
            renderer: Some(renderer),
            window,
            sample,
            playback,
            wants_bc7,
            passthrough: false,
            visible: true,
            surface_retried: false,
            device_rebuilt: false,
        })
    }

    fn set_passthrough(&mut self, enabled: bool) {
        if self.passthrough == enabled || (enabled && !self.input.has_tray()) {
            return;
        }
        match self.window.set_cursor_hittest(!enabled) {
            Ok(()) => {
                self.passthrough = enabled;
                self.window.request_redraw();
            }
            Err(error) => eprintln!("Could not change mouse passthrough: {error}"),
        }
    }

    fn set_visible(&mut self, visible: bool) {
        if self.visible
            && !visible
            && let Some(playback) = &mut self.playback
        {
            playback.pause();
        }
        self.visible = visible;
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
        if let Some(renderer) = &mut self.renderer {
            renderer.show(key, frame)?;
        }
        self.input.invalidate_hit_region();
        self.input.apply_hit_region(frame)?;
        self.window.request_redraw();
        Ok(())
    }

    fn fallback(&mut self, error: impl std::fmt::Display) -> Result<(), String> {
        eprintln!("Asset playback stopped; using placeholder: {error}");
        self.playback = None;
        self.refresh_frame()
    }

    fn playback_deadline(&mut self) -> Result<Option<Instant>, String> {
        if !self.visible || self.window.is_minimized() == Some(true) {
            return Ok(None);
        }
        let Some(playback) = &mut self.playback else {
            return Ok(None);
        };
        match playback.tick(Instant::now()) {
            Ok((changed, deadline)) => {
                if changed && let Err(error) = self.refresh_frame() {
                    self.fallback(error)?;
                    return Ok(None);
                }
                Ok(deadline)
            }
            Err(error) => {
                self.fallback(error)?;
                Ok(None)
            }
        }
    }

    fn menu(
        &mut self,
        event_loop: &ActiveEventLoop,
        position: Option<PhysicalPosition<i32>>,
    ) -> bool {
        match self.input.menu(position, self.passthrough, self.visible) {
            Ok(Some(Command::TogglePassthrough)) => self.set_passthrough(!self.passthrough),
            Ok(Some(Command::ToggleVisibility)) => self.set_visible(!self.visible),
            Ok(Some(Command::OpenPreview)) => return true,
            Ok(Some(Command::Exit)) => event_loop.exit(),
            Ok(None) => {}
            Err(error) => eprintln!("Could not open prototype menu: {error}"),
        }
        false
    }

    fn redraw(&mut self) -> Result<(), wgpu::SurfaceError> {
        if !self.visible {
            return Ok(());
        }
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        let size = self.window.inner_size();
        match renderer.draw(size.width, size.height) {
            Ok(()) => self.surface_retried = false,
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

impl ApplicationHandler<InputEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.scene.is_none() {
            match Scene::open(event_loop, self.proxy.clone()) {
                Ok(scene) => self.scene = Some(scene),
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                }
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: InputEvent) {
        let Some(scene) = &mut self.scene else {
            return;
        };
        match event {
            InputEvent::TrayMenu(position) => {
                if scene.menu(event_loop, Some(position)) {
                    self.open_preview(event_loop);
                }
            }
            InputEvent::TaskbarCreated => {
                scene.input.add_tray();
                if !scene.input.has_tray() {
                    scene.set_passthrough(false);
                    scene.set_visible(true);
                }
            }
            InputEvent::ResumeDisplay => {
                scene.surface_retried = false;
                if let Some(playback) = &mut scene.playback {
                    playback.pause();
                }
                if scene.visible {
                    scene.window.request_redraw();
                }
                if let Some(preview) = &self.preview {
                    preview.window.request_redraw();
                }
            }
            InputEvent::PageReady(result) => {
                if let Some(playback) = &mut scene.playback {
                    playback.accept(result);
                }
            }
            InputEvent::PetDeviceLost => {
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
                    scene.wants_bc7 && scene.playback.is_some(),
                    self.proxy.clone(),
                )) {
                    Ok(renderer) => {
                        if let Some(playback) = &mut scene.playback {
                            if !renderer.supports_bc7() {
                                playback.use_compatible_format();
                            }
                            playback.invalidate_frame();
                        }
                        scene.renderer = Some(renderer);
                        scene.surface_retried = false;
                        if let Err(error) =
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
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let frame = scene
                    .playback
                    .as_ref()
                    .and_then(Playback::frame)
                    .map_or(scene.sample.view(), |(_, frame)| frame);
                if let Err(error) = scene.input.apply_hit_region(frame) {
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
