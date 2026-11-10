use std::{error::Error, num::NonZeroU32, sync::Arc, time::Instant};

use egui::ViewportId;
use egui_wgpu::{WgpuConfiguration, winit::Painter};
use winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::Window,
};

pub struct Preview {
    pub window: Arc<Window>,
    state: egui_winit::State,
    painter: Painter,
    draft: String,
    sample: String,
    repaint_at: Option<Instant>,
}

impl Preview {
    pub fn open(event_loop: &ActiveEventLoop) -> Result<Self, Box<dyn Error>> {
        let size = LogicalSize::new(560, 460);
        let mut attributes = Window::default_attributes()
            .with_title("桌宠 · 中文输入预览")
            .with_inner_size(size)
            .with_visible(false);
        if let Some(monitor) = event_loop.primary_monitor() {
            let origin = monitor.position();
            let screen = monitor.size();
            let preview: PhysicalSize<u32> = size.to_physical(monitor.scale_factor());
            attributes = attributes.with_position(PhysicalPosition::new(
                origin.x + (screen.width.saturating_sub(preview.width) / 2) as i32,
                origin.y + (screen.height.saturating_sub(preview.height) / 2) as i32,
            ));
        }
        let window = Arc::new(event_loop.create_window(attributes)?);
        let context = egui::Context::default();
        if let Some(font) = std::env::var_os("WINDIR")
            .and_then(|root| std::fs::read(std::path::Path::new(&root).join("Fonts/msyh.ttc")).ok())
        {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("yahei".into(), egui::FontData::from_owned(font).into());
            for family in fonts.families.values_mut() {
                family.push("yahei".into());
            }
            context.set_fonts(fonts);
        } else {
            eprintln!("Chinese preview: Microsoft YaHei is unavailable");
        }
        let mut painter = pollster::block_on(Painter::new(
            context.clone(),
            WgpuConfiguration::default(),
            false,
            Default::default(),
        ));
        pollster::block_on(painter.set_window(ViewportId::ROOT, Some(window.clone())))?;
        let state = egui_winit::State::new(
            context,
            ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            window.theme(),
            painter.max_texture_side(),
        );
        window.set_visible(true);
        window.focus_window();
        window.request_redraw();
        Ok(Self {
            window,
            state,
            painter,
            draft: String::new(),
            sample: (1..=48)
                .map(|line| format!("{line:02}  桌宠预览文本，可选中并复制。\n"))
                .collect(),
            repaint_at: None,
        })
    }

    pub fn event(&mut self, event: &WindowEvent) {
        let response = self.state.on_window_event(&self.window, event);
        if let WindowEvent::Resized(size) = event
            && let (Some(width), Some(height)) =
                (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            self.painter
                .on_window_resized(ViewportId::ROOT, width, height);
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            self.draw();
        } else if response.repaint || matches!(event, WindowEvent::Resized(_)) {
            self.window.request_redraw();
        }
    }

    pub fn repaint_deadline(&mut self) -> Option<Instant> {
        if self
            .repaint_at
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            self.repaint_at = None;
            self.window.request_redraw();
        }
        self.repaint_at
    }

    fn draw(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 || self.window.is_minimized() == Some(true) {
            self.repaint_at = None;
            return;
        }
        let context = self.state.egui_ctx().clone();
        let input = self.state.take_egui_input(&self.window);
        let output = context.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("中文输入预览");
                ui.label("在下方输入中文，或从示例文本中选取并复制内容。");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(&self.sample).selectable(true));
                    });
            });
        });
        self.state
            .handle_platform_output(&self.window, output.platform_output);
        let primitives = context.tessellate(output.shapes, output.pixels_per_point);
        self.painter.paint_and_update_textures(
            ViewportId::ROOT,
            output.pixels_per_point,
            [0.06, 0.06, 0.06, 1.0],
            &primitives,
            &output.textures_delta,
            Vec::new(),
        );
        self.repaint_at = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .and_then(|viewport| Instant::now().checked_add(viewport.repaint_delay));
    }
}
