use windows::{
    Win32::{
        Devices::DeviceAndDriverInstallation::*,
        Foundation::{HWND, RECT},
        Graphics::{
            Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
            Gdi::*,
        },
        System::Registry::{HKEY, KEY_READ, RRF_RT_REG_BINARY, RegCloseKey, RegGetValueW},
        UI::WindowsAndMessaging::*,
    },
    core::{PCWSTR, w},
};

use crate::core::WorkArea;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Display {
    monitor: HMONITOR,
    pub device: [u16; 32],
    pub bounds: WorkArea,
    pub work: WorkArea,
}

impl Display {
    pub fn for_window(window: HWND) -> Option<Self> {
        // Screen and work-area coordinates are physical pixels in the per-monitor DPI context.
        unsafe {
            let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if monitor.is_invalid() || !GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
                return None;
            }
            let area = |rect: RECT| WorkArea {
                min: [rect.left, rect.top],
                max: [rect.right, rect.bottom],
            };
            Some(Self {
                monitor,
                device: info.szDevice,
                bounds: area(info.monitorInfo.rcMonitor),
                work: area(info.monitorInfo.rcWork),
            })
        }
    }

    pub fn has_fullscreen(self, pet: HWND) -> bool {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_invalid()
                || window == pet
                || window == GetDesktopWindow()
                || window == GetShellWindow()
                || !IsWindowVisible(window).as_bool()
                || IsIconic(window).as_bool()
                || MonitorFromWindow(window, MONITOR_DEFAULTTONULL) != self.monitor
            {
                return false;
            }
            let style = GetWindowLongW(window, GWL_STYLE) as u32;
            if IsZoomed(window).as_bool() && style & WS_CAPTION.0 == WS_CAPTION.0 {
                return false;
            }
            let mut frame = RECT::default();
            if DwmGetWindowAttribute(
                window,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut frame as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            )
            .is_err()
                && GetWindowRect(window, &mut frame).is_err()
            {
                return false;
            }
            frame.left <= self.bounds.min[0]
                && frame.top <= self.bounds.min[1]
                && frame.right >= self.bounds.max[0]
                && frame.bottom >= self.bounds.max[1]
        }
    }
}

pub fn physical_size(device: [u16; 32]) -> Option<[u16; 2]> {
    // Read the EDID for the single active monitor interface on this GDI display source.
    // Mirrored or virtual sources can have ambiguous or absent physical dimensions.
    unsafe {
        let name = PCWSTR(device.as_ptr());
        let mut path = None;
        for index in 0..32 {
            let mut target = DISPLAY_DEVICEW {
                cb: size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            if !EnumDisplayDevicesW(name, index, &mut target, EDD_GET_DEVICE_INTERFACE_NAME)
                .as_bool()
            {
                break;
            }
            if target.StateFlags & DISPLAY_DEVICE_ACTIVE != 0
                && (target.DeviceID[0] == 0 || path.replace(target.DeviceID).is_some())
            {
                return None;
            }
            if index == 31 {
                return None;
            }
        }
        let path = path?;
        let edid = read_edid(PCWSTR(path.as_ptr()))?;
        let mut dimensions = edid_dimensions(&edid)?;
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if !EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS, &mut mode).as_bool() {
            return None;
        }
        if matches!(
            mode.Anonymous1.Anonymous2.dmDisplayOrientation,
            DMDO_90 | DMDO_270
        ) {
            dimensions.swap(0, 1);
        }
        let mode_aspect = f64::from(mode.dmPelsWidth) / f64::from(mode.dmPelsHeight);
        let panel_aspect = f64::from(dimensions[0]) / f64::from(dimensions[1]);
        (0.8..=1.25)
            .contains(&(mode_aspect / panel_aspect))
            .then_some(dimensions)
    }
}

fn read_edid(path: PCWSTR) -> Option<[u8; 128]> {
    struct DeviceSet(HDEVINFO);
    impl Drop for DeviceSet {
        fn drop(&mut self) {
            let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
        }
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }

    // The device set and hardware registry key are local to the probe thread.
    unsafe {
        let set = DeviceSet(SetupDiCreateDeviceInfoList(None, HWND::default()).ok()?);
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        SetupDiOpenDeviceInterfaceW(set.0, path, 0, Some(&mut interface)).ok()?;
        let mut device = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        let mut detail = [0u64; 128];
        let detail_ptr = detail
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        (*detail_ptr).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        SetupDiGetDeviceInterfaceDetailW(
            set.0,
            &interface,
            Some(detail_ptr),
            size_of_val(&detail) as u32,
            None,
            Some(&mut device),
        )
        .ok()?;
        let key =
            Key(
                SetupDiOpenDevRegKey(set.0, &device, DICS_FLAG_GLOBAL.0, 0, DIREG_DEV, KEY_READ.0)
                    .ok()?,
            );
        let mut bytes = [0u8; 1024];
        let mut size = bytes.len() as u32;
        RegGetValueW(
            key.0,
            PCWSTR::null(),
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            Some(bytes.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
        (size >= 128).then(|| bytes[..128].try_into().unwrap())
    }
}

fn edid_dimensions(edid: &[u8; 128]) -> Option<[u16; 2]> {
    if edid[..8] != [0, 255, 255, 255, 255, 255, 255, 0]
        || edid.iter().copied().fold(0u8, u8::wrapping_add) != 0
    {
        return None;
    }
    let size = [u16::from(edid[21]) * 10, u16::from(edid[22]) * 10];
    size.iter()
        .all(|mm| (100..=2500).contains(mm))
        .then_some(size)
}

#[cfg(test)]
mod tests;
