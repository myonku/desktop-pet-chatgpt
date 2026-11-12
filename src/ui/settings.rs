use std::{error::Error, time::Instant};

use winit::{dpi::LogicalSize, event::WindowEvent, event_loop::ActiveEventLoop, window::Window};

use super::Panel;
use crate::core::preferences::{Preferences, SIZE_RANGE};

#[derive(Clone, PartialEq)]
pub struct SettingsDraft {
    pub preferences: Preferences,
    pub scale_percent: u16,
    pub autostart: bool,
    pub pack_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Preview,
    Save,
    Reset,
    PlaySound,
    RevealPack,
    Exit,
}

pub struct SettingsWindow {
    panel: Panel,
    pub draft: SettingsDraft,
    pub notice: Option<String>,
    pub saved_autostart: Option<bool>,
    pub development: bool,
    poster: Option<egui::TextureHandle>,
    name: String,
    trace_layout: bool,
    last_layout: Vec<(&'static str, egui::Rect)>,
}

impl SettingsWindow {
    pub fn open(
        event_loop: &ActiveEventLoop,
        draft: SettingsDraft,
        development: bool,
        trace_layout: bool,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            panel: Panel::open(event_loop, "桌宠 · 设置", LogicalSize::new(520, 625))?,
            draft,
            development,
            notice: None,
            saved_autostart: None,
            poster: None,
            name: "桌宠".to_owned(),
            trace_layout,
            last_layout: Vec::new(),
        })
    }

    pub fn window(&self) -> &Window {
        &self.panel.window
    }

    pub fn character(&mut self, name: Option<&str>, poster: &egui::ColorImage) {
        self.name = name.unwrap_or("桌宠").to_owned();
        self.poster = Some(self.panel.state.egui_ctx().load_texture(
            "character.poster",
            poster.clone(),
            egui::TextureOptions::LINEAR,
        ));
        self.window().request_redraw();
    }

    pub fn event(
        &mut self,
        event: &WindowEvent,
        tray: bool,
        wave: bool,
        pack_label: &str,
    ) -> Option<SettingsAction> {
        let before = (self.draft.preferences, self.draft.scale_percent);
        let mut layout = Vec::new();
        let mut record = |key, response: egui::Response| {
            if self.trace_layout {
                layout.push((key, response.rect));
            }
            response
        };
        let draft = &mut self.draft;
        let mut action = None;
        self.panel.event(event, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(poster) = &self.poster {
                        let size = poster.size_vec2();
                        ui.image((poster.id(), size * (56.0 / size.x.max(size.y))));
                    }
                    ui.vertical(|ui| {
                        ui.heading(&self.name);
                        ui.label("桌宠设置");
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.development {
                            ui.label("开发配置生效中。本窗口的改动只影响本次运行。");
                        }

                        record(
                            "scale",
                            ui.add(
                                egui::Slider::new(&mut draft.scale_percent, SIZE_RANGE)
                                    .text("桌宠大小")
                                    .suffix("%"),
                            ),
                        );
                        ui.checkbox(&mut draft.preferences.auto_move, "自移动");
                        ui.checkbox(&mut draft.preferences.fullscreen_hide, "当前屏全屏时隐藏");
                        ui.add_enabled(
                            tray || draft.preferences.passthrough,
                            egui::Checkbox::new(
                                &mut draft.preferences.passthrough,
                                "全窗口鼠标穿透",
                            ),
                        );
                        if !tray {
                            ui.label("托盘不可用，无法开启穿透。");
                        }

                        ui.separator();
                        ui.horizontal(|ui| {
                            record("sound", ui.checkbox(&mut draft.preferences.sound, "音效"));
                            if record(
                                "test_sound",
                                ui.add_enabled(
                                    wave && draft.preferences.sound,
                                    egui::Button::new("试听"),
                                ),
                            )
                            .clicked()
                            {
                                action = Some(SettingsAction::PlaySound);
                            }
                        });
                        if !wave {
                            ui.label("当前素材包未提供可播放的互动音效。");
                        }
                        record(
                            "task_bubbles",
                            ui.checkbox(&mut draft.preferences.task_bubbles, "任务气泡"),
                        );
                        ui.add_enabled(
                            draft.preferences.task_bubbles,
                            egui::Slider::new(
                                &mut draft.preferences.bubble_scale_percent,
                                50..=150,
                            )
                            .text("气泡大小")
                            .suffix("%"),
                        );
                        ui.label("任务气泡在外部任务集成接入后生效。");

                        ui.separator();
                        ui.add_enabled(
                            self.saved_autostart.is_some() && !self.development,
                            egui::Checkbox::new(&mut draft.autostart, "登录 Windows 时启动"),
                        );
                        if self.saved_autostart.is_none() && !self.development {
                            ui.label("当前环境无法设置登录自启。");
                        }
                        ui.label("素材包目录（重启后生效，留空使用默认包）");
                        record(
                            "pack_path",
                            ui.add_enabled(
                                !self.development,
                                egui::TextEdit::singleline(&mut draft.pack_path)
                                    .desired_width(f32::INFINITY),
                            ),
                        );
                        ui.label(pack_label);
                        if ui.button("定位素材目录").clicked() {
                            action = Some(SettingsAction::RevealPack);
                        }

                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            let label = if self.development {
                                "应用本次设置"
                            } else {
                                "应用并保存"
                            };
                            if record("save", ui.button(label)).clicked() {
                                action = Some(SettingsAction::Save);
                            }
                            if record("reset", ui.button("恢复默认")).clicked() {
                                action = Some(SettingsAction::Reset);
                            }
                            if record("exit", ui.button("退出桌宠")).clicked() {
                                action = Some(SettingsAction::Exit);
                            }
                        });
                        if let Some(notice) = &self.notice {
                            ui.label(notice);
                        }
                    });
            });
        });
        if !layout.is_empty() && layout != self.last_layout {
            for (key, rect) in &layout {
                eprintln!(
                    "P2 settings control: {key} {:.2} {:.2} {:.2} {:.2}",
                    rect.left(),
                    rect.top(),
                    rect.right(),
                    rect.bottom()
                );
            }
            self.last_layout = layout;
        }
        action.or_else(|| {
            ((draft.preferences, draft.scale_percent) != before).then_some(SettingsAction::Preview)
        })
    }

    pub fn repaint_deadline(&mut self) -> Option<Instant> {
        self.panel.repaint_deadline()
    }
}
