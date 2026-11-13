use std::os::windows::fs::OpenOptionsExt;

use serde_json::{Value, json};
use windows::Win32::System::Registry::{REG_DWORD, REG_QWORD, REG_SZ};

use super::*;
use crate::storage::{ConfigStore, tests::Profile};

#[test]
fn config_import_preserves_source_and_retries_before_and_after_publication() {
    let profile = Profile::new();
    let source = profile.0.join("config.json");
    let original = b"\xef\xbb\xbf{\"schema_version\":1,\"pack_path\":\"packs/test\",\"saved_size\":{\"base_dip\":240,\"percent\":110},\"preferences\":{\"sound\":false}}";
    fs::write(&source, original).unwrap();
    let import_config = || import(&profile.0, Source::ConfigJson(source.clone())).unwrap();
    let first = import_config();
    assert!(!first.already_applied);
    let file = profile.0.join(CONFIG_FILE);
    let journal = profile.0.join("backups/migrations").join(&first.id);
    let (_, config) = ConfigStore::load(file.clone(), false);
    let config = config.unwrap();
    assert_eq!(config.pack_path, Some(PathBuf::from("packs/test")));
    assert_eq!(config.saved_size.unwrap().dip(), 264.0);
    assert!(!config.preferences.sound);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(journal.join("source.json")).unwrap(), original);
    let generated = fs::read(&file).unwrap();
    assert!(
        !std::str::from_utf8(&generated)
            .unwrap()
            .contains("start_hidden")
    );

    fs::remove_file(journal.join("applied")).unwrap();
    assert!(import_config().already_applied);
    assert!(journal.join("applied").is_file());
    fs::remove_file(journal.join("applied")).unwrap();
    fs::remove_file(&file).unwrap();
    assert!(!import_config().already_applied);
    assert_eq!(fs::read(&file).unwrap(), generated);
    assert_eq!(fs::read_dir(journal.parent().unwrap()).unwrap().count(), 1);

    assert!(rollback(&profile.0, &first.id).unwrap());
    assert!(!rollback(&profile.0, &first.id).unwrap());
    assert!(!file.exists());
    assert_eq!(fs::read(journal.join("rolled-back")).unwrap(), generated);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert!(import(&profile.0, Source::ConfigJson(source)).is_err());
}

#[test]
fn config_import_refuses_collisions_and_rollback_refuses_edits_or_corrupt_backup() {
    let root = Profile::new();
    let source = root.0.join("config.json");
    let current = root.0.join(CONFIG_FILE);
    fs::write(&source, br#"{"schema_version":1}"#).unwrap();
    fs::write(&current, b"broken but existing").unwrap();
    let run = || import(&root.0, Source::ConfigJson(source.clone()));
    assert_eq!(run().err().unwrap().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&current).unwrap(), b"broken but existing");
    fs::remove_file(&current).unwrap();
    let id = run().unwrap().id;
    let migrated = fs::read(&current).unwrap();
    fs::write(&current, b"schema_version = 1\npack_path = 'edited'").unwrap();
    assert!(run().is_err());
    assert!(rollback(&root.0, &id).is_err());
    assert!(fs::read_to_string(&current).unwrap().contains("edited"));
    fs::write(&current, &migrated).unwrap();
    fs::write(
        root.0
            .join("backups/migrations")
            .join(&id)
            .join("source.json"),
        b"changed",
    )
    .unwrap();
    assert!(rollback(&root.0, &id).is_err());
    assert_eq!(fs::read(&current).unwrap(), migrated);
    assert!(rollback(&root.0, "../..").is_err());
}

#[test]
fn config_import_from_elsewhere_keeps_the_original_relative_pack_location() {
    let profile = Profile::new();
    let old = Profile::new();
    let source = old.0.join("config.json");
    fs::write(&source, br#"{"schema_version":1,"pack_path":"packs/dawn"}"#).unwrap();
    import(&profile.0, Source::ConfigJson(source)).unwrap();
    let (_, config) = ConfigStore::load(profile.0.join(CONFIG_FILE), false);
    assert_eq!(
        config.unwrap().pack_path.unwrap(),
        old.0.canonicalize().unwrap().join("packs/dawn")
    );
}

#[test]
fn atomic_publication_does_not_replace_an_existing_destination() {
    let root = Profile::new();
    let staged = root.0.join("staged");
    let current = root.0.join("current");
    fs::write(&staged, b"new").unwrap();
    fs::write(&current, b"existing").unwrap();
    assert!(move_new(&staged, &current).is_err());
    assert_eq!(fs::read(staged).unwrap(), b"new");
    assert_eq!(fs::read(current).unwrap(), b"existing");
}

#[test]
fn malformed_inputs_are_backed_up_and_corrected_inputs_can_be_imported() {
    let root = Profile::new();
    let source = root.0.join("config.json");
    let invalid = [
        br#"{"schema_version":2}"#.as_slice(),
        br#"{"preferences":{"sound":"CONFIDENTIAL_FIXTURE"}}"#,
        br#"{"schema_version":1,"development":{"trace_playback":true}}"#,
    ];
    for bytes in invalid {
        fs::write(&source, bytes).unwrap();
        let error = import(&root.0, Source::ConfigJson(source.clone()))
            .err()
            .unwrap();
        assert!(!error.to_string().contains("CONFIDENTIAL_FIXTURE"));
        assert!(!root.0.join(CONFIG_FILE).exists());
        assert_eq!(fs::read(&source).unwrap(), bytes);
    }
    assert_eq!(
        fs::read_dir(root.0.join("backups/migrations"))
            .unwrap()
            .count(),
        invalid.len()
    );
    fs::write(&source, br#"{"schema_version":1}"#).unwrap();
    let held = fs::OpenOptions::new()
        .write(true)
        .share_mode(0)
        .open(&source)
        .unwrap();
    assert!(import(&root.0, Source::ConfigJson(source.clone())).is_err());
    assert!(!root.0.join(CONFIG_FILE).exists());
    drop(held);
    assert!(import(&root.0, Source::ConfigJson(source)).is_ok());
}

#[test]
fn qsettings_values_map_to_product_preferences() {
    let root = Profile::new();
    let flag = |value: &str| {
        (
            REG_SZ.0,
            value
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        )
    };
    let values = BTreeMap::from([
        (
            "sizePercent".to_owned(),
            (REG_DWORD.0, 130_i32.to_le_bytes().to_vec()),
        ),
        (
            "bubblePercent".to_owned(),
            (REG_QWORD.0, 155_i64.to_le_bytes().to_vec()),
        ),
        ("taskBubblesDisabled".to_owned(), flag("true")),
        ("soundDisabled".to_owned(), flag("true")),
        ("autoHideFullscreen".to_owned(), flag("false")),
        ("autoMovementDisabled".to_owned(), flag("true")),
    ]);
    let result = import(&root.0, Source::BailongPreferences(values.clone())).unwrap();
    let (_, config) = ConfigStore::load(root.0.join(CONFIG_FILE), false);
    let config = config.unwrap();
    assert_eq!(config.initial_scale_percent, Some(130));
    assert_eq!(config.saved_size, None);
    assert_eq!(config.preferences.bubble_scale_percent, 150);
    assert!(!config.preferences.auto_move);
    assert!(!config.preferences.fullscreen_hide);
    assert!(!config.preferences.sound);
    assert!(!config.preferences.task_bubbles);
    let snapshot = root
        .0
        .join("backups/migrations")
        .join(result.id)
        .join("source.json");
    assert_eq!(
        decode_json::<LegacyPreferences>(&fs::read(snapshot).unwrap(), "").unwrap(),
        values
    );
}

#[test]
fn history_import_isolated_by_provider_and_session_and_rolls_back_as_one_directory() {
    let root = Profile::new();
    let source = root.0.join("conversations.json");
    let record = |id: &str, status: &str| {
        json!({
            "id": id, "title": "对话", "created": "2026-11-01T10:00:00", "cwd": "C:/工作区",
            "threadId": "display-only", "mode": "work", "turns": [
                {"status": status, "prompt": "输入", "output": "已有输出", "events": ["记录"]}
            ]
        })
    };
    let bad = serde_json::to_vec(&vec![
        record("../escape", "running"),
        record("../escape", "completed"),
    ])
    .unwrap();
    fs::write(&source, &bad).unwrap();
    let run = || import(&root.0, Source::BailongHistory(source.clone()));
    assert!(run().is_err());
    let provider = root.0.join("history").join(HISTORY_PROVIDER);
    assert!(!provider.exists());
    let good = serde_json::to_vec(&vec![
        record("../escape", "running"),
        record("second", "completed"),
    ])
    .unwrap();
    fs::write(&source, &good).unwrap();
    let id = run().unwrap().id;
    assert!(run().unwrap().already_applied);
    let file = provider.join(format!("{}.json", digest(b"../escape")));
    let snapshot: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    assert_eq!(snapshot["provider_id"], HISTORY_PROVIDER);
    assert_eq!(snapshot["legacy"]["id"], "../escape");
    let turn = &snapshot["legacy"]["turns"][0];
    assert_eq!(turn["status"], "interrupted");
    assert_eq!(turn["legacy_status"], "running");
    assert_eq!(turn["output"], "已有输出");
    assert_eq!(fs::read_dir(&provider).unwrap().count(), 3);
    fs::write(provider.join("added.json"), b"{}").unwrap();
    assert!(rollback(&root.0, &id).is_err());
    fs::remove_file(provider.join("added.json")).unwrap();
    assert!(rollback(&root.0, &id).unwrap());
    assert!(!provider.exists());
    assert!(!root.0.join("escape.json").exists());
    assert_eq!(fs::read(source).unwrap(), good);
}
