use std::os::windows::fs::OpenOptionsExt;

use super::*;

pub(super) struct Profile(pub(super) PathBuf);

impl Profile {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("desktop-pet-test-{}", file_suffix()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn toml_defaults_and_settings_round_trip() {
    let root = Profile::new();
    fs::write(
        root.0.join("config.toml"),
        b"\xef\xbb\xbfinitial_scale_percent = 70\npack_path = \"packs/dawn\"\n",
    )
    .unwrap();
    let (mut store, config) = ConfigStore::load(root.0.join("config.toml"), false);
    let mut config = config.unwrap();
    assert!(store.problem().is_none());
    assert_eq!(config.saved_size, None);
    assert!(config.preferences.sound);
    assert!(config.preferences.task_bubbles);
    assert!(!config.preferences.start_hidden);
    assert!(!config.preferences.passthrough);
    config.saved_size = Some(SavedSize {
        base_dip: 250.0,
        percent: 120,
    });
    config.preferences.sound = false;
    config.preferences.start_hidden = true;
    config.preferences.fullscreen_hide = false;
    config.preferences.task_bubbles = false;
    config.preferences.bubble_scale_percent = 125;
    config.preferences.passthrough = true;
    assert!(store.save(&config, false).unwrap().is_none());
    let (_, saved) = ConfigStore::load(root.0.join("config.toml"), false);
    assert_eq!(saved.as_ref(), Some(&config));
    assert_eq!(saved.unwrap().saved_size.unwrap().dip(), 300.0);
    store.save(&Config::default(), true).unwrap();
    let (_, reset) = ConfigStore::load(root.0.join("config.toml"), false);
    assert_eq!(reset, Some(Config::default()));
}

#[test]
fn malformed_and_future_configs_are_preserved_until_explicit_reset() {
    for bytes in [
        b"corrupt".as_slice(),
        b"schema_version = 2\n",
        b"[preferences]\nbubble_scale_percent = 300\n",
    ] {
        let root = Profile::new();
        let path = root.0.join("config.toml");
        fs::write(&path, bytes).unwrap();
        let (mut store, fallback) = ConfigStore::load(root.0.join("config.toml"), false);
        assert!(fallback.is_none());
        let defaults = Config::default();
        assert!(store.problem().is_some());
        assert!(store.save(&defaults, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let backup = store.save(&defaults, true).unwrap().unwrap();
        assert_eq!(fs::read(backup).unwrap(), bytes);
        assert!(store.problem().is_none());
    }
}

#[test]
fn save_refuses_to_overwrite_external_changes_and_locked_destinations() {
    let root = Profile::new();
    let path = root.0.join("config.toml");
    let bytes = b"schema_version = 1\n";
    fs::write(&path, bytes).unwrap();
    let (mut store, config) = ConfigStore::load(path.clone(), false);
    let config = config.unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .open(&path)
        .unwrap();
    assert!(store.save(&config, false).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    drop(lock);
    fs::write(&path, b"schema_version = 1\ninitial_scale_percent = 80\n").unwrap();
    assert!(store.save(&config, false).is_err());
    assert_ne!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn confirmed_repair_preserves_an_unreadable_file() {
    let root = Profile::new();
    let path = root.0.join("config.toml");
    let bytes = b"invalid";
    fs::write(&path, bytes).unwrap();
    let lock = OpenOptions::new()
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let (mut store, config) = ConfigStore::load(path.clone(), false);
    assert!(config.is_none());
    assert!(store.problem().is_some());
    assert!(store.save(&Config::default(), true).is_err());
    drop(lock);
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert!(!root.0.join("backups").exists());
}

#[test]
fn development_keys_require_both_the_feature_and_the_development_file() {
    let root = Profile::new();
    let path = root.0.join("config.toml");
    fs::write(
        &path,
        "[development]\nformat = \"rgba8\"\ntrace_input = true\n",
    )
    .unwrap();
    let (product, config) = ConfigStore::load(path.clone(), false);
    assert!(product.problem().is_some());
    assert!(config.is_none());

    let (development, config) = ConfigStore::load(path, true);
    assert_eq!(config.is_some(), cfg!(feature = "development"));
    assert_eq!(
        development.problem().is_none(),
        cfg!(feature = "development")
    );
}
