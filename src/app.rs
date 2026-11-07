use std::{error::Error, sync::Arc};

use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::windows::WindowAttributesExtWindows,
    window::{Window, WindowId, WindowLevel},
};

use crate::render::Renderer;

#[derive(Default)]
struct App {
    scene: Option<(Renderer, Arc<Window>)>,
    error: Option<Box<dyn Error>>,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let mut app = App::default();
    EventLoop::new()?.run_app(&mut app)?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

impl App {
    fn open(event_loop: &ActiveEventLoop) -> Result<(Renderer, Arc<Window>), Box<dyn Error>> {
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
        let renderer = pollster::block_on(Renderer::new(Arc::clone(&window)))?;
        window.set_visible(true);
        window.request_redraw();
        Ok((renderer, window))
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.scene.is_none() {
            match Self::open(event_loop) {
                Ok(scene) => self.scene = Some(scene),
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some((renderer, window)) = &mut self.scene else {
            return;
        };
        if window.id() != id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let size = window.inner_size();
                if let Err(error) = renderer.draw(size.width, size.height) {
                    self.error = Some(error.into());
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}
