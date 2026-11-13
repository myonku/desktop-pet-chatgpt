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
    pub fn default_pack(&self) -> io::Result<PathBuf> {
        if self.development {
            return Ok(self.store.root().join("assets/packs/white-dragon/0.1.0"));
        }
        Ok(env::current_exe()?
            .with_file_name("resources")
            .join("default"))
    }

    pub fn load(root: PathBuf) -> io::Result<Self> {
        let development = cfg!(feature = "development");
        let name = if development {
            "desktop-pet.dev.toml"
        } else {
            "config.toml"
        };
        let (store, config) = ConfigStore::load(root.join(name), development);
        Self::from_store(store, config, development)
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
