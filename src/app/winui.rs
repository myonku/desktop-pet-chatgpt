use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Write},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::mpsc::{self, SyncSender},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetWindowThreadProcessId, IsWindow, SW_RESTORE, SetForegroundWindow,
    ShowWindowAsync,
};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::core::{
    preferences::SIZE_RANGE,
    settings::{SettingsAction, SettingsDraft},
};

use super::{
    App, AppEvent,
    behavior::ActionPreview,
    clients::{ClientId, ClientView, SourceRoute},
};
use crate::integrations::chatgpt::CodexUsage;
use crate::platform::windows::{Command, QuickFollow};

const TITLE: &str = "设置";
const QUICK_TITLE: &str = "快捷面板";
const QUICK_WIDTH: i32 = 124;
const QUICK_ROW_HEIGHT: i32 = 34;
const QUICK_ROW_GAP: i32 = 4;
const QUICK_MARGIN: i32 = 6;
const OVERVIEW_TITLE: &str = "信息速览";
const HUB_TITLE: &str = "聊天窗口";
const OVERVIEW_WIDTH: i32 = 388;
const STARTUP_LIMIT: Duration = Duration::from_secs(5);
const WRITE_LIMIT: Duration = Duration::from_secs(5);
const WRITE_QUEUE: usize = 8;

fn quick_height(docked: bool, menu: bool) -> i32 {
    let rows = 4 + i32::from(docked);
    // Leave room for WinUI's per-row rounding at fractional display scales.
    let rounding_space = 4;
    rows * QUICK_ROW_HEIGHT
        + (rows - 1) * QUICK_ROW_GAP
        + 2 * QUICK_MARGIN
        + rounding_space
        + i32::from(menu) * 156
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Snapshot {
    draft: SettingsDraft,
    name: String,
    pack_status: Option<String>,
    development: bool,
    autostart: bool,
    tray: bool,
    sound: bool,
    diagonal_available: bool,
    notice: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct PackPresentation {
    name: String,
    status: Option<String>,
    sound: bool,
    diagonal_available: bool,
    poster: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct QuickSnapshot {
    parent_pid: u32,
    pet_window: isize,
    values: QuickValues,
    tray: bool,
    actions: Vec<(String, bool)>,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
struct QuickValues {
    scale: u16,
    auto_move: bool,
    sound: bool,
    fullscreen_hide: bool,
    docked: bool,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum QuickAction {
    AutoMove(bool),
    Sound(bool),
    FullscreenHide(bool),
    Scale(i16),
    Expand,
    Preview(usize),
    Settings,
    Hub,
    Exit,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum ClientAction {
    Refresh(SourceRoute),
    Clear(SourceRoute),
    Settings,
    Expand(ClientId),
    Select(ClientId),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct ClientPanels {
    clients: Vec<ClientView>,
    selected: Option<ClientId>,
}

impl ClientPanels {
    fn overview_height(&self) -> i32 {
        let mut height = 16;
        for view in self.clients.iter().filter(|view| view.enabled) {
            height += 120;
            if view.route.client == ClientId::ChatGpt {
                height += 26;
                if let Some(CodexUsage::Available { primary, secondary }) = &view.usage {
                    height += (i32::from(primary.is_some()) + i32::from(secondary.is_some())) * 24;
                }
            }
        }
        height.clamp(112, 440)
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct ClientSnapshot {
    parent_pid: u32,
    anchor: Option<isize>,
    position: Option<[i32; 2]>,
    view: ClientPanels,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelKind {
    Settings,
    Quick,
    Overview,
    Hub,
}

impl PanelKind {
    fn name(self) -> &'static str {
        match self {
            Self::Settings => "设置中心",
            Self::Quick => "快捷面板",
            Self::Overview => "信息速览",
            Self::Hub => HUB_TITLE,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum ToPanel {
    Open(Snapshot),
    Quick(QuickSnapshot),
    Client(ClientSnapshot),
    ClientValues(ClientPanels),
    QuickValues(QuickValues),
    Notice(String),
    Pack(PackPresentation),
    Reset(SettingsDraft),
    Committed(SettingsDraft),
    Saved {
        submitted: SettingsDraft,
        current: SettingsDraft,
    },
    Conflict(String),
}

#[derive(Serialize, Deserialize)]
enum ToPet {
    Ready(isize),
    Action(SettingsAction, SettingsDraft, SettingsDraft),
    QuickAction(QuickAction),
    QuickMenu(bool),
    ClientAction(ClientAction),
    HubPosition([i32; 2]),
    Closed,
}

pub(crate) struct PanelEvent {
    session: u64,
    message: PanelMessage,
}

enum PanelMessage {
    Received(ToPet),
    Closed,
    Written,
    WriteFailed,
}

pub(super) struct Panel {
    session: u64,
    kind: PanelKind,
    child: Child,
    outgoing: SyncSender<ToPanel>,
    writes: VecDeque<Instant>,
    write_failed: bool,
    pub(super) autostart: Option<bool>,
    startup_deadline: Option<Instant>,
    window: Option<HWND>,
    follow: QuickFollow,
    quick_actions: Vec<ActionPreview>,
    quick_menu: bool,
    pub(super) pending_draft: Option<(SettingsDraft, SettingsDraft)>,
    client_view: Option<ClientPanels>,
}

fn send(mut output: impl Write, message: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    output.write_all(&line)?;
    output.flush()
}

fn receive<T: DeserializeOwned>(line: &str) -> io::Result<T> {
    serde_json::from_str(line).map_err(io::Error::other)
}

impl Panel {
    fn open(
        session: u64,
        message: ToPanel,
        autostart: Option<bool>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> io::Result<Self> {
        let mut child = ProcessCommand::new(std::env::current_exe()?)
            .arg("--panel-child")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let output = child.stdout.take().unwrap();
        let mut input = child.stdin.take().unwrap();
        let (kind, client_view) = match &message {
            ToPanel::Quick(_) => (PanelKind::Quick, None),
            ToPanel::Client(snapshot) if snapshot.anchor.is_some() => {
                (PanelKind::Overview, Some(snapshot.view.clone()))
            }
            ToPanel::Client(snapshot) => (PanelKind::Hub, Some(snapshot.view.clone())),
            _ => (PanelKind::Settings, None),
        };
        let (outgoing, outgoing_messages) = mpsc::sync_channel(WRITE_QUEUE);
        let mut panel = Self {
            outgoing,
            writes: VecDeque::new(),
            write_failed: false,
            child,
            session,
            kind,
            autostart,
            startup_deadline: Some(Instant::now() + STARTUP_LIMIT),
            window: None,
            follow: QuickFollow::default(),
            quick_actions: Vec::new(),
            quick_menu: false,
            pending_draft: None,
            client_view,
        };
        // Transfer foreground permission before the child shows its window.
        unsafe {
            let _ = AllowSetForegroundWindow(panel.child.id());
        }
        let writer_proxy = proxy.clone();
        std::thread::Builder::new()
            .name("winui-panel-writer".into())
            .spawn(move || {
                for message in outgoing_messages {
                    let sent = send(&mut input, &message);
                    if let Err(error) = &sent {
                        eprintln!("WinUI panel write failed: {error}");
                    }
                    let completion = if sent.is_ok() {
                        PanelMessage::Written
                    } else {
                        PanelMessage::WriteFailed
                    };
                    if writer_proxy
                        .send_event(AppEvent::Winui(PanelEvent {
                            session,
                            message: completion,
                        }))
                        .is_err()
                        || sent.is_err()
                    {
                        break;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("winui-panel-reader".into())
            .spawn(move || {
                for line in BufReader::new(output).lines().map_while(Result::ok) {
                    let message = match receive(&line) {
                        Ok(message) => message,
                        Err(error) => {
                            eprintln!("WinUI panel message rejected: {error}");
                            break;
                        }
                    };
                    if proxy
                        .send_event(AppEvent::Winui(PanelEvent {
                            session,
                            message: PanelMessage::Received(message),
                        }))
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = proxy.send_event(AppEvent::Winui(PanelEvent {
                    session,
                    message: PanelMessage::Closed,
                }));
            })?;
        panel.send(message);
        Ok(panel)
    }

    fn accept_window(&mut self, raw: isize) -> bool {
        let hwnd = HWND(raw as *mut _);
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner != self.child.id() {
            return false;
        }
        self.window = Some(hwnd);
        self.startup_deadline = None;
        true
    }

    fn window(&self) -> Option<HWND> {
        self.window.filter(|window| {
            let mut owner = 0;
            unsafe { GetWindowThreadProcessId(*window, Some(&mut owner)) };
            owner == self.child.id() && unsafe { IsWindow(*window) }.as_bool()
        })
    }

    pub(super) fn alive(&self) -> bool {
        self.startup_deadline.is_some() || self.window().is_some()
    }

    fn focus(&self) {
        if let Some(window) = self.window() {
            unsafe {
                let _ = ShowWindowAsync(window, SW_RESTORE);
                let _ = SetForegroundWindow(window);
            }
        }
    }

    fn send(&mut self, message: ToPanel) {
        if self.write_failed {
            return;
        }
        if let Err(error) = self.outgoing.try_send(message) {
            eprintln!("WinUI panel write queue unavailable: {error}");
            self.write_failed = true;
        } else {
            self.writes.push_back(Instant::now());
        }
    }

    fn write_deadline(&self) -> Option<Instant> {
        self.writes.front().map(|at| *at + WRITE_LIMIT)
    }

    fn deadline(&self) -> Option<Instant> {
        self.startup_deadline
            .into_iter()
            .chain(self.write_deadline())
            .min()
    }

    pub(super) fn notify_committed(&mut self, draft: SettingsDraft) {
        self.send(ToPanel::Committed(draft));
    }

    pub(super) fn saved(&mut self, submitted: SettingsDraft, current: SettingsDraft) {
        self.pending_draft = None;
        self.send(ToPanel::Saved { submitted, current });
    }

    pub(super) fn conflict(&mut self, message: String) {
        self.send(ToPanel::Conflict(message));
    }

    pub(super) fn notice(&mut self, text: String) {
        self.send(ToPanel::Notice(text));
    }

    pub(super) fn reset(&mut self, draft: SettingsDraft) {
        self.autostart = self.autostart.map(|_| false);
        self.pending_draft = None;
        self.send(ToPanel::Reset(draft));
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        // Termination closes the read end and releases any blocked writer.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl App {
    pub(super) fn open_winui(
        &mut self,
        draft: SettingsDraft,
        autostart: Option<bool>,
        notice: Option<String>,
    ) {
        if let Some(panel) = &mut self.settings {
            panel.focus();
            if let Some(notice) = notice {
                panel.notice(notice);
            }
            return;
        }
        let scene = self.scene.as_ref().unwrap();
        let snapshot = Snapshot {
            draft,
            name: scene.assets.name.clone().unwrap_or_else(|| "桌宠".into()),
            pack_status: scene.pack_status.clone(),
            development: self.options.startup.development,
            autostart: autostart.is_some(),
            tray: scene.input.has_tray(),
            sound: scene.sound.available(),
            diagonal_available: scene.playback.has_diagonal(),
            notice: notice.unwrap_or_default(),
        };
        match self.spawn_panel(ToPanel::Open(snapshot), autostart) {
            Ok(panel) => self.settings = Some(panel),
            Err(error) => {
                eprintln!("Could not start WinUI settings: {error}");
                self.panel_error(PanelKind::Settings);
            }
        }
    }

    fn spawn_panel(&mut self, message: ToPanel, autostart: Option<bool>) -> io::Result<Panel> {
        self.next_panel_session += 1;
        Panel::open(
            self.next_panel_session,
            message,
            autostart,
            self.proxy.clone(),
        )
    }

    pub(super) fn toggle_quick(&mut self) {
        if self
            .quick
            .take()
            .is_some_and(|panel| panel.kind == PanelKind::Quick && panel.alive())
        {
            return;
        }
        let scene = self.scene.as_mut().unwrap();
        scene.pet.cancel_click();
        let (ids, actions) = scene.action_previews();
        let Ok(handle) = scene.window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let tray = scene.input.has_tray();
        let snapshot = QuickSnapshot {
            parent_pid: std::process::id(),
            pet_window: handle.hwnd.get(),
            values: self.quick_values(),
            tray,
            actions,
        };
        match self.spawn_panel(ToPanel::Quick(snapshot), None) {
            Ok(mut panel) => {
                panel.quick_actions = ids;
                self.quick = Some(panel);
            }
            Err(error) => {
                eprintln!("Could not start WinUI quick panel: {error}");
                self.panel_error(PanelKind::Quick);
            }
        }
    }

    pub(super) fn close_quick(&mut self) {
        self.quick = None;
    }

    pub(super) fn overview_open(&self) -> bool {
        self.quick
            .as_ref()
            .is_some_and(|panel| panel.kind == PanelKind::Overview && panel.alive())
    }

    pub(super) fn toggle_overview(&mut self) {
        if self
            .quick
            .take()
            .is_some_and(|panel| panel.kind == PanelKind::Overview && panel.alive())
        {
            return;
        }
        let Some(scene) = self
            .scene
            .as_ref()
            .filter(|scene| scene.visible && !scene.passthrough && !scene.minimized)
        else {
            return;
        };
        let Ok(handle) = scene.window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(pet) = handle.as_raw() else {
            return;
        };
        let snapshot = ClientSnapshot {
            parent_pid: std::process::id(),
            anchor: Some(pet.hwnd.get()),
            position: None,
            view: ClientPanels {
                clients: self.clients.views(Instant::now()),
                selected: None,
            },
        };
        match self.spawn_panel(ToPanel::Client(snapshot), None) {
            Ok(panel) => self.quick = Some(panel),
            Err(error) => {
                eprintln!("Could not start WinUI overview: {error}");
                self.panel_error(PanelKind::Overview);
            }
        }
    }

    pub(super) fn open_hub(&mut self) {
        self.close_quick();
        self.refresh_client_panels(Instant::now());
        if let Some(panel) = self.hub.as_ref().filter(|panel| panel.alive()) {
            panel.focus();
            return;
        }
        self.hub = None;
        let snapshot = ClientSnapshot {
            parent_pid: std::process::id(),
            anchor: None,
            position: self.hub_position,
            view: ClientPanels {
                clients: self.clients.views(Instant::now()),
                selected: Some(self.hub_client),
            },
        };
        match self.spawn_panel(ToPanel::Client(snapshot), None) {
            Ok(panel) => self.hub = Some(panel),
            Err(error) => {
                eprintln!("Could not start WinUI hub: {error}");
                self.panel_error(PanelKind::Hub);
            }
        }
    }

    pub(super) fn refresh_client_panels(&mut self, now: Instant) {
        let mut panels = [&mut self.quick, &mut self.hub]
            .into_iter()
            .filter_map(Option::as_mut)
            .filter(|panel| panel.client_view.is_some())
            .peekable();
        if panels.peek().is_none() {
            return;
        }
        let clients = self.clients.views(now);
        let mut resize_overview = false;
        for panel in panels {
            let view = ClientPanels {
                clients: clients.clone(),
                selected: (panel.kind == PanelKind::Hub).then_some(self.hub_client),
            };
            if panel.client_view.as_ref() != Some(&view) {
                resize_overview |= panel.kind == PanelKind::Overview
                    && panel.client_view.as_ref().unwrap().overview_height()
                        != view.overview_height();
                panel.send(ToPanel::ClientValues(view.clone()));
                panel.client_view = Some(view);
            }
        }
        if resize_overview {
            self.anchor_quick();
        }
    }

    pub(super) fn anchor_quick(&mut self) {
        if let Some(panel) = &mut self.quick {
            panel.follow.reanchor = true;
        }
    }

    fn follow_quick(&mut self, now: Instant) -> Option<Instant> {
        let panel = self
            .quick
            .as_mut()
            .filter(|panel| panel.follow.is_pending())?;
        let hwnd = panel.window()?;
        let scene = self.scene.as_ref()?;
        let handle = scene.window.window_handle().ok()?;
        let RawWindowHandle::Win32(pet) = handle.as_raw() else {
            return None;
        };
        let (width, height) = if panel.kind == PanelKind::Overview {
            (
                OVERVIEW_WIDTH,
                panel.client_view.as_ref().unwrap().overview_height(),
            )
        } else {
            (
                QUICK_WIDTH,
                quick_height(scene.pet.edge.is_some(), panel.quick_menu),
            )
        };
        match panel
            .follow
            .tick(hwnd, HWND(pet.hwnd.get() as _), width, height, now)
        {
            Ok(deadline) => deadline,
            Err(error) => {
                eprintln!("Could not position quick panel: {error}");
                panel.follow = QuickFollow::default();
                None
            }
        }
    }

    pub(super) fn handle_winui(&mut self, event_loop: &ActiveEventLoop, event: PanelEvent) {
        let Some(panel) = [&mut self.quick, &mut self.settings, &mut self.hub]
            .into_iter()
            .filter_map(Option::as_mut)
            .find(|panel| panel.session == event.session)
        else {
            return;
        };
        let kind = panel.kind;
        match event.message {
            PanelMessage::Written => {
                panel.writes.pop_front();
            }
            PanelMessage::WriteFailed => panel.write_failed = true,
            PanelMessage::Closed | PanelMessage::Received(ToPet::Closed) => {
                let starting = panel.startup_deadline.is_some();
                self.close_panel(kind);
                if starting {
                    self.panel_error(kind);
                }
            }
            PanelMessage::Received(ToPet::Ready(hwnd)) => {
                if !panel.accept_window(hwnd) {
                    return;
                }
                match kind {
                    PanelKind::Quick => self.refresh_quick_values(),
                    PanelKind::Overview => self.anchor_quick(),
                    PanelKind::Settings => self.notify_panel_pack(),
                    PanelKind::Hub => {}
                }
                if self.options.trace_playback {
                    eprintln!(
                        "WinUI {} ready",
                        match kind {
                            PanelKind::Settings => "settings",
                            PanelKind::Quick => "quick panel",
                            PanelKind::Overview => "overview",
                            PanelKind::Hub => "hub",
                        }
                    );
                }
            }
            PanelMessage::Received(ToPet::Action(action, draft, base))
                if kind == PanelKind::Settings =>
            {
                if !SIZE_RANGE.contains(&draft.scale_percent)
                    || !(50..=150).contains(&draft.preferences.bubble_scale_percent)
                {
                    self.settings_notice("尺寸超出设置范围。".into());
                    return;
                }
                self.apply_settings_action(event_loop, action, &draft, &base);
            }
            PanelMessage::Received(ToPet::QuickAction(action)) if kind == PanelKind::Quick => {
                match action {
                    QuickAction::Settings => {
                        self.open_settings(None);
                        self.close_quick();
                    }
                    QuickAction::Hub => self.open_hub(),
                    QuickAction::Exit => event_loop.exit(),
                    QuickAction::Expand => {
                        self.scene.as_mut().unwrap().expand(Instant::now());
                        self.refresh_quick_values();
                    }
                    QuickAction::Preview(index) => {
                        if let Some(id) = panel.quick_actions.get(index) {
                            self.scene.as_mut().unwrap().preview_action(id);
                        }
                        self.refresh_quick_values();
                    }
                    QuickAction::AutoMove(value) => {
                        if value != self.committed.unwrap().preferences.auto_move {
                            self.handle_menu(event_loop, Some(Command::ToggleAuto));
                        }
                    }
                    QuickAction::Sound(value) => {
                        if value != self.committed.unwrap().preferences.sound {
                            self.handle_menu(event_loop, Some(Command::ToggleSound));
                        }
                    }
                    QuickAction::FullscreenHide(value) => {
                        if value != self.committed.unwrap().preferences.fullscreen_hide {
                            self.handle_menu(event_loop, Some(Command::ToggleFullscreen));
                        }
                    }

                    QuickAction::Scale(delta) => {
                        let percent = self
                            .committed
                            .unwrap()
                            .size
                            .percent
                            .saturating_add_signed(delta);
                        self.save_quick_scale(
                            percent.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()),
                        );
                    }
                }
            }
            PanelMessage::Received(ToPet::QuickMenu(expanded)) if kind == PanelKind::Quick => {
                panel.quick_menu = expanded;
                self.anchor_quick();
            }
            PanelMessage::Received(ToPet::ClientAction(action))
                if matches!(kind, PanelKind::Overview | PanelKind::Hub) =>
            {
                match action {
                    ClientAction::Refresh(route) if kind == PanelKind::Hub => {
                        if let Some(client) = self.clients.get_mut(route) {
                            client.refresh(Instant::now(), &self.proxy);
                        }
                    }
                    ClientAction::Clear(route) if kind == PanelKind::Hub => {
                        if let Some(client) = self.clients.get_mut(route) {
                            client.clear();
                        }
                    }
                    ClientAction::Settings => {
                        self.open_settings(None);
                        self.close_quick();
                    }
                    ClientAction::Expand(id)
                        if kind == PanelKind::Overview && self.clients.contains(id) =>
                    {
                        self.hub_client = id;
                        self.open_hub();
                    }
                    ClientAction::Select(id)
                        if kind == PanelKind::Hub && self.clients.contains(id) =>
                    {
                        self.hub_client = id;
                    }
                    _ => {}
                }
                self.refresh_client_panels(Instant::now());
            }
            PanelMessage::Received(ToPet::HubPosition(position)) if kind == PanelKind::Hub => {
                self.hub_position = Some(position);
            }
            _ => {}
        }
    }

    pub(super) fn refresh_quick_values(&mut self) {
        let values = self.quick_values();
        if let Some(panel) = self
            .quick
            .as_mut()
            .filter(|panel| panel.kind == PanelKind::Quick)
        {
            panel.send(ToPanel::QuickValues(values));
            self.anchor_quick();
        }
    }

    fn quick_values(&self) -> QuickValues {
        let runtime = self.committed.unwrap();
        QuickValues {
            scale: runtime.size.percent,
            auto_move: runtime.preferences.auto_move,
            sound: runtime.preferences.sound,
            fullscreen_hide: runtime.preferences.fullscreen_hide,
            docked: self.scene.as_ref().unwrap().pet.edge.is_some(),
        }
    }

    pub(super) fn notify_panel_pack(&mut self) {
        if let Some(panel) = &mut self.settings {
            let scene = self.scene.as_ref().unwrap();
            let poster = scene
                .assets
                .settings_poster()
                .inspect_err(|error| eprintln!("Could not encode settings poster: {error}"))
                .ok()
                .flatten();
            panel.send(ToPanel::Pack(PackPresentation {
                name: scene.assets.name.clone().unwrap_or_else(|| "桌宠".into()),
                status: scene.pack_status.clone(),
                sound: scene.sound.available(),
                diagonal_available: scene.playback.has_diagonal(),
                poster,
            }));
        }
    }

    pub(super) fn panel_deadline(&mut self) -> Option<Instant> {
        let now = Instant::now();
        let failed = [
            self.settings.as_ref(),
            self.quick.as_ref(),
            self.hub.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|panel| {
            let transport =
                panel.write_failed || panel.write_deadline().is_some_and(|at| at <= now);
            (transport || panel.startup_deadline.is_some_and(|at| at <= now))
                .then_some((panel.kind, transport))
        })
        .collect::<Vec<_>>();
        for (kind, transport) in failed {
            self.close_panel(kind);
            if transport {
                self.report_panel_error("面板通信中断，请重新打开。");
            } else {
                self.panel_error(kind);
            }
        }
        let transport = [
            self.settings.as_ref(),
            self.quick.as_ref(),
            self.hub.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(Panel::deadline)
        .min();
        transport.into_iter().chain(self.follow_quick(now)).min()
    }

    fn close_panel(&mut self, kind: PanelKind) {
        match kind {
            PanelKind::Settings => self.close_settings(),
            PanelKind::Quick | PanelKind::Overview => self.close_quick(),
            PanelKind::Hub => self.hub = None,
        }
    }

    fn panel_error(&self, kind: PanelKind) {
        let name = kind.name();
        let message = format!(
            "无法打开{name}。请检查是否已安装本应用所需架构的 Windows App Runtime 2.4 或更新的 2.x 版本，然后重试。"
        );
        self.report_panel_error(&message);
    }

    fn report_panel_error(&self, message: &str) {
        eprintln!("{message}");
        let input = &self.scene.as_ref().unwrap().input;
        if !input.tray_notice("桌宠设置", message) {
            input.settings_error(message);
        }
    }
}

mod client;
mod quick;
mod view;
pub(super) use view::run_child;
