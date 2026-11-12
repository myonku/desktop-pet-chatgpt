use std::{io, os::windows::ffi::OsStrExt, path::Path};

use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, HMODULE},
        Media::Audio::{PlaySoundW, SND_ASYNC, SND_FLAGS, SND_MEMORY, SND_NODEFAULT},
        System::Registry::*,
    },
    core::{PCWSTR, w},
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("DesktopPetChatGPT");

pub struct Autostart {
    key: PCWSTR,
}

impl Default for Autostart {
    fn default() -> Self {
        Self { key: RUN_KEY }
    }
}

impl Autostart {
    pub fn read(&self) -> windows::core::Result<Option<Vec<u8>>> {
        let Some(key) = self.open(KEY_QUERY_VALUE)? else {
            return Ok(None);
        };
        let mut data = vec![0u8; 1024];
        let mut len = data.len() as u32;
        let result = unsafe {
            RegGetValueW(
                key.0,
                PCWSTR::null(),
                VALUE,
                RRF_RT_REG_SZ,
                None,
                Some(data.as_mut_ptr().cast()),
                Some(&mut len),
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        result.ok()?;
        data.truncate(len as usize);
        Ok(Some(data))
    }

    pub fn write(&self, value: Option<&[u8]>) -> windows::core::Result<()> {
        if let Some(data) = value {
            let mut key = Key(HKEY::default());
            unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    self.key,
                    0,
                    None,
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    None,
                    &mut key.0,
                    None,
                )
                .ok()?;
                RegSetValueExW(key.0, VALUE, 0, REG_SZ, Some(data)).ok()
            }
        } else if let Some(key) = self.open(KEY_SET_VALUE)? {
            let result = unsafe { RegDeleteValueW(key.0, VALUE) };
            if result == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                result.ok()
            }
        } else {
            Ok(())
        }
    }

    fn open(&self, access: REG_SAM_FLAGS) -> windows::core::Result<Option<Key>> {
        let mut key = HKEY::default();
        let result = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, self.key, 0, access, &mut key) };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        result.ok()?;
        Ok(Some(Key(key)))
    }
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }
}

pub fn startup_command(exe: &Path) -> io::Result<Vec<u8>> {
    let path: Vec<_> = exe.as_os_str().encode_wide().collect();
    if !exe.is_absolute() || path.len() + 2 > 260 || path.contains(&34) || path.contains(&0) {
        return Err(io::Error::other(
            "可执行文件路径无法用于 Windows 登录自启。",
        ));
    }
    Ok([34u16]
        .into_iter()
        .chain(path)
        .chain([34, 0])
        .flat_map(u16::to_le_bytes)
        .collect())
}

#[derive(Default)]
pub struct Sound {
    wave: Option<Vec<u8>>,
    playing: bool,
}

impl Sound {
    pub fn replace(&mut self, wave: Option<Vec<u8>>) {
        self.stop();
        self.wave = wave;
    }

    pub fn available(&self) -> bool {
        self.wave.is_some()
    }

    pub fn play(&mut self) -> bool {
        let Some(bytes) = &self.wave else {
            return false;
        };
        // WinMM retains the pointer until stop or completion. The buffer belongs to Sound.
        let started = unsafe {
            PlaySoundW(
                PCWSTR(bytes.as_ptr().cast()),
                HMODULE::default(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            )
            .as_bool()
        };
        self.playing |= started;
        started
    }

    pub fn stop(&mut self) {
        if self.playing {
            let _ = unsafe { PlaySoundW(PCWSTR::null(), HMODULE::default(), SND_FLAGS(0)) };
            self.playing = false;
        }
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
