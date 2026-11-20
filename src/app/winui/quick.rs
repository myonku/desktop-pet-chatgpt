use std::{io, sync::Arc};

use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::AllowSetForegroundWindow};
use windows_reactor::*;

use crate::core::preferences::SIZE_RANGE;
use crate::platform::windows::{
    QuickWindow, panel_window, position_quick_window, reveal_quick_window,
};

use super::{
    HUB_TITLE, QUICK_MARGIN, QUICK_ROW_GAP, QUICK_ROW_HEIGHT, QUICK_TITLE, QUICK_WIDTH,
    QuickAction, QuickSnapshot, QuickValues, ToPanel, ToPet, quick_height, send, view::listen,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Menu {
    Actions,
    Settings,
}

#[derive(Clone)]
pub(super) enum Message {
    Ready,
    Parent(Arc<ToPanel>),
    Action(QuickAction),
    ToggleMenu(Menu),
    Close,
}

pub(super) struct Quick {
    parent_pid: u32,
    pet_window: isize,
    values: QuickValues,
    window: Option<QuickWindow>,
    menu: Option<Menu>,
}

impl Component for Quick {
    type Input = QuickSnapshot;
    type Message = Message;

    fn create(input: &QuickSnapshot, context: &ComponentContext<Self>) -> Self {
        listen(context, Message::Parent, Message::Close);
        Self {
            parent_pid: input.parent_pid,
            pet_window: input.pet_window,
            values: input.values,
            window: None,
            menu: None,
        }
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Ready if self.window.is_none() => {
                let open = || -> Result<_, Box<dyn std::error::Error>> {
                    let hwnd = panel_window(QUICK_TITLE).ok_or("missing quick window")?;
                    let pet = HWND(self.pet_window as _);
                    let window = QuickWindow::install(hwnd, pet, false)?;
                    position_quick_window(
                        hwnd,
                        pet,
                        QUICK_WIDTH,
                        quick_height(self.values.docked, self.menu.is_some()),
                    )?;
                    reveal_quick_window(hwnd)?;
                    Ok((window, hwnd))
                };
                match open() {
                    Ok((window, hwnd)) => {
                        self.window = Some(window);
                        let _ = send(io::stdout().lock(), &ToPet::Ready(hwnd.0 as isize));
                    }
                    Err(error) => {
                        eprintln!("Could not present quick panel: {error}");
                        let _ = context.window().request_close();
                    }
                }
            }
            Message::Ready => {}
            Message::Parent(message) => {
                if let ToPanel::QuickValues(values) = *message {
                    self.values = values;
                }
                listen(context, Message::Parent, Message::Close);
            }
            Message::Action(action) => {
                if matches!(action, QuickAction::Settings | QuickAction::Hub) {
                    unsafe {
                        let _ = AllowSetForegroundWindow(self.parent_pid);
                    }
                }
                let _ = send(io::stdout().lock(), &ToPet::QuickAction(action));
            }
            Message::ToggleMenu(menu) => {
                let was_expanded = self.menu.is_some();
                self.menu = (self.menu != Some(menu)).then_some(menu);
                let expanded = self.menu.is_some();
                if was_expanded != expanded {
                    let _ = send(io::stdout().lock(), &ToPet::QuickMenu(expanded));
                }
            }
            Message::Close => {
                let _ = context.window().request_close();
            }
        }
    }

    fn view(&self, input: &QuickSnapshot, context: &mut ViewContext<Self>) -> View {
        context.window_title(QUICK_TITLE);
        context.window_visuals(
            WindowVisuals::new()
                .client_size(
                    f64::from(QUICK_WIDTH),
                    f64::from(quick_height(input.values.docked, false)),
                )
                .backdrop(WindowBackdrop::Acrylic),
        );
        context.on_window_size(context.callback(|_| Message::Ready));
        let button = |label, action, enabled| {
            Button::new()
                .height(f64::from(QUICK_ROW_HEIGHT))
                .horizontal_alignment(HorizontalAlignment::Stretch)
                .is_enabled(enabled)
                .on_click(context.message(Message::Action(action)))
                .content(label)
        };
        let toggle_menu = |label, menu| {
            Button::new()
                .height(f64::from(QUICK_ROW_HEIGHT))
                .horizontal_alignment(HorizontalAlignment::Stretch)
                .on_click(context.message(Message::ToggleMenu(menu)))
                .content(label)
        };
        let values = self.values;
        let scale = Grid::new()
            .columns([
                GridLength::Pixel(34.0),
                GridLength::STAR,
                GridLength::Pixel(34.0),
            ])
            .column_spacing(4.0)
            .children((
                button(
                    "+",
                    QuickAction::Scale(10),
                    values.scale < *SIZE_RANGE.end(),
                ),
                Border::new().grid_column(1).content(
                    TextBlock::new()
                        .text("缩放")
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .vertical_alignment(VerticalAlignment::Center),
                ),
                Border::new().grid_column(2).content(button(
                    "-",
                    QuickAction::Scale(-10),
                    values.scale > *SIZE_RANGE.start(),
                )),
            ));
        let menu = match self.menu {
            Some(Menu::Actions) => menu_scroll(
                StackPanel::new().spacing(3.0).keyed_children(
                    input
                        .actions
                        .iter()
                        .enumerate()
                        .map(|(index, (label, enabled))| {
                            let item = Button::new()
                                .horizontal_alignment(HorizontalAlignment::Stretch)
                                .is_enabled(*enabled)
                                .on_click(
                                    context.message(Message::Action(QuickAction::Preview(index))),
                                )
                                .content(
                                    TextBlock::new()
                                        .text(label.clone())
                                        .text_wrapping(TextWrapping::Wrap),
                                );
                            (index as u64, item)
                        }),
                ),
            ),
            Some(Menu::Settings) => {
                let toggle = |label, checked, action: fn(bool) -> QuickAction| {
                    CheckBox::new()
                        .is_checked(checked)
                        .on_is_checked_changed(
                            context.callback(move |value| Message::Action(action(value))),
                        )
                        .content(label)
                };
                menu_scroll(StackPanel::new().spacing(2.0).children((
                    toggle("自移动", values.auto_move, QuickAction::AutoMove),
                    toggle("音效", values.sound, QuickAction::Sound),
                    toggle(
                        "全屏隐藏",
                        values.fullscreen_hide,
                        QuickAction::FullscreenHide,
                    ),
                    button("全部设置", QuickAction::Settings, true),
                    if input.tray {
                        View::empty()
                    } else {
                        button("退出桌宠", QuickAction::Exit, true)
                    },
                )))
            }
            None => View::empty(),
        };
        let toolbar = StackPanel::new()
            .spacing(f64::from(QUICK_ROW_GAP))
            .margin(f64::from(QUICK_MARGIN));
        ScrollViewer::new()
            .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
            .content(toolbar.children((
                button(HUB_TITLE, QuickAction::Hub, true),
                scale,
                if values.docked {
                    button("展开桌宠", QuickAction::Expand, true)
                } else {
                    View::empty()
                },
                toggle_menu(
                    if self.menu == Some(Menu::Actions) {
                        "收起动作"
                    } else {
                        "动作预览"
                    },
                    Menu::Actions,
                ),
                toggle_menu(
                    if self.menu == Some(Menu::Settings) {
                        "收起设置"
                    } else {
                        "快捷设置"
                    },
                    Menu::Settings,
                ),
                menu,
            )))
    }
}

fn menu_scroll(content: View) -> View {
    Border::new()
        .background(ThemeBrush::CardBackground)
        .corner_radius(6.0)
        .padding(3.0)
        .content(
            ScrollViewer::new()
                .max_height(144.0)
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .content(content),
        )
}
