use std::{
    io,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            AllowSetForegroundWindow, GetWindowRect, IsIconic, IsZoomed, SWP_NOACTIVATE,
            SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, WM_CLOSE, WM_EXITSIZEMOVE,
            WM_NCDESTROY,
        },
    },
};
use windows_reactor::*;

use crate::platform::windows::{
    QuickWindow, apply_embedded_panel_icons, panel_window, position_quick_window,
    reveal_quick_window,
};
use crate::{
    app::clients::{ClientId, ClientView, local_clock_label, local_reset_label, utc_label},
    integrations::chatgpt::{CodexUsage, QueryError, UsageWindow},
};

use super::{
    ClientAction, ClientPanels, ClientSnapshot, HUB_TITLE, OVERVIEW_TITLE, OVERVIEW_WIDTH, ToPanel,
    ToPet, send, view::listen,
};

#[derive(Clone)]
pub(super) enum Message {
    Ready,
    Parent(Arc<ToPanel>),
    Action(ClientAction),
    Close,
}

pub(super) struct Client {
    ready: bool,
    view: ClientPanels,
    floating: Option<QuickWindow>,
    anchor: Option<isize>,
    parent_pid: u32,
    position: Option<[i32; 2]>,
}

impl Client {
    fn show(&mut self) -> Result<HWND, Box<dyn std::error::Error>> {
        let title = if self.anchor.is_some() {
            OVERVIEW_TITLE
        } else {
            HUB_TITLE
        };
        let hwnd = panel_window(title).ok_or("missing client information window")?;
        if let Some(anchor) = self.anchor {
            let pet = HWND(anchor as _);
            self.floating = Some(QuickWindow::install(hwnd, pet, true)?);
            position_quick_window(hwnd, pet, OVERVIEW_WIDTH, self.view.overview_height())?;
            reveal_quick_window(hwnd)?;
        } else {
            if let Some(position) = self.position
                && let Err(error) = restore_position(hwnd, position)
            {
                eprintln!("Could not restore hub position: {error}");
            }
            apply_embedded_panel_icons(hwnd);
            unsafe {
                let _ = SetWindowSubclass(hwnd, Some(remember_position), HUB_SUBCLASS, 0);
                let _ = SetForegroundWindow(hwnd);
            }
        }
        Ok(hwnd)
    }
}

impl Component for Client {
    type Input = ClientSnapshot;
    type Message = Message;

    fn create(input: &ClientSnapshot, context: &ComponentContext<Self>) -> Self {
        listen(context, Message::Parent, Message::Close);
        Self {
            ready: false,
            view: input.view.clone(),
            floating: None,
            anchor: input.anchor,
            parent_pid: input.parent_pid,
            position: input.position,
        }
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Ready if !self.ready => {
                self.ready = true;
                match self.show() {
                    Ok(hwnd) => {
                        let _ = send(io::stdout().lock(), &ToPet::Ready(hwnd.0 as isize));
                    }
                    Err(error) => {
                        eprintln!("Could not present client information: {error}");
                        let _ = context.window().request_close();
                    }
                }
            }
            Message::Ready => {}
            Message::Parent(message) => {
                if let ToPanel::ClientValues(view) = message.as_ref() {
                    self.view.clone_from(view);
                }
                listen(context, Message::Parent, Message::Close);
            }
            Message::Action(action) => {
                if matches!(action, ClientAction::Expand(_) | ClientAction::Settings) {
                    unsafe {
                        let _ = AllowSetForegroundWindow(self.parent_pid);
                    }
                }
                let _ = send(io::stdout().lock(), &ToPet::ClientAction(action));
            }
            Message::Close => {
                let _ = context.window().request_close();
            }
        }
    }

    fn view(&self, _: &ClientSnapshot, context: &mut ViewContext<Self>) -> View {
        let overview = self.anchor.is_some();
        let (title, width, height) = if overview {
            (
                OVERVIEW_TITLE,
                f64::from(OVERVIEW_WIDTH),
                f64::from(self.view.overview_height()),
            )
        } else {
            (HUB_TITLE, 500.0, 340.0)
        };
        context.window_title(title);
        let backdrop = if overview {
            WindowBackdrop::Acrylic
        } else {
            WindowBackdrop::Mica
        };
        context.window_visuals(
            WindowVisuals::new()
                .client_size(width, height)
                .backdrop(backdrop),
        );
        context.on_window_size(context.callback(|_| Message::Ready));

        let button = |label, action, enabled| {
            Button::new()
                .is_enabled(enabled)
                .on_click(context.message(Message::Action(action)))
                .content(label)
        };
        let mut rows = Vec::new();
        if overview {
            rows.extend(
                self.view
                    .clients
                    .iter()
                    .filter(|view| view.enabled)
                    .map(|view| (view.route.client as u64, self.overview_card(view, context))),
            );
            if rows.is_empty() {
                rows.push((100, TextBlock::new().text("尚未启用客户端连接").into()));
                rows.push((101, button("连接设置", ClientAction::Settings, true)));
            }
        } else {
            let selected = self.view.selected;
            if self.view.clients.len() > 1 {
                let navigation = StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(8.0)
                    .keyed_children(self.view.clients.iter().map(|view| {
                        let id = view.route.client;
                        (
                            id as u64,
                            button(id.name(), ClientAction::Select(id), Some(id) != selected),
                        )
                    }));
                rows.push((100, navigation));
            }
            if let Some(view) = self
                .view
                .clients
                .iter()
                .find(|view| Some(view.route.client) == selected)
            {
                rows.push((view.route.client as u64, self.details(view, context)));
            }
        }
        ScrollViewer::new().content(
            StackPanel::new()
                .margin(if overview { 12.0 } else { 16.0 })
                .spacing(if overview { 8.0 } else { 16.0 })
                .keyed_children(rows),
        )
    }
}

impl Client {
    fn overview_card(&self, view: &ClientView, context: &ViewContext<Self>) -> View {
        let id = view.route.client;
        let title = if view.connected {
            format!("已连接至{}", id.name())
        } else {
            id.name().to_owned()
        };
        let header = Grid::new()
            .columns([GridLength::STAR, GridLength::Auto])
            .column_spacing(8.0)
            .children((
                TextBlock::new()
                    .text(title)
                    .font_size(17.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .vertical_alignment(VerticalAlignment::Center),
                HyperlinkButton::new()
                    .grid_column(1)
                    .vertical_alignment(VerticalAlignment::Center)
                    .on_click(context.message(Message::Action(ClientAction::Expand(id))))
                    .content("打开聊天"),
            ));
        let plan = view.plan.as_deref().unwrap_or("—");
        let version = view.version.as_deref().unwrap_or("—");
        let metadata = Grid::new()
            .columns([GridLength::STAR, GridLength::Auto])
            .column_spacing(8.0)
            .children((
                TextBlock::new().text(format!("订阅：{plan}")),
                TextBlock::new()
                    .grid_column(1)
                    .text(format!("客户端版本：{version}")),
            ));
        let status = if view.stale {
            Some(format!("{} · 数据已过期", view.status))
        } else if view.plan.is_none() || view.refreshing {
            Some(view.status.clone())
        } else {
            None
        };
        let mut rows = vec![header, metadata];
        if let Some(status) = status {
            rows.push(TextBlock::new().text(status).font_size(12.0).into());
        }
        if id == ClientId::ChatGpt {
            let usage = view.usage.as_ref();
            let quota_heading = match usage {
                Some(CodexUsage::Available { .. }) => TextBlock::new()
                    .text("Codex 额度")
                    .font_weight(FontWeight::SEMI_BOLD),
                _ => TextBlock::new().text(usage_status(usage)),
            };
            let refreshed =
                view.observed_at
                    .and_then(local_clock_label)
                    .map_or_else(View::empty, |time| {
                        TextBlock::new()
                            .grid_column(1)
                            .vertical_alignment(VerticalAlignment::Center)
                            .font_size(12.0)
                            .text(format!("刷新于 {time}"))
                            .into()
                    });
            rows.push(
                Grid::new()
                    .columns([GridLength::STAR, GridLength::Auto])
                    .column_spacing(8.0)
                    .children((quota_heading, refreshed)),
            );
            if let Some(CodexUsage::Available { primary, secondary }) = usage {
                for (window, fallback) in [(primary, "主窗口"), (secondary, "次窗口")] {
                    if let Some(window) = window {
                        rows.push(usage_row(window, fallback));
                    }
                }
            }
        }
        Border::new()
            .background(ThemeBrush::CardBackground)
            .border_brush(ThemeBrush::CardStroke)
            .border_thickness(1.0)
            .corner_radius(8.0)
            .padding(Thickness::new(12.0, 9.0, 12.0, 12.0))
            .content(
                StackPanel::new()
                    .spacing(6.0)
                    .keyed_children(rows.into_iter().enumerate().map(|(i, row)| (i as u64, row))),
            )
    }

    fn details(&self, view: &ClientView, context: &ViewContext<Self>) -> View {
        let text = |label: String| {
            TextBlock::new()
                .text(label)
                .text_wrapping(TextWrapping::Wrap)
        };
        let button = |label, action, enabled| {
            Button::new()
                .is_enabled(enabled)
                .on_click(context.message(Message::Action(action)))
                .content(label)
        };
        let route = view.route;
        let mut rows: Vec<View> = vec![
            TextBlock::new()
                .text(route.client.name())
                .font_size(20.0)
                .into(),
            text(view.status.clone()).into(),
        ];
        if let Some(desktop) = &view.desktop {
            rows.push(text(desktop.clone()).into());
        }
        if view.enabled {
            let plan = view.plan.as_deref().unwrap_or("—");
            let freshness = if view.stale { "（已过期）" } else { "" };
            let plan_freshness = if view.plan.is_some() { freshness } else { "" };
            rows.push(text(format!("订阅：{plan}{plan_freshness}")).into());
            if route.client == ClientId::ChatGpt {
                match &view.usage {
                    Some(CodexUsage::Available { primary, secondary }) => {
                        rows.push(text(format!("Codex 额度{freshness}")).into());
                        for (window, fallback) in [(primary, "主窗口"), (secondary, "次窗口")]
                        {
                            if let Some(window) = window {
                                rows.push(text(usage_label(window, fallback)).into());
                            }
                        }
                    }
                    usage => rows.push(text(usage_status(usage.as_ref()).into()).into()),
                }
            }
            rows.push(text(route.client.source_label().into()).into());
            if let Some(time) = view.observed_at.and_then(utc_label) {
                rows.push(text(format!("本地获取时间：{time}")).into());
            }
        }
        let actions = (
            if view.enabled {
                button("刷新", ClientAction::Refresh(route), view.can_refresh)
            } else {
                View::empty()
            },
            if view.enabled {
                button("清除缓存", ClientAction::Clear(route), view.can_clear)
            } else {
                View::empty()
            },
            button("连接设置", ClientAction::Settings, true),
        );
        rows.push(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children(actions),
        );
        StackPanel::new()
            .spacing(6.0)
            .keyed_children(rows.into_iter().enumerate().map(|(i, row)| (i as u64, row)))
    }
}

fn usage_status(usage: Option<&CodexUsage>) -> &'static str {
    match usage {
        Some(CodexUsage::Failed {
            error: QueryError::TimedOut,
        }) => "Codex 额度：查询超时",
        Some(CodexUsage::Failed {
            error: QueryError::InvalidResponse,
        }) => "Codex 额度：查询响应不受支持",
        Some(CodexUsage::Failed { .. }) => "Codex 额度：暂不可用",
        Some(CodexUsage::Unavailable) => "Codex 额度：未返回额度窗口",
        _ => "Codex 额度：—",
    }
}

fn usage_period(window: &UsageWindow, fallback: &str) -> String {
    match window.window_duration_mins {
        Some(minutes) if minutes % 1440 == 0 => format!("{} 天", minutes / 1440),
        Some(minutes) if minutes % 60 == 0 => format!("{} 小时", minutes / 60),
        Some(minutes) => format!("{minutes} 分钟"),
        None => fallback.into(),
    }
}

fn usage_row(window: &UsageWindow, fallback: &str) -> View {
    let period = usage_period(window, fallback);
    let remaining = 100 - window.used_percent.clamp(0, 100);
    let reset = window.resets_at.and_then(local_reset_label);
    let summary = match reset {
        Some(time) => format!("剩余 {remaining}% · {time} 重置"),
        None => format!("剩余 {remaining}% · 重置时间未知"),
    };
    Grid::new()
        .columns([GridLength::Auto, GridLength::STAR, GridLength::Auto])
        .column_spacing(8.0)
        .children((
            TextBlock::new()
                .min_width(48.0)
                .font_size(12.0)
                .vertical_alignment(VerticalAlignment::Center)
                .text(period.clone()),
            ProgressBar::new()
                .grid_column(1)
                .vertical_alignment(VerticalAlignment::Center)
                .automation_name(format!("{period}剩余额度"))
                .value(f64::from(remaining)),
            TextBlock::new()
                .grid_column(2)
                .font_size(12.0)
                .vertical_alignment(VerticalAlignment::Center)
                .text(summary),
        ))
}

fn usage_label(window: &UsageWindow, fallback: &str) -> String {
    let period = usage_period(window, fallback);
    let reset = window
        .resets_at
        .and_then(|seconds| u64::try_from(seconds).ok())
        .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
        .and_then(utc_label)
        .map_or_else(
            || "重置时间未提供".to_owned(),
            |time| format!("重置 {time}"),
        );
    format!("{period}：已用 {}% · {reset}", window.used_percent)
}

const HUB_SUBCLASS: usize = 1;

unsafe extern "system" fn remember_position(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    // The subclass runs on the hub's UI thread. Coordinates belong to this window.
    unsafe {
        if matches!(message, WM_CLOSE | WM_EXITSIZEMOVE)
            && !IsIconic(hwnd).as_bool()
            && !IsZoomed(hwnd).as_bool()
        {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                let _ = send(
                    io::stdout().lock(),
                    &ToPet::HubPosition([rect.left, rect.top]),
                );
            }
        }
        if message == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(remember_position), id);
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

fn restore_position(hwnd: HWND, [x, y]: [i32; 2]) -> windows::core::Result<()> {
    let mut rect = RECT::default();
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetWindowRect(hwnd, &mut rect)?;
        if !GetMonitorInfoW(
            MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            return Err(windows::core::Error::from_win32());
        }
        let work = monitor.rcWork;
        SetWindowPos(
            hwnd,
            None,
            x.clamp(
                work.left,
                (work.right - (rect.right - rect.left)).max(work.left),
            ),
            y.clamp(
                work.top,
                (work.bottom - (rect.bottom - rect.top)).max(work.top),
            ),
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
}
