use std::cell::Cell;

use windows::{
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DWMWA_CLOAK, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
            },
            Gdi::{
                GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
            },
        },
        System::Threading::GetCurrentThreadId,
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::*,
        },
    },
    core::HSTRING,
};

mod follow;
pub(crate) use follow::QuickFollow;

pub(crate) fn panel_window(title: &str) -> Option<HWND> {
    struct Search<'a> {
        title: &'a [u16],
        window: Option<HWND>,
    }
    unsafe extern "system" fn find(hwnd: HWND, arg: LPARAM) -> BOOL {
        let state = unsafe { &mut *(arg.0 as *mut Search<'_>) };
        let mut title = [0; 80];
        let len = unsafe { GetWindowTextW(hwnd, &mut title) } as usize;
        if title[..len] == *state.title {
            state.window = Some(hwnd);
            return false.into();
        }
        true.into()
    }
    let title = HSTRING::from(title);
    let mut state = Search {
        title: title.as_wide(),
        window: None,
    };
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(find),
            LPARAM(&mut state as *mut _ as isize),
        );
    }
    state.window
}

pub(crate) fn fix_settings_frame(hwnd: HWND) -> windows::core::Result<()> {
    // The settings child owns this window. Preserve its caption, close and minimize controls.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        SetWindowLongW(
            hwnd,
            GWL_STYLE,
            style & !((WS_THICKFRAME | WS_MAXIMIZEBOX).0 as i32),
        );
        let mut bounds = RECT::default();
        GetWindowRect(hwnd, &mut bounds)?;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            return Err(windows::core::Error::from_win32());
        }
        let work = monitor.rcWork;
        let width = (bounds.right - bounds.left).min(work.right - work.left);
        let height = (bounds.bottom - bounds.top).min(work.bottom - work.top);
        SetWindowPos(
            hwnd,
            None,
            bounds.left.clamp(work.left, work.right - width),
            bounds.top.clamp(work.top, work.bottom - height),
            width,
            height,
            SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOACTIVATE,
        )
    }
}

// Reactor activates its native window before delivering the first size callback.
thread_local! {
    static CLOAKED_QUICK: Cell<Option<HWND>> = const { Cell::new(None) };
}

pub(crate) struct QuickStartup(HHOOK);

impl QuickStartup {
    pub(crate) fn begin() -> windows::core::Result<Self> {
        let hook = unsafe {
            SetWindowsHookExW(
                WH_CALLWNDPROC,
                Some(before_quick_show),
                None,
                GetCurrentThreadId(),
            )?
        };
        Ok(Self(hook))
    }
}

impl Drop for QuickStartup {
    fn drop(&mut self) {
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
        CLOAKED_QUICK.set(None);
    }
}

fn cloak(hwnd: HWND, hidden: bool) -> windows::core::Result<()> {
    let value = BOOL::from(hidden);
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            (&raw const value).cast(),
            std::mem::size_of_val(&value) as u32,
        )
    }
}

unsafe extern "system" fn before_quick_show(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && CLOAKED_QUICK.get().is_none() {
        let msg = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        let showing = (msg.message == WM_SHOWWINDOW && msg.wParam.0 != 0)
            || (msg.message == WM_WINDOWPOSCHANGING
                && msg.lParam.0 != 0
                && unsafe { (*(msg.lParam.0 as *const WINDOWPOS)).flags }.contains(SWP_SHOWWINDOW));
        if showing {
            let mut class = [0; 64];
            let len = unsafe { GetClassNameW(msg.hwnd, &mut class) };
            if len > 0
                && class[..len as usize]
                    .iter()
                    .copied()
                    .eq("WinUIDesktopWin32WindowClass".encode_utf16())
                && cloak(msg.hwnd, true).is_ok()
            {
                CLOAKED_QUICK.set(Some(msg.hwnd));
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub(crate) fn reveal_quick_window(hwnd: HWND) -> windows::core::Result<()> {
    if CLOAKED_QUICK.get() != Some(hwnd) {
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_UNEXPECTED,
            "Quick window was not cloaked before activation",
        ));
    }
    cloak(hwnd, false)?;
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

thread_local! {
    static QUICK_WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static QUICK_PET: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static QUICK_ESCAPE: Cell<bool> = const { Cell::new(false) };
    static QUICK_ACTIVATED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) struct QuickWindow {
    keyboard: HHOOK,
    foreground: HWINEVENTHOOK,
}

impl QuickWindow {
    pub(crate) fn install(hwnd: HWND, pet: HWND, escape: bool) -> windows::core::Result<Self> {
        let mut hooks = Self {
            keyboard: HHOOK::default(),
            foreground: HWINEVENTHOOK::default(),
        };
        unsafe {
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            SetWindowLongW(
                hwnd,
                GWL_STYLE,
                style
                    & !((WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX)
                        .0 as i32),
            );
            let extended = GetWindowLongW(hwnd, GWL_EXSTYLE);
            SetWindowLongW(
                hwnd,
                GWL_EXSTYLE,
                (extended & !(WS_EX_APPWINDOW.0 as i32)) | WS_EX_TOOLWINDOW.0 as i32,
            );
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOSIZE | SWP_NOMOVE | SWP_FRAMECHANGED | SWP_NOACTIVATE,
            )?;

            QUICK_WINDOW.set(hwnd);
            QUICK_PET.set(pet);
            QUICK_ESCAPE.set(escape);
            QUICK_ACTIVATED.set(GetForegroundWindow() == hwnd);
            hooks.keyboard = SetWindowsHookExW(
                WH_GETMESSAGE,
                Some(dismiss_gesture),
                None,
                GetCurrentThreadId(),
            )?;
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corner).cast(),
                std::mem::size_of_val(&corner) as u32,
            );
            hooks.foreground = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(foreground_changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            if hooks.foreground.is_invalid() {
                return Err(windows::core::Error::from_win32());
            }
        }
        Ok(hooks)
    }
}

impl Drop for QuickWindow {
    fn drop(&mut self) {
        QUICK_WINDOW.set(HWND::default());
        QUICK_PET.set(HWND::default());
        // These hooks run on the installing WinUI message thread.
        unsafe {
            if !self.keyboard.is_invalid() {
                let _ = UnhookWindowsHookEx(self.keyboard);
            }
            if !self.foreground.is_invalid() {
                let _ = UnhookWinEvent(self.foreground);
            }
        }
    }
}

fn dismiss_quick(hwnd: HWND) {
    if !hwnd.is_invalid() {
        unsafe {
            let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn foreground_changed(
    _: HWINEVENTHOOK,
    _: u32,
    foreground: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let hwnd = QUICK_WINDOW.get();
    if foreground == hwnd {
        QUICK_ACTIVATED.set(true);
    } else if QUICK_ACTIVATED.get()
        && !foreground.is_invalid()
        && foreground == unsafe { GetForegroundWindow() }
        && foreground != QUICK_PET.get()
        && !desktop_surface(foreground)
        && !same_process(foreground)
    {
        dismiss_quick(hwnd);
    }
}

fn same_process(hwnd: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == std::process::id()
}

fn desktop_surface(hwnd: HWND) -> bool {
    // Clicking the desktop can activate Explorer's wallpaper window without entering an app.
    unsafe {
        let shell = GetShellWindow();
        if hwnd == shell {
            return true;
        }
        let mut shell_pid = 0;
        let mut pid = 0;
        GetWindowThreadProcessId(shell, Some(&mut shell_pid));
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut class = [0; 24];
        let length = GetClassNameW(hwnd, &mut class) as usize;
        pid != 0 && pid == shell_pid && class[..length].iter().copied().eq("WorkerW".encode_utf16())
    }
}

unsafe extern "system" fn dismiss_gesture(code: i32, removed: WPARAM, arg: LPARAM) -> LRESULT {
    if code >= 0 && removed.0 == PM_REMOVE.0 as usize && arg.0 != 0 {
        let msg = unsafe { &mut *(arg.0 as *mut MSG) };
        let dismiss = matches!(msg.message, WM_RBUTTONUP | WM_NCRBUTTONUP)
            || (QUICK_ESCAPE.get() && msg.message == WM_KEYDOWN && msg.wParam.0 == 0x1B);
        if dismiss {
            // WinUI may direct keyboard input to a child HWND.
            let hwnd = QUICK_WINDOW.get();
            if unsafe { GetAncestor(msg.hwnd, GA_ROOT) } == hwnd {
                dismiss_quick(hwnd);
                msg.message = WM_NULL;
            }
        }
    }
    unsafe { CallNextHookEx(None, code, removed, arg) }
}

pub(crate) fn position_quick_window(
    hwnd: HWND,
    pet: HWND,
    width_dip: i32,
    height_dip: i32,
) -> windows::core::Result<()> {
    place_quick(hwnd, quick_placement(pet, width_dip, height_dip)?.bounds)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct QuickPlacement {
    bounds: RECT,
    work: RECT,
    monitor: HMONITOR,
    dpi: u32,
}

fn quick_placement(
    pet: HWND,
    width_dip: i32,
    height_dip: i32,
) -> windows::core::Result<QuickPlacement> {
    let mut anchor = RECT::default();
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetWindowRect(pet, &mut anchor)?;
        let display = MonitorFromWindow(pet, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(display, &mut monitor).as_bool() {
            return Err(windows::core::Error::from_win32());
        }
        let work = monitor.rcWork;
        let dpi = GetDpiForWindow(pet);
        let work_width = work.right - work.left;
        let work_height = work.bottom - work.top;
        if dpi == 0 || work_width <= 0 || work_height <= 0 {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_UNEXPECTED,
                "Quick panel anchor has no usable monitor geometry",
            ));
        }
        let scale = |dip| dip * dpi as i32 / 96;
        let width = scale(width_dip).clamp(1, work_width);
        let height = scale(height_dip).clamp(1, work_height);
        let gap = scale(8);
        let right = anchor.right + gap;
        let left = anchor.left - gap - width;
        let (x, y) = if right + width <= work.right {
            (right, anchor.top)
        } else if left >= work.left {
            (left, anchor.top)
        } else if anchor.top - gap - height >= work.top {
            (anchor.left, anchor.top - gap - height)
        } else {
            (anchor.left, anchor.bottom + gap)
        };
        let x = x.clamp(work.left, work.right - width);
        let y = y.clamp(work.top, work.bottom - height);
        Ok(QuickPlacement {
            bounds: RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            },
            work,
            monitor: display,
            dpi,
        })
    }
}

fn place_quick(hwnd: HWND, bounds: RECT) -> windows::core::Result<()> {
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    }
}
