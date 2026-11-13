use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use windows::Win32::System::Registry::{REG_DWORD, REG_QWORD, REG_SZ};

use crate::{
    core::preferences::{Preferences, SavedSize},
    platform::windows::LegacyPreferences,
};

use super::{HISTORY_PROVIDER, SESSION_LIMIT, decode_json, digest, invalid};
use crate::storage::Config;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonConfig {
    schema_version: u32,
    initial_scale_percent: Option<u16>,
    saved_size: Option<SavedSize>,
    pack_path: Option<PathBuf>,
    #[serde(default)]
    preferences: Preferences,
}

pub(super) fn config_json(bytes: &[u8], source_dir: &Path, profile: &Path) -> io::Result<Config> {
    let old: JsonConfig = decode_json(bytes, "旧版配置")?;
    let mut config = Config {
        schema_version: old.schema_version,
        initial_scale_percent: old.initial_scale_percent,
        saved_size: old.saved_size,
        pack_path: old.pack_path,
        preferences: old.preferences,
        #[cfg(feature = "development")]
        development: None,
    };
    config.validate(false)?;
    if source_dir != profile
        && let Some(path) = &mut config.pack_path
        && !path.is_absolute()
    {
        let resolved = source_dir.join(&*path);
        if !resolved.is_absolute() {
            return Err(invalid("旧素材路径无法相对于原配置目录解析。"));
        }
        *path = resolved;
    }
    Ok(config)
}

pub(super) fn preferences(bytes: &[u8]) -> io::Result<Config> {
    let values: LegacyPreferences = decode_json(bytes, "旧版注册表偏好")?;
    let number = |name, min, max| {
        registry_value(&values, name, |kind, data| {
            let number = match kind {
                REG_DWORD => i64::from(i32::from_le_bytes(data.try_into().ok()?)),
                REG_QWORD => i64::from_le_bytes(data.try_into().ok()?),
                REG_SZ => registry_string(data)?.parse().ok()?,
                _ => return None,
            };
            u16::try_from(number.clamp(min, max)).ok()
        })
    };
    let flag = |name| {
        registry_value(&values, name, |kind, data| {
            if kind != REG_SZ {
                return None;
            }
            match registry_string(data)?.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }
        })
    };
    let defaults = Preferences::default();
    Ok(Config {
        initial_scale_percent: number("sizePercent", 50, 200)?,
        preferences: Preferences {
            auto_move: flag("autoMovementDisabled")?
                .map_or(defaults.auto_move, |disabled| !disabled),
            fullscreen_hide: flag("autoHideFullscreen")?.unwrap_or(defaults.fullscreen_hide),
            sound: flag("soundDisabled")?.map_or(defaults.sound, |disabled| !disabled),
            task_bubbles: flag("taskBubblesDisabled")?
                .map_or(defaults.task_bubbles, |disabled| !disabled),
            bubble_scale_percent: number("bubblePercent", 50, 150)?
                .unwrap_or(defaults.bubble_scale_percent),
            ..defaults
        },
        ..Config::default()
    })
}

fn registry_value<T>(
    values: &LegacyPreferences,
    name: &str,
    decode: impl FnOnce(windows::Win32::System::Registry::REG_VALUE_TYPE, &[u8]) -> Option<T>,
) -> io::Result<Option<T>> {
    values
        .get(name)
        .map(|(kind, data)| {
            decode(
                windows::Win32::System::Registry::REG_VALUE_TYPE(*kind),
                data,
            )
            .ok_or_else(|| invalid(&format!("旧版注册表偏好 {name} 的类型或值无效。")))
        })
        .transpose()
}

fn registry_string(data: &[u8]) -> Option<String> {
    let (units, rest) = data.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let units: Vec<_> = units.iter().copied().map(u16::from_le_bytes).collect();
    let end = units
        .iter()
        .rposition(|&unit| unit != 0)
        .map_or(0, |index| index + 1);
    String::from_utf16(&units[..end]).ok()
}

#[derive(Deserialize, Serialize)]
struct LegacyConversation {
    id: String,
    title: String,
    created: String,
    cwd: String,
    #[serde(rename(deserialize = "threadId", serialize = "thread_id"))]
    thread_id: Option<String>,
    mode: Option<String>,
    #[serde(
        rename(deserialize = "desktopSync", serialize = "desktop_sync"),
        default
    )]
    desktop_sync: bool,
    turns: Vec<LegacyTurn>,
}

#[derive(Deserialize, Serialize)]
struct LegacyTurn {
    status: String,
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    legacy_status: Option<String>,
    prompt: String,
    created: Option<String>,
    output: Option<String>,
    result: Option<String>,
    error: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    events: Vec<String>,
}

#[derive(Serialize)]
struct SessionSnapshot<'a> {
    schema_version: u32,
    provider_id: &'static str,
    session_id: &'a str,
    legacy: LegacyConversation,
}

pub(super) fn history(bytes: &[u8], migration_id: &str) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let conversations: Vec<LegacyConversation> = decode_json(bytes, "旧版对话历史")?;
    let mut files = BTreeMap::from([(".migration-id".into(), migration_id.as_bytes().to_vec())]);
    for mut chat in conversations {
        if chat.id.trim().is_empty() {
            return Err(invalid("旧版对话 ID 为空。"));
        }
        let session_id = digest(chat.id.as_bytes());
        let name = format!("{session_id}.json");
        if files.contains_key(&name) {
            return Err(invalid("旧版历史存在重复的对话 ID。"));
        }
        for turn in &mut chat.turns {
            if !matches!(
                turn.status.as_str(),
                "completed" | "failed" | "interrupted" | "cancelled" | "canceled"
            ) {
                turn.legacy_status =
                    Some(std::mem::replace(&mut turn.status, "interrupted".into()));
                if turn.error.as_deref().is_none_or(str::is_empty) {
                    turn.error = Some("旧版任务未确认完成。".into());
                }
            }
        }
        let snapshot = SessionSnapshot {
            schema_version: 1,
            provider_id: HISTORY_PROVIDER,
            session_id: &session_id,
            legacy: chat,
        };
        let data = serde_json::to_vec(&snapshot).map_err(io::Error::other)?;
        if data.len() as u64 > SESSION_LIMIT {
            return Err(invalid("转换后的单个会话超过 16 MiB。"));
        }
        files.insert(name, data);
    }
    Ok(files)
}
