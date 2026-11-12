use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_SCALE_PERCENT: u16 = 70;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    schema_version: Option<u32>,
    initial_scale_percent: Option<u16>,
    pack_path: Option<PathBuf>,
}

impl ConfigFile {
    fn validate(mut self, path: &Path, product: bool) -> io::Result<Self> {
        if (product || self.schema_version.is_some()) && self.schema_version != Some(SCHEMA_VERSION)
        {
            return Err(io::Error::other("expected schema_version = 1"));
        }
        if self
            .initial_scale_percent
            .is_some_and(|scale| !(50..=200).contains(&scale))
        {
            return Err(io::Error::other("initial_scale_percent must be 50–200"));
        }
        if let Some(pack) = &mut self.pack_path {
            if pack.as_os_str().is_empty() {
                return Err(io::Error::other("pack_path must name a pack directory"));
            }
            if !pack.is_absolute() {
                *pack = path.parent().unwrap_or(Path::new("")).join(&*pack);
            }
        }
        Ok(self)
    }
}

pub(super) struct StartupConfig {
    pub scale_percent: u16,
    pub pack_path: Option<PathBuf>,
}

impl StartupConfig {
    pub fn load(dev_path: Option<&Path>) -> io::Result<Self> {
        let mut product = ConfigFile::default();
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            let path = PathBuf::from(local).join("DesktopPetChatGPT/config.json");
            match read_product(&path) {
                Ok(config) => product = config,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => eprintln!("Could not load {}: {error}", path.display()),
            }
        }

        let dev = if let Some(path) = dev_path {
            let load = || {
                let text = fs::read_to_string(path)?;
                toml_edit::de::from_str::<ConfigFile>(text.trim_start_matches('\u{feff}'))
                    .map_err(io::Error::other)?
                    .validate(path, false)
            };
            load().map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", path.display()))
            })?
        } else {
            ConfigFile::default()
        };

        let scale = dev
            .initial_scale_percent
            .or(product.initial_scale_percent)
            .unwrap_or(DEFAULT_SCALE_PERCENT);
        let pack_path = match dev.pack_path.or(product.pack_path) {
            Some(path) => Some(path),
            None => {
                let default = env::current_exe()?
                    .with_file_name("resources")
                    .join("default");
                default.exists().then_some(default)
            }
        };
        Ok(Self {
            scale_percent: scale,
            pack_path,
        })
    }
}

fn read_product(path: &Path) -> io::Result<ConfigFile> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str::<ConfigFile>(text.trim_start_matches('\u{feff}'))
        .map_err(io::Error::other)?
        .validate(path, true)
}
