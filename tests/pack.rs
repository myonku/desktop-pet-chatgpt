use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use desktop_pet_chatgpt::{
    core::{ActionChoice, Behavior},
    media::Pack,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn parser_test_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 128, 128);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[128; 128 * 128 * 4])
        .unwrap();
    bytes
}

fn bind(clip: &str) -> serde_json::Value {
    json!({"default":"base", "variants":{"base":clip}})
}

#[test]
fn icon_payload_is_optional_and_rejects_invalid_directory_entries() {
    use desktop_pet_chatgpt::media::LoadedResource;

    let fixture = Fixture::create();
    assert!(
        fixture
            .open()
            .load_resource("app.icon", None)
            .unwrap()
            .is_none()
    );
    let png = parser_test_png();
    let mut ico = vec![0, 0, 1, 0, 1, 0, 128, 128, 0, 0, 1, 0, 32, 0];
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22_u32.to_le_bytes());
    ico.extend_from_slice(&png);
    let manifest_path = fixture.0.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    for valid in [true, false] {
        if !valid {
            ico[18..22].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        let digest = format!("{:x}", Sha256::digest(&ico));
        manifest["resources"][0] = json!({
            "key": "app.icon", "content": {"kind": "icon",
            "file": {"path": "app.ico", "sha256": digest}}
        });
        fs::write(&manifest_path, manifest.to_string()).unwrap();
        fs::write(fixture.0.join("app.ico"), &ico).unwrap();
        let pack = fixture.open();
        assert_eq!(pack.validate_payloads().is_ok(), valid);
        if valid {
            let Some(LoadedResource::Icon(icon)) = pack.load_resource("app.icon", None).unwrap()
            else {
                panic!("missing icon");
            };
            assert_eq!(icon.png_for_size(32), png);
        } else {
            assert!(pack.load_resource("app.icon", None).is_err());
        }
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn create() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "desktop-pet-pack-{}-{}-{id}",
            std::process::id(),
            timestamp.as_nanos(),
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        let png = parser_test_png();
        fs::write(fixture.0.join("sample.png"), &png).unwrap();
        let digest = format!("{:x}", Sha256::digest(&png));
        let manifest = json!({
            "schema_version": 2,
            "engine_api": 2, "required_features": [], "encoding": "png",
            "pack_id": "png-only",
            "pack_version": "0.1.0",
            "color_space": "sRGB",
            "alpha": "straight",
            "source_sha256": [&digest],
            "tools": {"fixture": "codec-test"},
            "pages": [{"id": "first", "size": [128, 128], "payload": {
                "path": "sample.png", "sha256": digest, "bytes": png.len(),
            }}],
            "clips": [{
                "id": "idle", "logical_size": [128, 128], "looping": true,
                "anchor": [0.5, 1], "offset": [0, 0], "scale": 1, "directions": [],
                "frames": [{"page": "first", "rect": [0, 0, 128, 128], "duration_us": 33333}],
            }],
            "bindings": {"idle": bind("idle")},
            "resources": [
                {"key":"label","content":{"kind":"image","page":"first"}},
                {"key":"label","client_id":"chatgpt","content":{"kind":"image","page":"first"}}
            ],
        });
        fs::write(fixture.0.join("manifest.json"), manifest.to_string()).unwrap();
        fixture
    }

    fn open(&self) -> Pack {
        Pack::open(&self.0).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn png_pages_validate_and_decode() {
    let pack = Fixture::create();
    let pack = pack.open();
    pack.validate_payloads().unwrap();
    assert_eq!(
        pack.load_page(0, || false).unwrap().rgba.len(),
        128 * 128 * 4
    );
}

#[test]
fn resource_lookup_prefers_client_then_generic() {
    let fixture = Fixture::create();
    let pack = fixture.open();
    assert_eq!(
        pack.resource("label", Some("chatgpt"))
            .unwrap()
            .client_id
            .as_deref(),
        Some("chatgpt"),
    );
    assert!(
        pack.resource("label", Some("dsh"))
            .unwrap()
            .client_id
            .is_none()
    );
    assert!(pack.resource("missing", Some("chatgpt")).is_none());
}

#[test]
fn v2_bindings_are_independent_of_clip_ids_and_page_lengths_are_checked() {
    let fixture = Fixture::create();
    let path = fixture.0.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["clips"][0]["id"] = json!("rest");
    manifest["bindings"] = json!({"idle": bind("rest"), "interact": bind("rest")});
    let mut extra = manifest["clips"][0].clone();
    extra["id"] = json!("idle");
    manifest["clips"].as_array_mut().unwrap().insert(0, extra);
    let write = |manifest: &serde_json::Value| fs::write(&path, manifest.to_string()).unwrap();
    write(&manifest);
    let pack = fixture.open();
    assert_eq!(pack.startup_action(), 1);
    assert_eq!(
        pack.action_index(Behavior::Interact, ActionChoice::Default),
        Some(1)
    );
    assert_eq!(
        pack.action_index(Behavior::Drag, ActionChoice::Default),
        None
    );
    assert_eq!(pack.manifest().clips[0].id, "idle");
    pack.validate_payloads().unwrap();
    for (key, value) in [
        ("schema_version", json!(1)),
        ("schema_version", json!(3)),
        ("engine_api", json!(1)),
        ("engine_api", json!(3)),
        ("pack_version", json!("1")),
        ("encoding", json!("bc7")),
        ("required_features", json!(["future"])),
        (
            "bindings",
            json!({"idle":bind("rest"), "drag":bind("missing")}),
        ),
        (
            "bindings",
            json!({"idle":bind("rest"), "move.left":bind("rest")}),
        ),
        (
            "bindings",
            json!({"idle":bind("rest"), "unknown":bind("rest")}),
        ),
        ("bindings", json!({"interact":bind("rest")})),
    ] {
        let mut bad = manifest.clone();
        bad[key] = value;
        write(&bad);
        assert!(Pack::open(&fixture.0).is_err(), "{key}: {}", bad[key]);
    }
    manifest["pages"][0]["payload"]["bytes"] = json!(1);
    write(&manifest);
    let pack = fixture.open();
    assert!(pack.load_page(0, || false).is_err());
    assert!(pack.validate_payloads().is_err());
}

#[test]
fn semantic_variants_require_a_default_and_validate_each_movement() {
    let fixture = Fixture::create();
    let path = fixture.0.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut clip = manifest["clips"][0].clone();
    clip["id"] = json!("walk-asset");
    clip["preview_name"] = json!("向左走");
    clip["looping"] = json!(false);
    clip["movement_us"] = json!([10, 20]);
    clip["directions"] = json!(["left"]);
    let mut fly = clip.clone();
    fly["id"] = json!("flight-asset");
    let mut diagonal = clip.clone();
    diagonal["id"] = json!("diagonal-asset");
    manifest["clips"]
        .as_array_mut()
        .unwrap()
        .extend([clip, fly, diagonal]);
    manifest["bindings"]["move.left"] = json!({
        "default":"walk",
        "variants":{"walk":"walk-asset", "fly":"flight-asset"}
    });
    manifest["bindings"]["move.up-left"] = json!({
        "default":"climb",
        "variants":{"climb":"diagonal-asset"}
    });
    fs::write(&path, manifest.to_string()).unwrap();
    let pack = fixture.open();
    let behavior = Behavior::MoveLeft;
    assert_eq!(pack.action_index(behavior, ActionChoice::Default), Some(1));
    assert_eq!(
        pack.action_index(Behavior::MoveUpLeft, ActionChoice::Default),
        Some(3)
    );
    assert_eq!(
        pack.action_index(behavior, ActionChoice::Variant("fly")),
        Some(2)
    );
    assert_eq!(
        pack.action_index(behavior, ActionChoice::Variant("absent")),
        Some(1)
    );
    assert!(matches!(
        pack.action_index(behavior, ActionChoice::Random),
        Some(1 | 2)
    ));
    assert_eq!(
        pack.manifest().clips[1].preview_name.as_deref(),
        Some("向左走")
    );

    let mut missing_default = manifest.clone();
    missing_default["bindings"]["move.left"]["default"] = json!("run");
    let mut wrong_direction = manifest.clone();
    wrong_direction["clips"][2]["directions"] = json!(["right"]);
    let mut wrong_diagonal_direction = manifest.clone();
    wrong_diagonal_direction["clips"][3]["directions"] = json!(["right"]);
    let mut duplicate_clip = manifest.clone();
    duplicate_clip["bindings"]["move.left"]["variants"]["fly"] = json!("walk-asset");
    let duplicate_key = manifest.to_string().replace(
        "\"fly\":\"flight-asset\"",
        "\"fly\":\"flight-asset\",\"fly\":\"flight-asset\"",
    );
    for (text, message) in [
        (missing_default.to_string(), "unknown default variant"),
        (wrong_direction.to_string(), "move.left/fly needs"),
        (
            wrong_diagonal_direction.to_string(),
            "move.up-left/climb needs",
        ),
        (duplicate_clip.to_string(), "duplicate clip"),
        (duplicate_key, "duplicate binding key"),
    ] {
        fs::write(&path, text).unwrap();
        let Err(error) = Pack::open(&fixture.0) else {
            panic!("accepted invalid manifest");
        };
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[test]
fn feature_specific_clips_are_preview_only() {
    let fixture = Fixture::create();
    let path = fixture.0.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut clip = manifest["clips"][0].clone();
    clip["id"] = json!("work02");
    clip["preview_name"] = json!("敲键盘");
    manifest["clips"].as_array_mut().unwrap().push(clip);
    fs::write(&path, manifest.to_string()).unwrap();
    let pack = fixture.open();
    assert_eq!(
        pack.manifest().clips[1].preview_name.as_deref(),
        Some("敲键盘")
    );
    for key in [
        "work",
        "quota",
        "reaction.thinking",
        "reaction.positive",
        "reaction.negative",
    ] {
        let mut invalid = manifest.clone();
        invalid["bindings"][key] = bind("work02");
        fs::write(&path, invalid.to_string()).unwrap();
        let Err(error) = Pack::open(&fixture.0) else {
            panic!("accepted feature-specific binding {key}");
        };
        assert!(
            error
                .to_string()
                .contains(&format!("unknown asset behavior {key}"))
        );
    }
}

#[test]
fn validator_requires_a_looping_idle_binding_and_rejects_schema_one() {
    let fixture = Fixture::create();
    let path = fixture.0.join("manifest.json");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (key, value) in [("looping", json!(false)), ("schema_version", json!(1))] {
        let mut invalid = manifest.clone();
        if key == "looping" {
            invalid["clips"][0]["looping"] = value;
        } else {
            invalid[key] = value;
        }
        fs::write(&path, invalid.to_string()).unwrap();
        for mode in ["--playback", "--distribution"] {
            let result = std::process::Command::new(env!("CARGO_BIN_EXE_validate-pack"))
                .arg(mode)
                .arg(&fixture.0)
                .output()
                .unwrap();
            assert!(!result.status.success());
            if key == "schema_version" {
                assert!(
                    String::from_utf8_lossy(&result.stderr)
                        .contains("unsupported asset pack schema 1")
                );
            }
        }
    }
}

#[test]
fn optional_resources_fail_independently_at_runtime_and_fail_strict_validation() {
    let fixture = Fixture::create();
    let manifest_path = fixture.0.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert!(
        fixture
            .open()
            .load_resource("character.name", None)
            .unwrap()
            .is_none()
    );
    let digest = format!("{:x}", Sha256::digest([0xff]));
    manifest["resources"][0] = json!({
        "key": "character.name", "content": {"kind": "text",
        "file": {"path": "name.txt", "sha256": digest}}
    });
    fs::write(&manifest_path, manifest.to_string()).unwrap();
    let pack = fixture.open();
    assert!(pack.load_resource("character.name", None).is_err());
    assert!(pack.validate_payloads().is_err());
    for bytes in [&b"wrong digest"[..], &[0xff][..]] {
        fs::write(fixture.0.join("name.txt"), bytes).unwrap();
        let pack = fixture.open();
        assert!(pack.load_resource("character.name", None).is_err());
        assert!(pack.validate_payloads().is_err());
    }
    let mut poster = manifest["pages"][0].clone();
    poster["id"] = json!("poster");
    poster["payload"]["path"] = json!("missing-poster.png");
    manifest["pages"].as_array_mut().unwrap().push(poster);
    manifest["resources"][0] = json!({
        "key": "character.poster", "content": {"kind": "image", "page": "poster"}
    });
    fs::write(&manifest_path, manifest.to_string()).unwrap();
    let pack = fixture.open();
    assert!(pack.load_resource("character.poster", None).is_err());
    assert!(pack.validate_payloads().is_err());
    manifest["clips"][0]["frames"][0]["page"] = json!("poster");
    fs::write(&manifest_path, manifest.to_string()).unwrap();
    assert!(Pack::open(&fixture.0).is_err());
}
