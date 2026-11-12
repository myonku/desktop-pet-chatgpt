use std::{
    env,
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
};

use sha2::{Digest, Sha256};
use windows::{
    Win32::{
        Foundation::{
            BOOL, CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, WPARAM,
        },
        Storage::FileSystem::GetDriveTypeW,
        System::{Com::CoTaskMemFree, Threading::CreateMutexW},
        UI::{
            Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellExecuteW},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

pub(super) const ACTIVATE_MESSAGE: u32 = WM_APP + 2;

pub struct UserProfile {
    pub root: PathBuf,
    pub login_registration: bool,
}

pub fn user_profile() -> io::Result<UserProfile> {
    // Windows allocates the known-folder string with the COM task allocator.
    let known = unsafe {
        SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, HANDLE::default())
            .ok()
            .map(|path| {
                let folder = PathBuf::from(OsString::from_wide(path.as_wide()));
                CoTaskMemFree(Some(path.0.cast()));
                folder
            })
    };
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| known.clone())
        .ok_or_else(|| io::Error::other("Windows Local AppData is unavailable"))?;
    let base = std::path::absolute(base)?;
    let login_registration = known.as_ref().is_some_and(|known| {
        base.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&known.as_os_str().to_string_lossy())
    });
    Ok(UserProfile {
        root: base.join("DesktopPetChatGPT"),
        login_registration,
    })
}

pub struct Instance {
    handle: HANDLE,
    marker: Vec<u16>,
}

impl Instance {
    pub fn acquire(root: &Path) -> windows::core::Result<Option<Self>> {
        Self::open(root, true)
    }

    pub fn acquire_for_migration(root: &Path) -> windows::core::Result<Option<Self>> {
        Self::open(root, false)
    }

    fn open(root: &Path, activate_existing: bool) -> windows::core::Result<Option<Self>> {
        let digest = Sha256::digest(root.as_os_str().to_string_lossy().to_lowercase().as_bytes());
        let name = format!("DesktopPetChatGPT.{digest:x}");
        let marker = wide(&name);
        let mutex = wide(&format!("Local\\{name}"));
        // Keep the mutex handle for the whole application lifetime, including window teardown.
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(mutex.as_ptr()))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                if !activate_existing {
                    let _ = CloseHandle(handle);
                    return Ok(None);
                }
                let mut search = FindInstance {
                    marker: PCWSTR(marker.as_ptr()),
                    window: HWND::default(),
                };
                let _ = EnumWindows(
                    Some(find_instance),
                    LPARAM((&mut search as *mut FindInstance) as isize),
                );
                if search.window.is_invalid()
                    || PostMessageW(search.window, ACTIVATE_MESSAGE, WPARAM(0), LPARAM(0)).is_err()
                {
                    MessageBoxW(
                        HWND::default(),
                        w!("桌宠已经运行。可从系统托盘打开设置。"),
                        w!("Desktop Pet ChatGPT"),
                        MB_OK | MB_SETFOREGROUND,
                    );
                }
                let _ = CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(Self { handle, marker }))
        }
    }

    pub fn marker(&self) -> &[u16] {
        &self.marker
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

struct FindInstance {
    marker: PCWSTR,
    window: HWND,
}

unsafe extern "system" fn find_instance(hwnd: HWND, data: LPARAM) -> BOOL {
    // EnumWindows invokes this callback synchronously on the calling thread.
    let search = unsafe { &mut *(data.0 as *mut FindInstance) };
    if unsafe { GetPropW(hwnd, search.marker) }.is_invalid() {
        return true.into();
    }
    search.window = hwnd;
    false.into()
}

pub fn local_path(path: &Path) -> io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(io::Error::other("素材包需要本地磁盘路径。"));
    };
    let drive = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
        _ => return Err(io::Error::other("素材包需要本地磁盘路径。")),
    };
    let root = [u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0];
    let kind = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
    // Drive types 2, 3, 5 and 6 are removable, fixed, optical and RAM disks.
    if !matches!(kind, 2 | 3 | 5 | 6) {
        return Err(io::Error::other("素材包需要可访问的本地磁盘。"));
    }
    Ok(path)
}

pub fn reveal_directory(path: &Path) -> io::Result<()> {
    let path = local_path(path)?;
    let directory = path
        .ancestors()
        .find(|ancestor| ancestor.is_dir())
        .ok_or_else(|| io::Error::other("未找到可打开的素材目录。"))?;
    let path = local_path(&directory.canonicalize()?)?;
    let path: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("open"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err(io::Error::other(format!(
            "无法打开目录（Windows {}）。",
            result.0 as isize
        )));
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

pub fn confirm_config_reset(problem: &str) -> bool {
    let message = wide(&format!(
        "无法加载配置文件。是否备份原文件并用默认配置覆盖？选择「否」将退出。\n\n{problem}"
    ));
    unsafe {
        MessageBoxW(
            HWND::default(),
            PCWSTR(message.as_ptr()),
            w!("桌宠配置无法加载"),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND,
        ) == IDYES
    }
}

pub fn show_config_error(message: &str) {
    let message = wide(message);
    unsafe {
        MessageBoxW(
            HWND::default(),
            PCWSTR(message.as_ptr()),
            w!("桌宠配置恢复失败"),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}
