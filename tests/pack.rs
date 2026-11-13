use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use desktop_pet_chatgpt::media::{Pack, Variant};
use serde_json::json;
use sha2::{Digest, Sha256};

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
        let png = include_bytes!("../src/render/alpha-color.png");
        fs::write(fixture.0.join("sample.png"), png).unwrap();
        let digest = format!("{:x}", Sha256::digest(png));
        let manifest = json!({
            "schema_version": 1,
            "pack_id": "png-only",
            "pack_version": "1",
            "color_space": "sRGB",
            "alpha": "straight",
            "source_sha256": [&digest],
            "tools": {"fixture": "embedded"},
            "pages": [{"id": "first", "size": [128, 128], "png": {
                "path": "sample.png", "sha256": digest,
            }}],
            "actions": [{
                "id": "idle", "logical_size": [128, 128], "looping": true,
                "anchor": [0.5, 1], "offset": [0, 0], "scale": 1, "directions": [],
                "frames": [{"page": "first", "rect": [0, 0, 128, 128], "duration_us": 33333}],
            }],
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
fn png_only_pages_validate_and_supply_both_compatibility_fallbacks() {
    let fixture = Fixture::create();
    let pack = fixture.open();
    pack.validate_payloads().unwrap();
    let baseline = pack.load_page(0, Variant::Png, || false).unwrap();
    for requested in [Variant::Rgba8, Variant::Bc7] {
        let actual = pack.load_page(0, requested, || false).unwrap();
        assert_eq!(actual.variant, Variant::Png);
        assert_eq!(actual.rgba, baseline.rgba);
        assert!(actual.bc7.is_none());
    }
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
fn distribution_requires_looping_idle_but_playback_accepts_reference() {
    let fixture = Fixture::create();
    let manifest_path = fixture.0.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    for (id, looping, playback, distribution) in [
        ("idle", true, true, true),
        ("idle", false, false, false),
        ("interact", false, false, false),
        ("reference", true, true, false),
    ] {
        manifest["actions"][0]["id"] = json!(id);
        manifest["actions"][0]["looping"] = json!(looping);
        fs::write(&manifest_path, manifest.to_string()).unwrap();
        let run = |mode| {
            std::process::Command::new(env!("CARGO_BIN_EXE_validate-pack"))
                .arg(mode)
                .arg(&fixture.0)
                .output()
                .unwrap()
                .status
                .success()
        };
        assert_eq!(run("--playback"), playback, "{id}, looping={looping}");
        assert_eq!(
            run("--distribution"),
            distribution,
            "{id}, looping={looping}"
        );
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
    poster["png"]["path"] = json!("missing-poster.png");
    manifest["pages"].as_array_mut().unwrap().push(poster);
    manifest["resources"][0] = json!({
        "key": "character.poster", "content": {"kind": "image", "page": "poster"}
    });
    fs::write(&manifest_path, manifest.to_string()).unwrap();
    let pack = fixture.open();
    assert!(pack.load_resource("character.poster", None).is_err());
    assert!(pack.validate_payloads().is_err());
    manifest["actions"][0]["frames"][0]["page"] = json!("poster");
    fs::write(&manifest_path, manifest.to_string()).unwrap();
    assert!(Pack::open(&fixture.0).is_err());
}
