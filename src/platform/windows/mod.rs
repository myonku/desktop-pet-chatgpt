use std::{
    cell::{Cell, RefCell},
    time::Instant,
};

use windows::{
    Win32::{
        Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Dwm::DwmFlush,
        Graphics::Gdi::{
            ClientToScreen, CreateRectRgn, DeleteObject, ExtCreateRegion, HRGN, RDH_RECTANGLES,
            RGNDATAHEADER, SetWindowRgn,
        },
        System::{
            LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount,
            SystemServices::MK_LBUTTON,
        },
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{
                DefSubclassProc, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
                NIIF_NOSOUND, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
                NOTIFYICON_VERSION_4, NOTIFYICONDATAW, RemoveWindowSubclass, SetWindowSubclass,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};
use winit::{
    dpi::PhysicalPosition,
    event_loop::EventLoopProxy,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

use crate::{
    app::AppEvent,
    media::{PackIcon, frame::FrameView},
};

pub(crate) mod chatgpt;
mod display;
mod panel;
pub(crate) use panel::{
    QuickFollow, QuickStartup, QuickWindow, fix_settings_frame, panel_window,
    position_quick_window, reveal_quick_window,
};
mod preferences;
mod session;
pub use display::{Display, physical_size};
pub use preferences::{Autostart, LegacyPreferences, Sound, legacy_preferences, startup_command};
pub use session::{
    Instance, choose_pack_folder, confirm_config_reset, local_path, reveal_directory,
    show_config_error, user_profile,
};

#[derive(Debug)]
pub enum Gesture {
    Press,
    Click(Instant, [i32; 2], [u32; 2]),
    DragStarted,
    Drop,
    Cancel,
}

const TRAY_MESSAGE: u32 = WM_APP + 1;
const MENU_TIMER: usize = 0x5054;
const OPEN_MENU: u32 = WM_APP + 3;
const TRAY_ID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const SUBCLASS_ID: usize = 1;
const APP_ICON_RESOURCE: u16 = 101;
const TRAY_ICON_RESOURCE: u16 = 102;

pub enum Command {
    TogglePassthrough,
    ToggleVisibility,
    ToggleAuto,
    ToggleSound,
    ToggleFullscreen,
    OpenSettings,
    OpenHub,
    Exit,
}

pub struct MenuState {
    pub passthrough: bool,
    pub visible: bool,
    pub auto_move: bool,
    pub sound: bool,
    pub fullscreen_hide: bool,
}

struct MenuRequest {
    position: PhysicalPosition<i32>,
    state: MenuState,
}
struct Callback {
    proxy: EventLoopProxy<AppEvent>,
    taskbar_created: u32,
    drag: Cell<Option<Drag>>,
    edge_frame: Cell<bool>,
    trace_input: bool,
    menu: RefCell<Option<MenuRequest>>,
    menu_open: Cell<bool>,
}

pub struct WindowInput {
    hwnd: HWND,
    _callback: Box<Callback>,
    tray: bool,
    tray_icon: Option<OwnedIcon>,
    instance_marker: Vec<u16>,
    hit_bounds: Option<(i32, i32, i32, i32)>,
}

impl WindowInput {
    pub fn new(
        window: &Window,
        proxy: EventLoopProxy<AppEvent>,
        trace_input: bool,
        instance_marker: &[u16],
        tray_icon: Option<PackIcon>,
    ) -> Result<Self, String> {
        let handle = window.window_handle().map_err(|error| error.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("expected a Win32 window".into());
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        // The callback is installed and removed on the window's event thread.
        let callback = Box::new(Callback {
            proxy,
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            drag: Cell::new(None),
            edge_frame: Cell::new(false),
            trace_input,
            menu: RefCell::new(None),
            menu_open: Cell::new(false),
        });
        if !unsafe {
            SetWindowSubclass(
                hwnd,
                Some(window_proc),
                SUBCLASS_ID,
                &*callback as *const Callback as usize,
            )
        }
        .as_bool()
        {
            return Err("could not install window input callback".into());
        }
        let mut input = Self {
            hwnd,
            _callback: callback,
            tray: false,
            tray_icon: custom_tray_icon(hwnd, tray_icon.as_ref()),
            instance_marker: instance_marker.to_vec(),
            hit_bounds: None,
        };
        // The marker lets another launch find this exact profile's window.
        unsafe {
            SetPropW(
                hwnd,
                PCWSTR(instance_marker.as_ptr()),
                HANDLE((&*input._callback as *const Callback).cast_mut().cast()),
            )
            .map_err(|error| error.to_string())?;
        }
        input.add_tray();
        Ok(input)
    }

    pub fn display(&self) -> Option<Display> {
        Display::for_window(self.hwnd)
    }

    pub fn fullscreen(&self, display: Display) -> bool {
        display.has_fullscreen(self.hwnd)
    }

    pub fn cancel_gesture(&self) {
        self._callback.drag.set(None);
    }

    pub fn gesture_in_progress(&self) -> bool {
        self._callback.drag.get().is_some()
    }

    pub fn edge_frame_presented(&self) -> bool {
        self._callback.edge_frame.get()
    }

    pub fn frame_presented(&self, edge: bool, drag_ready: bool) {
        self._callback.edge_frame.set(edge);
        if drag_ready
            && let Some(mut drag) = self._callback.drag.get()
            && let Some((cursor, started)) = drag.deferred.take()
        {
            // Let the compositor pick up the new pose before moving the HWND.
            let _ = unsafe { DwmFlush() };
            self._callback.drag.set((!drag.released).then_some(drag));
            drag.move_window(self.hwnd, cursor, started);
            if drag.released {
                let _ = self
                    ._callback
                    .proxy
                    .send_event(AppEvent::Gesture(Gesture::Drop));
            }
        }
    }

    pub fn settings_error(&self, message: &str) {
        let text: Vec<_> = message.encode_utf16().chain([0]).collect();
        unsafe {
            MessageBoxW(
                self.hwnd,
                PCWSTR(text.as_ptr()),
                w!("桌宠设置"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    pub fn tray_notice(&self, title: &str, message: &str) -> bool {
        if !self.tray {
            return false;
        }
        let mut data = tray_data(self.hwnd);
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_NOSOUND;
        for (slot, unit) in data
            .szInfoTitle
            .iter_mut()
            .take(63)
            .zip(title.encode_utf16())
        {
            *slot = unit;
        }
        for (slot, unit) in data.szInfo.iter_mut().take(255).zip(message.encode_utf16()) {
            *slot = unit;
        }
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool()
    }

    pub fn has_tray(&self) -> bool {
        self.tray
    }

    pub fn replace_tray_icon(&mut self, icon: Option<&PackIcon>) {
        self.tray_icon = custom_tray_icon(self.hwnd, icon);
        if self.tray {
            let mut data = tray_data(self.hwnd);
            data.uFlags = NIF_ICON;
            data.hIcon = match tray_icon_handle(self.hwnd, self.tray_icon.as_ref()) {
                Some((handle, _)) => handle,
                None => return,
            };
            if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
                eprintln!("Could not update tray icon after changing packs");
            }
        }
    }

    pub fn add_tray(&mut self) {
        unsafe {
            let mut data = tray_data(self.hwnd);
            if self.tray {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            let result = (|| {
                let (icon, source) = tray_icon_handle(self.hwnd, self.tray_icon.as_ref())?;
                data.hIcon = icon;
                data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
                data.uCallbackMessage = TRAY_MESSAGE;
                for (slot, ch) in data
                    .szTip
                    .iter_mut()
                    .zip("Desktop Pet ChatGPT".encode_utf16())
                {
                    *slot = ch;
                }
                if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                    eprintln!("Shell_NotifyIconW(NIM_ADD) failed");
                    return None;
                }
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                if Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool() {
                    eprintln!("Tray icon: {source}");
                    return Some(());
                }
                eprintln!("Shell_NotifyIconW(NIM_SETVERSION) failed");
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                None
            })();
            self.tray = result.is_some();
        }
        if !self.tray {
            eprintln!("Tray unavailable; use the role quick panel to exit");
        }
    }

    pub fn menu(
        &self,
        position: PhysicalPosition<i32>,
        state: MenuState,
    ) -> windows::core::Result<()> {
        let callback = &self._callback;
        if callback.menu_open.get() || callback.menu.borrow().is_some() {
            return Ok(());
        }
        callback.menu.replace(Some(MenuRequest { position, state }));
        // Dispatch after returning from the winit handler so modal messages can reach it.
        let result = unsafe { PostMessageW(self.hwnd, OPEN_MENU, WPARAM(0), LPARAM(0)) };
        if result.is_err() {
            callback.menu.take();
        }
        result
    }
}

fn track_menu(
    hwnd: HWND,
    position: PhysicalPosition<i32>,
    state: MenuState,
) -> windows::core::Result<Option<Command>> {
    let MenuState {
        passthrough,
        visible,
        auto_move,
        sound,
        fullscreen_hide,
    } = state;
    // The popup is owned by this HWND and dispatched on its event thread.
    unsafe {
        let menu = Menu(CreatePopupMenu()?);
        let check = if passthrough {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        AppendMenuW(menu.0, MF_STRING | check, 1, w!("全窗口鼠标穿透(&P)"))?;
        AppendMenuW(
            menu.0,
            MF_STRING,
            4,
            if visible {
                w!("隐藏桌宠(&H)")
            } else {
                w!("显示桌宠(&H)")
            },
        )?;
        let checked = |on| if on { MF_CHECKED } else { MF_UNCHECKED };
        AppendMenuW(menu.0, MF_STRING | checked(auto_move), 5, w!("自移动(&M)"))?;
        AppendMenuW(menu.0, MF_STRING | checked(sound), 9, w!("音效(&U)"))?;
        AppendMenuW(
            menu.0,
            MF_STRING | checked(fullscreen_hide),
            10,
            w!("全屏隐藏(&F)"),
        )?;
        AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu.0, MF_STRING, 11, w!("聊天窗口(&C)"))?;
        AppendMenuW(menu.0, MF_STRING, 7, w!("设置(&S)"))?;
        AppendMenuW(menu.0, MF_STRING, 2, w!("退出(&X)"))?;
        let previous = GetForegroundWindow();
        let _ = SetForegroundWindow(hwnd);
        let selected = TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            position.x,
            position.y,
            0,
            hwnd,
            None,
        );
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        // Opening a panel needs the foreground permission granted by this native menu.
        if !matches!(selected.0, 7 | 11)
            && !previous.is_invalid()
            && previous != hwnd
            && GetForegroundWindow() == hwnd
        {
            let _ = SetForegroundWindow(previous);
        }
        Ok(match selected.0 {
            1 => Some(Command::TogglePassthrough),
            2 => Some(Command::Exit),
            4 => Some(Command::ToggleVisibility),
            5 => Some(Command::ToggleAuto),
            7 => Some(Command::OpenSettings),
            9 => Some(Command::ToggleSound),
            10 => Some(Command::ToggleFullscreen),
            11 => Some(Command::OpenHub),
            _ => None,
        })
    }
}

impl WindowInput {
    pub fn invalidate_hit_region(&mut self) {
        self.hit_bounds = None;
    }

    pub fn apply_hit_region(&mut self, frame: FrameView<'_>) -> Result<(), String> {
        // The region clips only fully transparent pixels. Coordinates are relative to
        // the HWND, so account for any client offset even on a borderless window.
        unsafe {
            let mut client = RECT::default();
            let mut outer = RECT::default();
            let mut origin = POINT::default();
            GetClientRect(self.hwnd, &mut client).map_err(|error| error.to_string())?;
            if client.right <= 0 || client.bottom <= 0 {
                return Ok(());
            }
            GetWindowRect(self.hwnd, &mut outer).map_err(|error| error.to_string())?;
            if !ClientToScreen(self.hwnd, &mut origin).as_bool() {
                return Err("could not locate the client area".into());
            }
            let offset_x = origin.x - outer.left;
            let offset_y = origin.y - outer.top;
            let bounds = (client.right, client.bottom, offset_x, offset_y);
            if self.hit_bounds == Some(bounds) {
                return Ok(());
            }
            let mut runs = Vec::<RECT>::new();
            let [_, _, width, height] = frame.rect;
            let [left, top, right, bottom] = frame
                .canvas
                .destination([client.right as u32, client.bottom as u32]);
            let span_x = right - left;
            let span_y = bottom - top;
            for (y, row) in frame.rows().enumerate() {
                let row_top = (top + scaled_edge(y as u32, height, span_y)).clamp(0, client.bottom)
                    + offset_y;
                let row_bottom = (top + scaled_edge(y as u32 + 1, height, span_y))
                    .clamp(0, client.bottom)
                    + offset_y;
                if row_top == row_bottom {
                    continue;
                }
                let mut start = None;
                let alpha = row
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| pixel[3])
                    .chain([0]);
                for (x, alpha) in alpha.enumerate() {
                    if alpha != 0 {
                        start.get_or_insert(x as u32);
                    } else if let Some(first) = start.take() {
                        let run_left = (left + scaled_edge(first, width, span_x))
                            .clamp(0, client.right)
                            + offset_x;
                        let run_right = (left + scaled_edge(x as u32, width, span_x))
                            .clamp(0, client.right)
                            + offset_x;
                        if run_left == run_right {
                            continue;
                        }
                        if let Some(last) = runs.last_mut()
                            && last.top == row_top
                            && last.right == run_left
                        {
                            last.right = run_right;
                        } else {
                            runs.push(RECT {
                                left: run_left,
                                top: row_top,
                                right: run_right,
                                bottom: row_bottom,
                            });
                        }
                    }
                }
            }
            let region = Region::from_runs(&runs)?;
            if SetWindowRgn(self.hwnd, region.0, true) == 0 {
                return Err("could not set the alpha hit region".into());
            }
            // Windows owns the region after SetWindowRgn succeeds.
            std::mem::forget(region);
            self.hit_bounds = Some(bounds);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Drag {
    start: POINT,
    grab_x: i32,
    grab_y: i32,
    dpi: u32,
    x_threshold: u32,
    y_threshold: u32,
    moving: bool,
    deferred: Option<(POINT, Option<u32>)>,
    released: bool,
}

impl Drag {
    fn start(hwnd: HWND, position: LPARAM) -> Option<Self> {
        unsafe {
            let mut start = message_point(position.0 as usize);
            let mut outer = RECT::default();
            if !ClientToScreen(hwnd, &mut start).as_bool() {
                return None;
            }
            GetWindowRect(hwnd, &mut outer).ok()?;
            let dpi = GetDpiForWindow(hwnd).max(1);
            Some(Self {
                start,
                grab_x: start.x - outer.left,
                grab_y: start.y - outer.top,
                dpi,
                x_threshold: GetSystemMetricsForDpi(SM_CXDRAG, dpi).max(1) as u32,
                y_threshold: GetSystemMetricsForDpi(SM_CYDRAG, dpi).max(1) as u32,
                moving: false,
                deferred: None,
                released: false,
            })
        }
    }

    fn update(mut self, callback: &Callback, hwnd: HWND, position: LPARAM) {
        let mut cursor = message_point(position.0 as usize);
        // Handle the move while dispatching its native message, before a queued
        // button-up can make the cursor or button state refer to a later event.
        unsafe {
            if !ClientToScreen(hwnd, &mut cursor).as_bool() {
                return;
            }
        }
        let started = !self.moving
            && (cursor.x.abs_diff(self.start.x) > self.x_threshold
                || cursor.y.abs_diff(self.start.y) > self.y_threshold);
        self.moving |= started;
        let message_time = callback
            .trace_input
            .then(|| unsafe { GetMessageTime() as u32 });
        if self.deferred.is_some() || (started && callback.edge_frame.get()) {
            self.deferred = Some((cursor, message_time));
        }
        callback.drag.set(Some(self));
        if started {
            let _ = callback
                .proxy
                .send_event(AppEvent::Gesture(Gesture::DragStarted));
        }
        if self.moving && self.deferred.is_none() {
            self.move_window(hwnd, cursor, message_time);
        }
    }

    fn move_window(self, hwnd: HWND, cursor: POINT, started: Option<u32>) {
        unsafe {
            let dpi = GetDpiForWindow(hwnd).max(1);
            self.place_at_cursor(hwnd, cursor, dpi);
            let current_dpi = GetDpiForWindow(hwnd).max(1);
            if current_dpi != dpi {
                self.place_at_cursor(hwnd, cursor, current_dpi);
            }
            if let Some(started) = started
                && DwmFlush().is_ok()
            {
                let latency = GetTickCount().wrapping_sub(started);
                eprintln!("Drag compositor flush: {latency} ms");
            }
        }
    }

    fn place_at_cursor(self, hwnd: HWND, cursor: POINT, dpi: u32) {
        let scale = f64::from(dpi) / f64::from(self.dpi);
        let x = cursor.x - (f64::from(self.grab_x) * scale).round() as i32;
        let y = cursor.y - (f64::from(self.grab_y) * scale).round() as i32;
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

fn message_point(value: usize) -> POINT {
    POINT {
        x: value as i16 as i32,
        y: (value >> 16) as i16 as i32,
    }
}

fn scaled_edge(source_edge: u32, source_size: u32, target_size: i32) -> i32 {
    // Select the first destination pixel center on or after the source edge.
    let source_size = u64::from(source_size);
    ((2 * u64::from(source_edge) * target_size as u64 + source_size - 1) / (2 * source_size)) as i32
}

impl Drop for WindowInput {
    fn drop(&mut self) {
        unsafe {
            if self.tray {
                let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(self.hwnd));
            }
            let _ = RemovePropW(self.hwnd, PCWSTR(self.instance_marker.as_ptr()));
            let _ = RemoveWindowSubclass(self.hwnd, Some(window_proc), SUBCLASS_ID);
        }
    }
}

fn tray_icon_size(hwnd: HWND) -> i32 {
    unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd)) }.max(16)
}

fn custom_tray_icon(hwnd: HWND, icon: Option<&PackIcon>) -> Option<OwnedIcon> {
    OwnedIcon::from_pack(icon?, tray_icon_size(hwnd))
        .inspect_err(|error| eprintln!("Could not create custom tray icon: {error}"))
        .ok()
}

fn tray_icon_handle(hwnd: HWND, custom: Option<&OwnedIcon>) -> Option<(HICON, &'static str)> {
    if let Some(icon) = custom {
        return Some((icon.0, "custom pack resource"));
    }
    if let Ok(icon) = embedded_icon(TRAY_ICON_RESOURCE, tray_icon_size(hwnd)) {
        return Some((icon, "embedded application resource"));
    }
    unsafe { LoadIconW(HINSTANCE::default(), IDI_APPLICATION) }
        .inspect_err(|error| eprintln!("Could not load tray icon: {error}"))
        .ok()
        .map(|icon| (icon, "Windows system resource"))
}

fn embedded_icon(id: u16, size: i32) -> windows::core::Result<HICON> {
    // Shared resource handles stay owned by the executable module.
    let module = unsafe { GetModuleHandleW(None) }?;
    let handle = unsafe {
        LoadImageW(
            HINSTANCE(module.0),
            PCWSTR(usize::from(id) as *const u16),
            IMAGE_ICON,
            size,
            size,
            LR_SHARED | LR_DEFAULTCOLOR,
        )
    }?;
    Ok(HICON(handle.0))
}

pub(crate) struct WindowIcons {
    // Keep custom HICONs alive until after the owning window is dropped.
    _custom: [OwnedIcon; 2],
}

impl WindowIcons {
    pub(crate) fn install(window: &Window, icon: Option<&PackIcon>) -> Option<Self> {
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return None;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        if let Some(icon) = icon {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let [small, big] = [SM_CXSMICON, SM_CXICON].map(|metric| {
                OwnedIcon::from_pack(icon, unsafe { GetSystemMetricsForDpi(metric, dpi) })
            });
            match (small, big) {
                (Ok(small), Ok(big)) => {
                    set_window_icon(hwnd, ICON_SMALL, small.0);
                    set_window_icon(hwnd, ICON_BIG, big.0);
                    return Some(Self {
                        _custom: [small, big],
                    });
                }
                (Err(error), _) | (_, Err(error)) => {
                    eprintln!("Could not load pack window icon: {error}")
                }
            }
        }
        apply_embedded_panel_icons(hwnd);
        None
    }
}

fn set_window_icon(hwnd: HWND, kind: u32, icon: HICON) {
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(kind as usize),
            LPARAM(icon.0 as isize),
        );
    }
}

pub(crate) fn apply_embedded_panel_icons(hwnd: HWND) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        for (kind, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
            if let Ok(icon) = embedded_icon(APP_ICON_RESOURCE, GetSystemMetricsForDpi(metric, dpi))
            {
                set_window_icon(hwnd, kind, icon);
            }
        }
    }
}

struct OwnedIcon(HICON);

impl OwnedIcon {
    fn from_pack(icon: &PackIcon, size: i32) -> windows::core::Result<Self> {
        let png = icon.png_for_size(size as u32);
        let mut words = vec![0_u32; png.len().div_ceil(4)];
        // Win32 requires DWORD alignment for the input resource bytes.
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), png.len()) };
        bytes.copy_from_slice(png);
        unsafe { CreateIconFromResourceEx(bytes, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR) }
            .map(Self)
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

struct Menu(HMENU);

impl Drop for Menu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

struct Region(HRGN);

impl Region {
    fn from_runs(runs: &[RECT]) -> Result<Self, String> {
        let handle = if let Some(&first) = runs.first() {
            const HEADER_SLOTS: usize = size_of::<RGNDATAHEADER>() / size_of::<RECT>();
            const {
                assert!(size_of::<RGNDATAHEADER>() == HEADER_SLOTS * size_of::<RECT>());
                assert!(align_of::<RECT>() >= align_of::<RGNDATAHEADER>());
            }
            let bytes = u32::try_from(size_of_val(runs) + size_of::<RGNDATAHEADER>())
                .map_err(|_| "alpha hit region exceeds the GDI size limit")?;
            let bounds = runs.iter().fold(first, |bounds, rect| RECT {
                left: bounds.left.min(rect.left),
                top: bounds.top.min(rect.top),
                right: bounds.right.max(rect.right),
                bottom: bounds.bottom.max(rect.bottom),
            });
            let mut data = Vec::with_capacity(HEADER_SLOTS + runs.len());
            data.resize(HEADER_SLOTS, RECT::default());
            data.extend_from_slice(runs);
            // RGNDATA is an aligned header followed by sorted, non-overlapping RECTs.
            unsafe {
                data.as_mut_ptr()
                    .cast::<RGNDATAHEADER>()
                    .write(RGNDATAHEADER {
                        dwSize: size_of::<RGNDATAHEADER>() as u32,
                        iType: RDH_RECTANGLES,
                        nCount: runs.len() as u32,
                        nRgnSize: size_of_val(runs) as u32,
                        rcBound: bounds,
                    });
                ExtCreateRegion(None, bytes, data.as_ptr().cast())
            }
        } else {
            unsafe { CreateRectRgn(0, 0, 0, 0) }
        };
        if handle.is_invalid() {
            return Err("could not allocate an alpha hit region".into());
        }
        Ok(Self(handle))
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0) };
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // The owning WindowInput keeps this allocation alive until subclass removal.
    let callback = unsafe { &*(data as *const Callback) };
    if message == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    match message {
        OPEN_MENU => {
            if let Some(request) = callback.menu.take() {
                callback.menu_open.set(true);
                if unsafe { SetTimer(hwnd, MENU_TIMER, 16, None) } == 0 {
                    eprintln!("Could not schedule playback during the tray menu");
                }
                let result = track_menu(hwnd, request.position, request.state);
                let _ = unsafe { KillTimer(hwnd, MENU_TIMER) };
                callback.menu_open.set(false);
                match result {
                    Ok(Some(command)) => {
                        let _ = callback.proxy.send_event(AppEvent::MenuSelected(command));
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("Could not open menu: {error}"),
                }
            }
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == MENU_TIMER => {
            let _ = callback.proxy.send_event(AppEvent::ModalTick);
            return LRESULT(0);
        }
        session::ACTIVATE_MESSAGE => {
            let _ = callback.proxy.send_event(AppEvent::OpenSettings);
            return LRESULT(0);
        }
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => {
            let _ = callback.proxy.send_event(AppEvent::ResumeDisplay);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if callback.drag.get().is_some_and(|drag| drag.released) {
                let _ = callback
                    .proxy
                    .send_event(AppEvent::Gesture(Gesture::Cancel));
            }
            callback.drag.set(Drag::start(hwnd, lparam));
            let _ = callback.proxy.send_event(AppEvent::Gesture(Gesture::Press));
        }
        WM_MOUSEMOVE if wparam.0 & MK_LBUTTON.0 as usize != 0 => {
            if let Some(drag) = callback.drag.get().filter(|drag| !drag.released) {
                drag.update(callback, hwnd, lparam);
            }
        }
        WM_LBUTTONUP => {
            if let Some(mut drag) = callback.drag.take() {
                if drag.deferred.is_some() {
                    drag.released = true;
                    callback.drag.set(Some(drag));
                } else {
                    let gesture = if drag.moving {
                        Gesture::Drop
                    } else {
                        let tolerance = unsafe {
                            [
                                GetSystemMetricsForDpi(SM_CXDOUBLECLK, drag.dpi) as u32 / 2,
                                GetSystemMetricsForDpi(SM_CYDOUBLECLK, drag.dpi) as u32 / 2,
                            ]
                        };
                        Gesture::Click(Instant::now(), [drag.start.x, drag.start.y], tolerance)
                    };
                    let _ = callback.proxy.send_event(AppEvent::Gesture(gesture));
                }
            }
        }
        WM_MOUSEMOVE | WM_CAPTURECHANGED
            if callback.drag.get().is_some_and(|drag| drag.released) => {}
        WM_MOUSEMOVE | WM_CANCELMODE | WM_CAPTURECHANGED if callback.drag.take().is_some() => {
            let _ = callback
                .proxy
                .send_event(AppEvent::Gesture(Gesture::Cancel));
        }
        _ => {}
    }
    let event = if message == TRAY_MESSAGE && ((lparam.0 >> 16) as u16) == TRAY_ID as u16 {
        let notification = lparam.0 as u16 as u32;
        if matches!(notification, WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT) {
            let point = message_point(wparam.0);
            Some(AppEvent::TrayMenu(PhysicalPosition::new(point.x, point.y)))
        } else {
            None
        }
    } else if message == callback.taskbar_created && message != 0 {
        Some(AppEvent::TaskbarCreated)
    } else {
        None
    };
    if let Some(event) = event {
        let _ = callback.proxy.send_event(event);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests;
