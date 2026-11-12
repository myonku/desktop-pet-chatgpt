use std::os::windows::fs::OpenOptionsExt;

use super::*;

struct Profile(PathBuf);

impl Profile {
    fn new() -> Self {
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
fn old_config_defaults_and_new_settings_round_trip() {
    let root = Profile::new();
    fs::write(
        root.0.join("config.json"),
        r#"{"schema_version":1,"initial_scale_percent":70,"pack_path":"packs/dawn"}"#,
    )
    .unwrap();
    let (mut store, mut config) = ConfigStore::load(root.0.clone());
    assert!(store.problem().is_none());
    assert_eq!(config.saved_size, None);
    assert!(config.preferences.sound);
    assert!(config.preferences.task_bubbles);
    assert!(!config.preferences.passthrough);
    config.saved_size = Some(SavedSize {
        base_dip: 250.0,
        percent: 120,
    });
    config.preferences.sound = false;
    config.preferences.passthrough = true;
    assert!(store.save(&config, false).unwrap().is_none());
    let (_, saved) = ConfigStore::load(root.0.clone());
    assert_eq!(saved, config);
    assert_eq!(saved.saved_size.unwrap().dip(), 300.0);
    store.save(&Config::default(), true).unwrap();
    let (_, reset) = ConfigStore::load(root.0.clone());
    assert_eq!(reset, Config::default());
}

#[test]
fn malformed_and_future_configs_are_preserved_until_explicit_reset() {
    for bytes in [
        b"corrupt".as_slice(),
        br#"{"schema_version":2}"#,
        br#"{"schema_version":1,"preferences":{"bubble_scale_percent":300}}"#,
    ] {
        let root = Profile::new();
        let path = root.0.join("config.json");
        fs::write(&path, bytes).unwrap();
        let (mut store, fallback) = ConfigStore::load(root.0.clone());
        assert_eq!(fallback, Config::default());
        assert!(store.problem().is_some());
        assert!(store.save(&fallback, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let backup = store.save(&fallback, true).unwrap().unwrap();
        assert_eq!(fs::read(backup).unwrap(), bytes);
        assert!(store.problem().is_none());
    }
}

#[test]
fn save_refuses_to_overwrite_external_changes_and_locked_destinations() {
    let root = Profile::new();
    let path = root.0.join("config.json");
    let bytes = br#"{"schema_version":1}"#;
    fs::write(&path, bytes).unwrap();
    let (mut store, config) = ConfigStore::load(root.0.clone());
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .open(&path)
        .unwrap();
    assert!(store.save(&config, false).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    drop(lock);
    fs::write(&path, br#"{"schema_version":1,"initial_scale_percent":80}"#).unwrap();
    assert!(store.save(&config, false).is_err());
    assert_ne!(fs::read(&path).unwrap(), bytes);
}
