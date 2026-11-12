use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    core::preferences::{INITIAL_SCALE, Preferences, SavedSize},
    storage::{Config, ConfigStore},
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DevConfig {
    schema_version: Option<u32>,
    initial_scale_percent: Option<u16>,
    pack_path: Option<PathBuf>,
}

pub(super) struct StartupConfig {
    pub product: Config,
    pub store: ConfigStore,
    pub development: bool,
    pub scale_percent: u16,
    pub saved_size: Option<SavedSize>,
    pub pack_path: Option<PathBuf>,
}

impl StartupConfig {
    pub fn load(root: PathBuf, dev_path: Option<&Path>) -> io::Result<Self> {
        let (store, product) = ConfigStore::load(root);
        if let Some(error) = store.problem() {
            eprintln!("Could not load configuration: {error}");
        }
        let dev = if let Some(path) = dev_path {
            let load = || {
                let text = fs::read_to_string(path)?;
                let mut dev: DevConfig =
                    toml_edit::de::from_str(text.trim_start_matches('\u{feff}'))
                        .map_err(io::Error::other)?;
                if dev.schema_version.is_some_and(|version| version != 1) {
                    return Err(io::Error::other("expected schema_version = 1"));
                }
                let mut config = Config::default();
                config.initial_scale_percent = dev.initial_scale_percent;
                config.pack_path.clone_from(&dev.pack_path);
                config.validate()?;
                dev.pack_path = dev
                    .pack_path
                    .map(|pack| resolve_pack(path.parent().unwrap_or(Path::new("")), &pack));
                Ok(dev)
            };
            load().map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", path.display()))
            })?
        } else {
            DevConfig::default()
        };
        let scale_percent = dev
            .initial_scale_percent
            .or(product.initial_scale_percent)
            .unwrap_or(INITIAL_SCALE);
        let saved_size = if dev.initial_scale_percent.is_some() {
            None
        } else {
            product.saved_size
        };
        let pack_path = match dev.pack_path.or_else(|| {
            product
                .pack_path
                .as_ref()
                .map(|path| resolve_pack(store.root(), path))
        }) {
            Some(pack) => Some(pack),
            None => {
                let default = env::current_exe()?
                    .with_file_name("resources")
                    .join("default");
                default.exists().then_some(default)
            }
        };
        Ok(Self {
            product,
            store,
            development: dev_path.is_some(),
            scale_percent,
            saved_size,
            pack_path,
        })
    }

    pub fn preferences(&self) -> Preferences {
        self.product.preferences
    }
}

fn resolve_pack(base: &Path, path: &Path) -> PathBuf {
    base.join(path)
}
