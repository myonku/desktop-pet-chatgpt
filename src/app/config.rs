use std::{env, io, path::PathBuf};

use crate::{
    platform::windows::{confirm_config_reset, show_config_error},
    storage::{Config, ConfigStore},
};

pub(super) struct StartupConfig {
    pub config: Config,
    pub store: ConfigStore,
    pub development: bool,
    pub pack_path: Option<PathBuf>,
}

impl StartupConfig {
    pub fn load(root: PathBuf) -> io::Result<Self> {
        if cfg!(feature = "development") {
            let (store, config) =
                ConfigStore::load(env::current_dir()?.join("desktop-pet.dev.toml"), true);
            if !store.is_missing() {
                return Self::from_store(store, config, true);
            }
        }
        let (store, config) = ConfigStore::load(root.join("config.toml"), false);
        Self::from_store(store, config, false)
    }

    fn from_store(
        mut store: ConfigStore,
        config: Option<Config>,
        development: bool,
    ) -> io::Result<Self> {
        let config = match config {
            Some(config) => config,
            None => {
                let problem = store.problem().unwrap();
                eprintln!("Could not load configuration: {problem}");
                if !confirm_config_reset(problem) {
                    return Err(io::Error::other("用户取消了配置恢复，启动已停止。"));
                }
                let defaults = Config::default();
                match store.save(&defaults, true) {
                    Ok(backup) => {
                        if let Some(backup) = backup {
                            eprintln!("Invalid configuration backed up to {}", backup.display());
                        }
                    }
                    Err(error) => {
                        let message = format!("无法写入默认配置，启动已停止。\n\n{error}");
                        show_config_error(&message);
                        return Err(io::Error::other(message));
                    }
                }
                defaults
            }
        };
        let pack_path = config
            .pack_path
            .as_ref()
            .map(|path| store.root().join(path));
        Ok(Self {
            config,
            store,
            development,
            pack_path,
        })
    }
}
