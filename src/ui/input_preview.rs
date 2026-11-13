use super::{ActiveEventLoop, Error, LogicalSize, Panel, Window, WindowEvent};
use std::time::Instant;

pub struct InputPreview {
    panel: Panel,
    draft: String,
    sample: String,
}

impl InputPreview {
    pub fn open(event_loop: &ActiveEventLoop) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            panel: Panel::open(
                event_loop,
                "桌宠 · 中文输入预览",
                LogicalSize::new(560, 460),
            )?,
            draft: String::new(),
            sample: (1..=48)
                .map(|line| format!("{line:02}  桌宠预览文本，可选中并复制。\n"))
                .collect(),
        })
    }

    pub fn window(&self) -> &Window {
        &self.panel.window
    }

    pub fn event(&mut self, event: &WindowEvent) {
        self.panel.event(event, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("中文输入预览");
                ui.label("在下方输入中文，或从示例文本中选取并复制内容。");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add(egui::Label::new(&self.sample).selectable(true));
                });
            });
        });
    }

    pub fn repaint_deadline(&mut self) -> Option<Instant> {
        self.panel.repaint_deadline()
    }
}
