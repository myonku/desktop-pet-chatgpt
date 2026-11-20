use std::{env, error::Error, ffi::OsString, io, path::PathBuf};

use crate::{
    platform::windows::{Instance, legacy_preferences, user_profile},
    storage::migration::{self, Source},
};

const USAGE: &str = "+desktop-pet-chatgpt
desktop-pet-chatgpt migrate config-json [path]
desktop-pet-chatgpt migrate bailong-preferences
desktop-pet-chatgpt migrate bailong-history [path]
desktop-pet-chatgpt migrate rollback <migration-id>";

pub(super) fn run(args: &[OsString]) -> Result<(), Box<dyn Error>> {
    let (kind, parameters) = match args {
        [action] if action == "--help" || action == "migrate" => {
            println!("{USAGE}");
            return Ok(());
        }
        [action, kind, parameters @ ..] if action == "migrate" => (kind, parameters),
        _ => return Err(io::Error::other(USAGE).into()),
    };
    let profile = user_profile()?.root;
    let _instance = Instance::acquire_for_migration(&profile)?
        .ok_or_else(|| io::Error::other("请先退出此配置目录下的桌宠实例或等待迁移完成。"))?;
    let source = match (kind.to_str(), parameters) {
        (Some("config-json"), []) => Source::ConfigJson(profile.join("config.json")),
        (Some("config-json"), [path]) => Source::ConfigJson(PathBuf::from(path)),
        (Some("bailong-preferences"), []) => Source::BailongPreferences(legacy_preferences()?),
        (Some("bailong-history"), []) => {
            let directory = env::var_os("BAILONG_DATA_DIR")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| profile.with_file_name("BailongPet"));
            Source::BailongHistory(directory.join("conversations.json"))
        }
        (Some("bailong-history"), [path]) => Source::BailongHistory(PathBuf::from(path)),
        (Some("rollback"), [id]) => {
            let id = id.to_str().ok_or_else(|| io::Error::other(USAGE))?;
            let changed = migration::rollback(&profile, id)?;
            println!(
                "{} {id}",
                if changed {
                    "Rolled back"
                } else {
                    "Already rolled back"
                }
            );
            return Ok(());
        }
        _ => return Err(io::Error::other(USAGE).into()),
    };
    let result = migration::import(&profile, source)?;
    println!(
        "{} {}",
        if result.already_applied {
            "Already imported"
        } else {
            "Imported"
        },
        result.id
    );
    Ok(())
}
