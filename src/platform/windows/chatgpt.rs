use std::{
    ffi::OsString,
    mem::size_of,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
};

use windows::{
    Win32::{
        Foundation::{
            ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_FILES, ERROR_SUCCESS, FILETIME, HANDLE,
            STILL_ACTIVE,
        },
        Storage::Packaging::Appx::{GetPackageFullName, GetPackagesByPackageFamily},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            RemoteDesktop::ProcessIdToSessionId,
            Threading::{
                GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    },
    core::{PWSTR, w},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Desktop {
    pub installed: bool,
    pub version: Option<String>,
    pub source: Option<Source>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub cli: PathBuf,
    desktop_pid: u32,
    desktop_started: u64,
}

impl Source {
    pub(crate) fn new(cli: PathBuf, desktop_pid: u32, desktop_started: u64) -> Self {
        Self {
            cli,
            desktop_pid,
            desktop_started,
        }
    }
}

const PACKAGE: &str = "OpenAI.Codex";
const PUBLISHER: &str = "2p2nqsd0c76g0";

pub(crate) fn discover() -> windows::core::Result<Desktop> {
    let mut count = 0;
    let mut length = 0;
    // Query the current user's registered packages without reading their files.
    let status = unsafe {
        GetPackagesByPackageFamily(
            w!("OpenAI.Codex_2p2nqsd0c76g0"),
            &mut count,
            None,
            &mut length,
            PWSTR::null(),
        )
    };
    if status != ERROR_SUCCESS && status != ERROR_INSUFFICIENT_BUFFER {
        return Err(status.into());
    }
    let mut desktop = Desktop {
        installed: count > 0,
        ..Desktop::default()
    };
    if !desktop.installed {
        return Ok(desktop);
    }

    let mut session = 0;
    unsafe { ProcessIdToSessionId(std::process::id(), &mut session)? };
    let processes = candidates(session)?;
    let mut roots: Vec<_> = processes
        .iter()
        .filter(|process| process.version.is_some())
        .filter(|process| {
            !processes.iter().any(|parent| {
                parent.pid == process.parent
                    && parent.started <= process.started
                    && parent.version.is_some()
                    && parent.version == process.version
            })
        })
        .collect();
    // Each launch uses this desktop pet process's one App Server login context.
    // Multiple Electron windows and CLI children do not create separate credentials.
    roots.sort_unstable_by_key(|root| (root.started, root.pid));
    desktop.version = roots.first().and_then(|root| root.version.clone());
    for root in roots {
        if let Some(cli) = processes
            .iter()
            .filter(|process| process.parent == root.pid && process.started >= root.started)
            .filter_map(|process| process.cli.as_ref())
            .min()
        {
            desktop.version.clone_from(&root.version);
            desktop.source = Some(Source::new(cli.clone(), root.pid, root.started));
            break;
        }
    }
    Ok(desktop)
}

struct Process {
    pid: u32,
    parent: u32,
    started: u64,
    version: Option<String>,
    cli: Option<PathBuf>,
}

fn candidates(session: u32) -> windows::core::Result<Vec<Process>> {
    let cli_root =
        super::session::local_app_data().map(|path| path.join("OpenAI").join("Codex").join("bin"));
    // RAII handle closes the snapshot when process enumeration ends.
    let snapshot =
        unsafe { OwnedHandle::from_raw_handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?.0) };
    let snapshot_handle = HANDLE(snapshot.as_raw_handle());
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    unsafe { Process32FirstW(snapshot_handle, &mut entry)? };
    let mut processes = Vec::new();
    loop {
        if let Some(process) = candidate(&entry, session, cli_root.as_deref()) {
            processes.push(process);
        }
        if let Err(error) = unsafe { Process32NextW(snapshot_handle, &mut entry) } {
            return if error.code() == ERROR_NO_MORE_FILES.to_hresult() {
                Ok(processes)
            } else {
                Err(error)
            };
        }
    }
}

fn candidate(
    entry: &PROCESSENTRY32W,
    session: u32,
    cli_root: Option<&std::path::Path>,
) -> Option<Process> {
    let name = os_string(&entry.szExeFile);
    let is_desktop = name.eq_ignore_ascii_case("ChatGPT.exe");
    if !is_desktop && !name.eq_ignore_ascii_case("codex.exe") {
        return None;
    }
    let pid = entry.th32ProcessID;
    let mut process_session = 0;
    let process = unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        OwnedHandle::from_raw_handle(handle.0)
    };
    unsafe { ProcessIdToSessionId(pid, &mut process_session).ok()? };
    if process_session != session {
        return None;
    }
    let handle = HANDLE(process.as_raw_handle());
    let mut exit_code = 0;
    unsafe { GetExitCodeProcess(handle, &mut exit_code).ok()? };
    if exit_code != STILL_ACTIVE.0 as u32 {
        return None;
    }
    let [mut start, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
    unsafe { GetProcessTimes(handle, &mut start, &mut exit, &mut kernel, &mut user).ok()? };
    let (version, cli) = if is_desktop {
        (Some(package_version(handle)?), None)
    } else {
        let mut image = vec![0; 32768];
        let mut length = image.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(image.as_mut_ptr()),
                &mut length,
            )
            .ok()?
        };
        let path = PathBuf::from(OsString::from_wide(&image[..length as usize]));
        let parent = path.parent()?.parent()?;
        if !parent
            .as_os_str()
            .eq_ignore_ascii_case(cli_root?.as_os_str())
            || !path.file_name()?.eq_ignore_ascii_case("codex.exe")
        {
            return None;
        }
        (None, Some(path))
    };
    Some(Process {
        pid,
        parent: entry.th32ParentProcessID,
        started: (u64::from(start.dwHighDateTime) << 32) | u64::from(start.dwLowDateTime),
        version,
        cli,
    })
}

fn package_version(handle: HANDLE) -> Option<String> {
    let mut length = 0;
    if unsafe { GetPackageFullName(handle, &mut length, PWSTR::null()) }
        != ERROR_INSUFFICIENT_BUFFER
    {
        return None;
    }
    let mut name = vec![0; length as usize];
    if unsafe { GetPackageFullName(handle, &mut length, PWSTR(name.as_mut_ptr())) } != ERROR_SUCCESS
    {
        return None;
    }
    let full_name = os_string(&name);
    let mut fields = full_name.to_str()?.split('_');
    let package = fields.next()?;
    let version = fields.next()?;
    (package.eq_ignore_ascii_case(PACKAGE) && fields.next_back()?.eq_ignore_ascii_case(PUBLISHER))
        .then(|| version.to_owned())
}

fn os_string(buffer: &[u16]) -> OsString {
    let length = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..length])
}
