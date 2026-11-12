use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::core::preferences::{Preferences, SIZE_RANGE, SavedSize};

const SCHEMA_VERSION: u32 = 1;
const CONFIG_LIMIT: u64 = 64 * 1024;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_scale_percent: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_size: Option<SavedSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_path: Option<PathBuf>,
    #[serde(default)]
    pub preferences: Preferences,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            initial_scale_percent: None,
            saved_size: None,
            pack_path: None,
            preferences: Preferences::default(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid("expected schema_version = 1"));
        }
        if self
            .initial_scale_percent
            .is_some_and(|scale| !SIZE_RANGE.contains(&scale))
        {
            return Err(invalid("initial_scale_percent must be 50–200"));
        }
        if self.saved_size.is_some_and(|size| !size.valid()) {
            return Err(invalid("invalid saved_size"));
        }
        if !(50..=150).contains(&self.preferences.bubble_scale_percent) {
            return Err(invalid("bubble_scale_percent must be 50–150"));
        }
        if self.pack_path.as_ref().is_some_and(|path| {
            path.as_os_str().is_empty() || path.as_os_str().as_encoded_bytes().contains(&0)
        }) {
            return Err(invalid("pack_path must name a pack directory"));
        }
        Ok(())
    }
}

pub struct ConfigStore {
    root: PathBuf,
    snapshot: Option<Vec<u8>>,
    problem: Option<String>,
    unreadable: bool,
}

impl ConfigStore {
    pub fn load(root: PathBuf) -> (Self, Config) {
        let file = root.join("config.json");
        let (snapshot, result, unreadable) = match read_snapshot(&file) {
            Ok(Some(bytes)) => {
                let json = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
                let result = serde_json::from_slice::<Config>(json)
                    .map_err(io::Error::other)
                    .and_then(|config| {
                        config.validate()?;
                        Ok(config)
                    });
                (Some(bytes), result, false)
            }
            Ok(None) => (None, Ok(Config::default()), false),
            Err(error) => (None, Err(error), true),
        };
        let problem = result
            .as_ref()
            .err()
            .map(|error| format!("{} — {error}", file.display()));
        (
            Self {
                root,
                snapshot,
                problem,
                unreadable,
            },
            result.unwrap_or_default(),
        )
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub fn save(&mut self, config: &Config, repair: bool) -> io::Result<Option<PathBuf>> {
        config.validate()?;
        let file = self.root.join("config.json");
        if self.unreadable || self.problem.is_some() && !repair {
            return Err(io::Error::other(
                "配置文件不可写。请修复文件后重启，或在设置中明确恢复默认值。",
            ));
        }
        if read_snapshot(&file)? != self.snapshot {
            return Err(io::Error::other("配置文件已在外部更改，请重启后再保存。"));
        }
        let mut data = serde_json::to_vec_pretty(config).map_err(io::Error::other)?;
        data.push(b'\n');
        if data.len() as u64 > CONFIG_LIMIT {
            return Err(invalid("configuration exceeds 64 KiB"));
        }
        let backup = if let (Some(_), Some(original)) = (&self.problem, &self.snapshot) {
            let folder = self.root.join("backups");
            fs::create_dir_all(&folder)?;
            let path = folder.join(format!("config-invalid-{}.json", file_suffix()));
            write_new(&path, original)?;
            Some(path)
        } else {
            None
        };

        fs::create_dir_all(&self.root)?;
        let staged = self.root.join(format!(".config-{}.tmp", file_suffix()));
        write_new(&staged, &data)?;
        // Same-directory rename atomically replaces the destination on Windows.
        if let Err(error) = fs::rename(&staged, &file) {
            let _ = fs::remove_file(staged);
            return Err(error);
        }
        self.snapshot = Some(data);
        self.problem = None;
        Ok(backup)
    }
}

fn read_snapshot(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(CONFIG_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > CONFIG_LIMIT {
        return Err(invalid("configuration exceeds 64 KiB"));
    }
    Ok(Some(bytes))
}

fn write_new(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn file_suffix() -> String {
    format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    )
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests;
