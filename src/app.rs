use std::{error::Error, sync::Arc};

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
                    .with_title("Desktop Pet ChatGPT — P1a")
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
        input.apply_hit_region(&sample)?;
        let renderer = pollster::block_on(Renderer::new(Arc::clone(&window), &sample, proxy))?;
        window.set_visible(true);
        window.request_redraw();
        Ok(Self {
            input,
            renderer: Some(renderer),
            window,
            sample,
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
        self.visible = visible;
        self.window.set_visible(visible);
        if visible {
            self.window.request_redraw();
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
                if scene.visible {
                    scene.window.request_redraw();
                }
                if let Some(preview) = &self.preview {
                    preview.window.request_redraw();
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
                    &scene.sample,
                    self.proxy.clone(),
                )) {
                    Ok(renderer) => {
                        scene.renderer = Some(renderer);
                        scene.surface_retried = false;
                        scene.window.request_redraw();
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
                if let Err(error) = scene.input.apply_hit_region(&scene.sample) {
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
        let deadline = self.preview.as_mut().and_then(Preview::repaint_deadline);
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
