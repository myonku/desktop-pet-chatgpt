use std::cell::Cell;

use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Dwm::DwmFlush,
        Graphics::Gdi::{
            ClientToScreen, CombineRgn, CreateRectRgn, DeleteObject, HRGN, RGN_ERROR, RGN_OR,
            SetWindowRgn,
        },
        System::{SystemInformation::GetTickCount, SystemServices::MK_LBUTTON},
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{
                DefSubclassProc, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
                NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
                RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::w,
};
use winit::{
    dpi::PhysicalPosition,
    event_loop::EventLoopProxy,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

use crate::{app::AppEvent, media::frame::FrameView};

const TRAY_MESSAGE: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const SUBCLASS_ID: usize = 1;

pub enum Command {
    TogglePassthrough,
    OpenPreview,
    ToggleVisibility,
    Exit,
}

struct Callback {
    proxy: EventLoopProxy<AppEvent>,
    taskbar_created: u32,
    drag: Cell<Option<Drag>>,
    trace_input: bool,
}

pub struct WindowInput {
    hwnd: HWND,
    _callback: Box<Callback>,
    tray: bool,
    hit_bounds: Option<(i32, i32, i32, i32)>,
}

impl WindowInput {
    pub fn new(
        window: &Window,
        proxy: EventLoopProxy<AppEvent>,
        trace_input: bool,
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
            trace_input,
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
            hit_bounds: None,
        };
        input.add_tray();
        Ok(input)
    }

    pub fn has_tray(&self) -> bool {
        self.tray
    }

    pub fn add_tray(&mut self) {
        // The predefined system icon is shared and does not need destruction.
        unsafe {
            let mut data = tray_data(self.hwnd);
            if self.tray {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            let result = (|| {
                data.hIcon = LoadIconW(HINSTANCE::default(), IDI_APPLICATION)
                    .map_err(|error| eprintln!("Could not load tray icon: {error}"))
                    .ok()?;
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
                    return Some(());
                }
                eprintln!("Shell_NotifyIconW(NIM_SETVERSION) failed");
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                None
            })();
            self.tray = result.is_some();
        }
        if !self.tray {
            eprintln!("Tray unavailable; use the prototype's right-click menu to exit");
        }
    }

    pub fn menu(
        &self,
        position: Option<PhysicalPosition<i32>>,
        passthrough: bool,
        visible: bool,
    ) -> windows::core::Result<Option<Command>> {
        // The popup is owned by this HWND and dispatched on its event thread.
        unsafe {
            let mut point = POINT::default();
            if let Some(position) = position {
                point.x = position.x;
                point.y = position.y;
            } else {
                GetCursorPos(&mut point)?;
            }
            let menu = Menu(CreatePopupMenu()?);
            let check = if passthrough {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
            let enabled = if self.tray { MF_ENABLED } else { MF_GRAYED };
            AppendMenuW(menu.0, MF_STRING | check | enabled, 1, w!("全窗口鼠标穿透"))?;
            AppendMenuW(menu.0, MF_STRING, 2, w!("退出原型"))?;
            AppendMenuW(menu.0, MF_STRING, 3, w!("中文输入预览"))?;
            AppendMenuW(
                menu.0,
                MF_STRING | enabled,
                4,
                if visible {
                    w!("隐藏桌宠")
                } else {
                    w!("显示桌宠")
                },
            )?;
            let previous = GetForegroundWindow();
            let _ = SetForegroundWindow(self.hwnd);
            let selected = TrackPopupMenu(
                menu.0,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                point.x,
                point.y,
                0,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(self.hwnd, WM_NULL, WPARAM(0), LPARAM(0));
            if !previous.is_invalid() && previous != self.hwnd && GetForegroundWindow() == self.hwnd
            {
                let _ = SetForegroundWindow(previous);
            }
            Ok(match selected.0 {
                1 if self.tray => Some(Command::TogglePassthrough),
                2 => Some(Command::Exit),
                3 => Some(Command::OpenPreview),
                4 if self.tray => Some(Command::ToggleVisibility),
                _ => None,
            })
        }
    }

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
            let region = Region::rect(0, 0, 0, 0)?;
            let [_, _, width, height] = frame.rect;
            for (y, row) in frame.rows().enumerate() {
                let top = scaled_edge(y as u32, height, client.bottom) + offset_y;
                let bottom = scaled_edge(y as u32 + 1, height, client.bottom) + offset_y;
                if top == bottom {
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
                        let left = scaled_edge(first, width, client.right) + offset_x;
                        let right = scaled_edge(x as u32, width, client.right) + offset_x;
                        if left == right {
                            continue;
                        }
                        let run = Region::rect(left, top, right, bottom)?;
                        if CombineRgn(region.0, region.0, run.0, RGN_OR) == RGN_ERROR {
                            return Err("could not combine alpha hit regions".into());
                        }
                    }
                }
            }
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
            })
        }
    }

    fn update(mut self, hwnd: HWND, position: LPARAM) -> Self {
        let mut cursor = message_point(position.0 as usize);
        // Handle the move while dispatching its native message, before a queued
        // button-up can make the cursor or button state refer to a later event.
        unsafe {
            if !ClientToScreen(hwnd, &mut cursor).as_bool() {
                return self;
            }
            self.moving |= cursor.x.abs_diff(self.start.x) > self.x_threshold
                || cursor.y.abs_diff(self.start.y) > self.y_threshold;
            if self.moving {
                let dpi = GetDpiForWindow(hwnd).max(1);
                self.place_at_cursor(hwnd, cursor, dpi);
                let current_dpi = GetDpiForWindow(hwnd).max(1);
                if current_dpi != dpi {
                    self.place_at_cursor(hwnd, cursor, current_dpi);
                }
            }
        }
        self
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
            let _ = RemoveWindowSubclass(self.hwnd, Some(window_proc), SUBCLASS_ID);
        }
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
    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Result<Self, String> {
        let handle = unsafe { CreateRectRgn(left, top, right, bottom) };
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
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => {
            let _ = callback.proxy.send_event(AppEvent::ResumeDisplay);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => callback.drag.set(Drag::start(hwnd, lparam)),
        WM_MOUSEMOVE if wparam.0 & MK_LBUTTON.0 as usize != 0 => {
            if let Some(drag) = callback.drag.get() {
                let started = callback
                    .trace_input
                    .then(|| unsafe { GetMessageTime() as u32 });
                let drag = drag.update(hwnd, lparam);
                callback.drag.set(Some(drag));
                if let Some(started) = started
                    && drag.moving
                    && unsafe { DwmFlush() }.is_ok()
                {
                    let latency = unsafe { GetTickCount() }.wrapping_sub(started);
                    eprintln!("P1b drag compositor flush: {latency} ms");
                }
            }
        }
        WM_MOUSEMOVE | WM_LBUTTONUP | WM_CANCELMODE | WM_CAPTURECHANGED => {
            callback.drag.set(None);
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
