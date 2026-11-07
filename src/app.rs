use std::{error::Error, sync::Arc};

use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    platform::windows::WindowAttributesExtWindows,
    window::{Window, WindowId, WindowLevel},
};

use crate::{
    platform::windows::{Command, InputEvent, WindowInput},
    render::Renderer,
    sample::Sample,
};

struct App {
    scene: Option<Scene>,
    proxy: EventLoopProxy<InputEvent>,
    error: Option<Box<dyn Error>>,
}

struct Scene {
    input: WindowInput,
    renderer: Renderer,
    window: Arc<Window>,
    sample: Sample,
    passthrough: bool,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::<InputEvent>::with_user_event().build()?;
    let mut app = App {
        scene: None,
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
        let mut input = WindowInput::new(&window, proxy)?;
        input.apply_hit_region(&sample)?;
        let renderer = pollster::block_on(Renderer::new(Arc::clone(&window), &sample))?;
        window.set_visible(true);
        window.request_redraw();
        Ok(Self {
            input,
            renderer,
            window,
            sample,
            passthrough: false,
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

    fn menu(&mut self, event_loop: &ActiveEventLoop, position: Option<PhysicalPosition<i32>>) {
        match self.input.menu(position, self.passthrough) {
            Ok(Some(Command::TogglePassthrough)) => self.set_passthrough(!self.passthrough),
            Ok(Some(Command::Exit)) => event_loop.exit(),
            Ok(None) => {}
            Err(error) => eprintln!("Could not open prototype menu: {error}"),
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
            InputEvent::TrayMenu(position) => scene.menu(event_loop, Some(position)),
            InputEvent::TaskbarCreated => {
                scene.input.add_tray();
                if !scene.input.has_tray() {
                    scene.set_passthrough(false);
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
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Err(error) = scene.input.apply_hit_region(&scene.sample) {
                    self.error = Some(error.into());
                    event_loop.exit();
                }
                scene.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let size = scene.window.inner_size();
                if let Err(error) = scene.renderer.draw(size.width, size.height) {
                    self.error = Some(error.into());
                    event_loop.exit();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Right,
                ..
            } if !scene.passthrough => scene.menu(event_loop, None),
            _ => {}
        }
    }
}
