use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            KillTimer, SetTimer, WM_ENTERMENULOOP, WM_ENTERSIZEMOVE, WM_EXITMENULOOP,
            WM_EXITSIZEMOVE, WM_TIMER,
        },
    },
};
use winit::{
    event_loop::EventLoopProxy,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

use crate::app::AppEvent;

pub(super) const TIMER: usize = 0x5054;
const SUBCLASS: usize = 2;

// The Windows menu and move/resize loops pump messages but bypass winit's wait deadline.
pub(super) fn start(hwnd: HWND) {
    if unsafe { SetTimer(hwnd, TIMER, 16, None) } == 0 {
        eprintln!("Could not schedule playback during the Windows modal loop");
    }
}

pub(super) fn stop(hwnd: HWND) {
    let _ = unsafe { KillTimer(hwnd, TIMER) };
}

pub(crate) struct SettingsModal {
    hwnd: HWND,
    _proxy: Box<EventLoopProxy<AppEvent>>,
}

impl SettingsModal {
    pub(crate) fn install(
        window: &Window,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self, String> {
        let handle = window.window_handle().map_err(|error| error.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("expected a Win32 settings window".into());
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        let proxy = Box::new(proxy);
        if !unsafe {
            SetWindowSubclass(
                hwnd,
                Some(settings_proc),
                SUBCLASS,
                &*proxy as *const _ as usize,
            )
        }
        .as_bool()
        {
            return Err("could not install settings modal callback".into());
        }
        Ok(Self {
            hwnd,
            _proxy: proxy,
        })
    }
}

impl Drop for SettingsModal {
    fn drop(&mut self) {
        stop(self.hwnd);
        let _ = unsafe { RemoveWindowSubclass(self.hwnd, Some(settings_proc), SUBCLASS) };
    }
}

unsafe extern "system" fn settings_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    match message {
        WM_ENTERSIZEMOVE | WM_ENTERMENULOOP => start(hwnd),
        WM_EXITSIZEMOVE | WM_EXITMENULOOP => stop(hwnd),
        WM_TIMER if wparam.0 == TIMER => {
            let proxy = unsafe { &*(data as *const EventLoopProxy<AppEvent>) };
            let _ = proxy.send_event(AppEvent::ModalTick);
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
