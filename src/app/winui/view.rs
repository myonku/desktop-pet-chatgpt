use std::{
    io::{self, BufRead},
    sync::Arc,
};

use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
use windows_reactor::*;

use super::{Snapshot, ToPanel, ToPet, receive, send};
use crate::{
    core::{
        preferences::MovementMode,
        settings::{SettingsAction, SettingsDraft},
    },
    platform::windows::{
        QuickStartup, apply_embedded_panel_icons, choose_pack_folder, fix_settings_frame,
        panel_window,
    },
};

pub(in crate::app) fn run_child() -> Result<(), Box<dyn std::error::Error>> {
    match read_parent()? {
        ToPanel::Open(snapshot) => App::run_component::<Settings>(snapshot)?,
        ToPanel::Quick(snapshot) => {
            let _startup = QuickStartup::begin()?;
            App::run_component::<super::quick::Quick>(snapshot)?;
        }
        ToPanel::Client(snapshot) => {
            let _startup = snapshot
                .anchor
                .is_some()
                .then(QuickStartup::begin)
                .transpose()?;
            App::run_component::<super::client::Client>(snapshot)?;
        }
        _ => return Err("missing panel snapshot".into()),
    }
    let _ = send(io::stdout().lock(), &ToPet::Closed);
    Ok(())
}

pub(super) fn listen<C: Component>(
    context: &ComponentContext<C>,
    parent: fn(Arc<ToPanel>) -> C::Message,
    close: C::Message,
) where
    C::Message: Send + 'static,
{
    context.spawn_background(move |_| read_parent().map(Arc::new).map_or(close, parent));
}

fn read_parent() -> io::Result<ToPanel> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    receive(&line)
}

#[derive(Clone)]
enum Message {
    Parent(std::sync::Arc<ToPanel>),
    Ready,
    Change(fn(&mut SettingsDraft, bool), bool),
    Movement(Option<usize>),
    Connection(bool),
    Scale(f64),
    BubbleScale(f64),
    Pack(String),
    ChoosePack,
    Action(SettingsAction),
    Close,
}

const MOVEMENT_MODES: [(MovementMode, &str); 3] = [
    (MovementMode::Off, "关闭"),
    (MovementMode::Horizontal, "仅水平"),
    (MovementMode::AllDirections, "全方向"),
];

struct Settings {
    draft: SettingsDraft,
    base: SettingsDraft,
    conflict: bool,
    poster: Option<EncodedImage>,
    name: String,
    pack_status: Option<String>,
    sound: bool,
    diagonal_available: bool,
    notice: String,
    ready: bool,
}

impl Settings {
    fn action(&self, action: SettingsAction) {
        let _ = send(
            io::stdout().lock(),
            &ToPet::Action(action, self.draft.clone(), self.base.clone()),
        );
    }

    fn preview(&mut self) {
        self.conflict = false;
        self.action(SettingsAction::Preview);
    }
}

impl Component for Settings {
    type Input = Snapshot;
    type Message = Message;

    fn create(input: &Snapshot, context: &ComponentContext<Self>) -> Self {
        if !cfg!(feature = "window-diagnostics")
            || std::env::var_os("DESKTOP_PET_TEST_BLOCK_PANEL_INPUT").is_none()
        {
            listen(context, Message::Parent, Message::Close);
        }
        Self {
            draft: input.draft.clone(),
            base: input.draft.clone(),
            conflict: false,
            poster: None,
            name: input.name.clone(),
            pack_status: input.pack_status.clone(),
            sound: input.sound,
            diagonal_available: input.diagonal_available,
            notice: input.notice.clone(),
            ready: false,
        }
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Parent(message) => {
                match message.as_ref() {
                    ToPanel::Notice(notice) => self.notice.clone_from(notice),
                    ToPanel::Pack(pack) => {
                        self.name.clone_from(&pack.name);
                        self.pack_status.clone_from(&pack.status);
                        self.sound = pack.sound;
                        self.diagonal_available = pack.diagonal_available;
                        self.poster = pack.poster.clone().map(EncodedImage::new);
                    }
                    ToPanel::Reset(draft) => {
                        self.draft.clone_from(draft);
                        self.base.clone_from(draft);
                        self.conflict = false;
                    }
                    ToPanel::Committed(current) => self.draft.rebase(&mut self.base, current),
                    ToPanel::Saved { submitted, current } => {
                        self.base.clone_from(submitted);
                        self.draft.rebase(&mut self.base, current);
                        // A later local edit is now based on the saved value.
                        self.base.clone_from(current);
                        self.conflict = false;
                    }
                    ToPanel::Conflict(message) => {
                        self.notice.clone_from(message);
                        self.conflict = true;
                    }
                    _ => {}
                }
                listen(context, Message::Parent, Message::Close);
            }
            Message::Ready => {
                if !self.ready {
                    self.ready = true;
                    if let Some(hwnd) = panel_window(super::TITLE) {
                        if let Err(error) = fix_settings_frame(hwnd) {
                            eprintln!("Could not configure settings window frame: {error}");
                        }
                        apply_embedded_panel_icons(hwnd);
                        unsafe {
                            let _ = SetForegroundWindow(hwnd);
                        }
                        let _ = send(io::stdout().lock(), &ToPet::Ready(hwnd.0 as isize));
                    }
                }
            }
            Message::Change(set, value) => {
                set(&mut self.draft, value);
                self.preview();
            }
            Message::Movement(index) => {
                if let Some(&(mode, _)) = index.and_then(|index| MOVEMENT_MODES.get(index))
                    && self.draft.preferences.movement_mode() != mode
                {
                    self.draft.preferences.set_movement_mode(mode);
                    self.preview();
                }
            }
            Message::Connection(enabled) => {
                if self.draft.preferences.chatgpt_connection != enabled {
                    self.draft.preferences.chatgpt_connection = enabled;
                    self.conflict = false;
                    self.action(SettingsAction::DraftChanged);
                }
            }
            Message::Scale(value) => {
                self.draft.scale_percent = value.round() as u16;
                self.preview();
            }
            Message::BubbleScale(value) => {
                self.draft.preferences.bubble_scale_percent = value.round() as u16;
                self.preview();
            }
            Message::Pack(text) => {
                if text != self.draft.pack_path {
                    self.draft.pack_path = text;
                    self.conflict = false;
                    self.action(SettingsAction::DraftChanged);
                }
            }
            Message::ChoosePack => {
                let Some(owner) = panel_window(super::TITLE) else {
                    self.notice = "无法确定设置窗口。".into();
                    return;
                };
                match choose_pack_folder(owner) {
                    Ok(Some(path)) => match path.to_str() {
                        Some(path) => {
                            self.draft.pack_path = path.to_owned();
                            self.conflict = false;
                            self.action(SettingsAction::DraftChanged);
                            self.action(SettingsAction::CheckPack);
                        }
                        None => self.notice = "素材目录无法写入配置。".into(),
                    },
                    Ok(None) => {}
                    Err(error) => self.notice = format!("无法打开目录选择器：{error}"),
                }
            }
            Message::Action(action) => self.action(action),
            Message::Close => {
                let _ = context.window().request_close();
            }
        }
    }

    fn view(&self, input: &Snapshot, context: &mut ViewContext<Self>) -> View {
        context.window_title(super::TITLE);
        context.window_visuals(
            WindowVisuals::new()
                .client_size(300.0, 500.0)
                .backdrop(WindowBackdrop::Mica),
        );
        context.on_window_size(context.callback(|_| Message::Ready));
        let draft = &self.draft;
        let preferences = draft.preferences;
        let toggle = |label: &str, value, enabled, set| {
            CheckBox::new()
                .is_checked(value)
                .is_enabled(enabled)
                .on_is_checked_changed(
                    context.callback(move |checked| Message::Change(set, checked)),
                )
                .content(label)
        };
        let button = |action| Button::new().on_click(context.message(Message::Action(action)));
        let scale = |id, value: u16, min, max, message: fn(f64) -> Message| {
            Slider::new()
                .automation_id(id)
                .minimum(min)
                .maximum(max)
                .value(f64::from(value))
                .step_frequency(1.0)
                .on_value_changed(context.callback(message))
        };
        let poster: View = self.poster.as_ref().map_or_else(
            || {
                Border::new()
                    .width(48.0)
                    .height(48.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .into()
            },
            |image| {
                Image::new()
                    .width(48.0)
                    .height(48.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .automation_id("pack-preview")
                    .source_data(image.clone())
                    .into()
            },
        );
        let header = Grid::new()
            .columns([GridLength::Auto, GridLength::STAR])
            .column_spacing(12.0)
            .children((
                poster,
                TextBlock::new()
                    .text(self.name.clone())
                    .font_size(24.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .text_wrapping(TextWrapping::Wrap)
                    .vertical_alignment(VerticalAlignment::Center)
                    .grid_column(1),
            ));
        let movement_mode = preferences.movement_mode();
        let movement = StackPanel::new().spacing(4.0).children((
            trailing_row(
                TextBlock::new()
                    .text("自主移动")
                    .font_weight(FontWeight::SEMI_BOLD)
                    .horizontal_alignment(HorizontalAlignment::Left)
                    .vertical_alignment(VerticalAlignment::Center),
                ComboBox::new()
                    .automation_id("movement-mode")
                    .min_width(112.0)
                    .items_source(MOVEMENT_MODES.map(|(_, label)| label))
                    .selected_index(
                        MOVEMENT_MODES
                            .iter()
                            .position(|&(mode, _)| mode == movement_mode),
                    )
                    .on_selection_changed(context.callback(Message::Movement)),
            ),
            if movement_mode == MovementMode::AllDirections && !self.diagonal_available {
                TextBlock::new()
                    .text("当前素材包不支持全方向移动。")
                    .text_wrapping(TextWrapping::Wrap)
                    .into()
            } else {
                View::empty()
            },
        ));
        let behavior = StackPanel::new().spacing(3.0).children((
            movement,
            toggle(
                "当前屏全屏时隐藏",
                preferences.fullscreen_hide,
                true,
                |d: &mut SettingsDraft, v| d.preferences.fullscreen_hide = v,
            ),
            toggle(
                "下次启动时隐藏桌宠",
                preferences.start_hidden,
                input.tray,
                |d: &mut SettingsDraft, v| d.preferences.start_hidden = v,
            ),
            toggle(
                "全窗口鼠标穿透",
                preferences.passthrough,
                input.tray || preferences.passthrough,
                |d: &mut SettingsDraft, v| d.preferences.passthrough = v,
            ),
            trailing_row(
                toggle(
                    "音效",
                    preferences.sound,
                    true,
                    |d: &mut SettingsDraft, v| d.preferences.sound = v,
                ),
                button(SettingsAction::PlaySound)
                    .is_enabled(self.sound && preferences.sound)
                    .content("试听"),
            ),
            toggle(
                "任务气泡",
                preferences.task_bubbles,
                true,
                |d: &mut SettingsDraft, v| d.preferences.task_bubbles = v,
            ),
            slider_row(
                "缩放比例",
                draft.scale_percent,
                scale(
                    "pet-scale",
                    draft.scale_percent,
                    50.0,
                    200.0,
                    Message::Scale,
                ),
            ),
            slider_row(
                "气泡大小",
                preferences.bubble_scale_percent,
                scale(
                    "bubble-scale",
                    preferences.bubble_scale_percent,
                    50.0,
                    150.0,
                    Message::BubbleScale,
                )
                .is_enabled(preferences.task_bubbles),
            ),
            toggle(
                "登录 Windows 时启动",
                draft.autostart,
                input.autostart,
                |d: &mut SettingsDraft, v| d.autostart = v,
            ),
        ));
        let integration = CheckBox::new()
            .automation_id("chatgpt-connection")
            .is_checked(preferences.chatgpt_connection)
            .on_is_checked_changed(context.callback(Message::Connection))
            .content("连接到ChatGPT");
        let pack_warning = self
            .pack_status
            .as_ref()
            .map_or_else(View::empty, |message| {
                TextBlock::new()
                    .text(message.clone())
                    .text_wrapping(TextWrapping::Wrap)
                    .into()
            });
        let assets = StackPanel::new().spacing(8.0).children((
            TextBlock::new()
                .text("素材包目录（重启后生效）")
                .font_weight(FontWeight::SEMI_BOLD),
            TextBox::new()
                .automation_id("asset-pack-path")
                .text(draft.pack_path.clone())
                .placeholder_text("默认素材包")
                .is_enabled(!input.development)
                .on_text_changed(context.callback(Message::Pack)),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    Button::new()
                        .is_enabled(!input.development)
                        .on_click(context.message(Message::ChoosePack))
                        .content("选择目录"),
                    button(SettingsAction::CheckPack).content("检查素材包"),
                )),
            button(SettingsAction::RevealPack)
                .style(ButtonStyle::TextLink)
                .content("定位素材目录"),
            pack_warning,
        ));
        let maintenance = StackPanel::new().spacing(6.0).children((
            TextBlock::new()
                .text("维护")
                .font_weight(FontWeight::SEMI_BOLD),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    button(SettingsAction::Reset).content("恢复默认"),
                    button(SettingsAction::Exit).content("退出桌宠"),
                )),
        ));
        let body = ScrollViewer::new()
            .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
            .content(StackPanel::new().spacing(12.0).margin(16.0).children((
                header,
                divider(),
                behavior,
                divider(),
                integration,
                divider(),
                assets,
                divider(),
                maintenance,
            )));
        let (save, save_label) = if self.conflict {
            (SettingsAction::Overwrite, "覆盖冲突并保存")
        } else if input.development {
            (SettingsAction::Save, "应用本次设置")
        } else {
            (SettingsAction::Save, "应用并保存")
        };
        let notice = if self.notice.is_empty() {
            View::empty()
        } else {
            ScrollViewer::new()
                .max_height(72.0)
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .content(
                    TextBlock::new()
                        .text(self.notice.clone())
                        .text_wrapping(TextWrapping::Wrap),
                )
        };
        let footer = Border::new()
            .grid_row(1)
            .background(ThemeBrush::CardBackground)
            .border_brush(ThemeBrush::CardStroke)
            .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
            .padding(Thickness::new(16.0, 10.0, 16.0, 10.0))
            .content(
                StackPanel::new().spacing(8.0).children((
                    notice,
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .horizontal_alignment(HorizontalAlignment::Right)
                        .spacing(8.0)
                        .children((
                            Button::new()
                                .on_click(context.message(Message::Close))
                                .content("关闭"),
                            button(save).style(ButtonStyle::Accent).content(save_label),
                        )),
                )),
            );
        Grid::new()
            .rows([GridLength::STAR, GridLength::Auto])
            .children((body, footer))
    }
}

fn divider() -> View {
    Border::new()
        .background(ThemeBrush::CardStroke)
        .height(1.0)
        .into()
}

fn trailing_row(left: impl Into<View>, right: impl Into<View>) -> View {
    Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .children((
            left.into(),
            Border::new().grid_column(1).content(right.into()),
        ))
}

fn slider_row(label: &str, percent: u16, slider: Slider) -> View {
    StackPanel::new().children((
        trailing_row(
            TextBlock::new().text(label),
            TextBlock::new().text(format!("{percent}%")),
        ),
        slider,
    ))
}
