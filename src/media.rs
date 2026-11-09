use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const MANIFEST_LIMIT: usize = 1024 * 1024;
const PAGE_LIMIT: usize = 32 * 1024 * 1024;
const AUX_LIMIT: usize = 1024 * 1024;
const MAX_SIDE: u32 = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub pack_id: String,
    pub pack_version: String,
    pub color_space: String,
    pub alpha: String,
    pub source_sha256: Vec<String>,
    pub tools: BTreeMap<String, String>,
    pub pages: Vec<Page>,
    pub actions: Vec<Action>,
    pub resources: Vec<Resource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub size: [u32; 2],
    pub png: Payload,
    pub rgba8: Payload,
    pub bc7: Option<Payload>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub path: String,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub logical_size: [u32; 2],
    pub looping: bool,
    pub anchor: [f32; 2],
    pub offset: [f32; 2],
    pub scale: f32,
    pub directions: Vec<Direction>,
    pub movement_us: Option<[u64; 2]>,
    pub frames: Vec<Frame>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub page: String,
    pub rect: [u32; 4],
    pub duration_us: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub key: String,
    pub client_id: Option<String>,
    pub content: ResourceContent,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceContent {
    Image { page: String },
    Audio { file: Payload },
    Text { file: Payload },
}

pub struct Pack {
    root: PathBuf,
    manifest: Manifest,
}

impl Pack {
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let manifest = serde_json::from_slice(&read_bounded(
            &contained_path(&root, "manifest.json")?,
            MANIFEST_LIMIT,
        )?)
        .map_err(io::Error::other)?;
        let pack = Self { root, manifest };
        pack.validate_manifest()?;
        Ok(pack)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn resource(&self, key: &str, client_id: Option<&str>) -> Option<&Resource> {
        let matches = |client| {
            self.manifest
                .resources
                .iter()
                .find(|resource| resource.key == key && resource.client_id.as_deref() == client)
        };
        matches(client_id).or_else(|| matches(None))
    }

    fn validate_manifest(&self) -> io::Result<()> {
        let m = &self.manifest;
        if m.schema_version != 1
            || m.pack_id.trim().is_empty()
            || m.pack_version.trim().is_empty()
            || m.color_space != "sRGB"
            || m.alpha != "straight"
            || m.source_sha256.is_empty()
            || !m.source_sha256.iter().all(|hash| valid_hash(hash))
            || m.tools.is_empty()
            || m.tools.values().any(|value| value.trim().is_empty())
            || m.pages.is_empty()
            || m.actions.is_empty()
        {
            return Err(invalid("unsupported schema or missing pack metadata"));
        }
        let mut pages = BTreeMap::new();
        for page in &m.pages {
            let [w, h] = page.size;
            if page.id.trim().is_empty()
                || pages.insert(page.id.as_str(), page.size).is_some()
                || !valid_size(page.size)
                || w as usize * h as usize * 4 > PAGE_LIMIT
                || (page.bc7.is_some() && (w % 4 != 0 || h % 4 != 0))
            {
                return Err(invalid(format!("invalid or duplicate page {}", page.id)));
            }
            for payload in [&page.png, &page.rgba8].into_iter().chain(&page.bc7) {
                self.validate_path(payload)?;
            }
        }
        let mut actions = BTreeSet::new();
        for action in &m.actions {
            if action.id.trim().is_empty()
                || !actions.insert(&action.id)
                || !valid_size(action.logical_size)
                || !action.scale.is_finite()
                || action.scale <= 0.0
                || action
                    .anchor
                    .iter()
                    .chain(&action.offset)
                    .any(|v| !v.is_finite())
                || action.frames.is_empty()
            {
                return Err(invalid(format!(
                    "invalid or duplicate action {}",
                    action.id
                )));
            }
            let mut duration = 0_u64;
            for frame in &action.frames {
                let Some(&[pw, ph]) = pages.get(frame.page.as_str()) else {
                    return Err(invalid(format!("unknown page {}", frame.page)));
                };
                let [x, y, w, h] = frame.rect;
                if w == 0 || h == 0 || x > pw || y > ph || w > pw - x || h > ph - y {
                    return Err(invalid("frame rectangle exceeds page bounds"));
                }
                duration = duration
                    .checked_add(frame.duration_us)
                    .filter(|_| frame.duration_us > 0)
                    .ok_or_else(|| invalid("zero or overflowing frame duration"))?;
            }
            if let Some([start, end]) = action.movement_us
                && (start >= end || end > duration)
            {
                return Err(invalid("movement window exceeds action duration"));
            }
        }
        let mut keys = BTreeSet::new();
        for resource in &m.resources {
            if resource.key.trim().is_empty()
                || resource
                    .client_id
                    .as_ref()
                    .is_some_and(|id| id.trim().is_empty())
                || !keys.insert((&resource.key, &resource.client_id))
            {
                return Err(invalid(format!(
                    "invalid or duplicate resource {}",
                    resource.key
                )));
            }
            match &resource.content {
                ResourceContent::Image { page } if !pages.contains_key(page.as_str()) => {
                    return Err(invalid(format!("unknown image page {page}")));
                }
                ResourceContent::Image { .. } => {}
                ResourceContent::Audio { file } | ResourceContent::Text { file } => {
                    self.validate_path(file)?;
                }
            }
        }
        Ok(())
    }

    fn validate_path(&self, payload: &Payload) -> io::Result<()> {
        if !valid_hash(&payload.sha256) {
            return Err(invalid("invalid SHA-256 digest"));
        }
        contained_path(&self.root, &payload.path)?;
        Ok(())
    }

    fn read(&self, payload: &Payload, limit: usize) -> io::Result<Vec<u8>> {
        let bytes = read_bounded(&contained_path(&self.root, &payload.path)?, limit)?;
        if format!("{:x}", Sha256::digest(&bytes)) != payload.sha256 {
            return Err(invalid(format!("checksum mismatch for {}", payload.path)));
        }
        Ok(bytes)
    }

    pub fn validate_payloads(&self) -> io::Result<()> {
        for page in &self.manifest.pages {
            let [w, h] = page.size;
            let rgba_bytes = w as usize * h as usize * 4;
            let png = self.read(&page.png, PAGE_LIMIT)?;
            let mut reader = png::Decoder::new(Cursor::new(png))
                .read_info()
                .map_err(io::Error::other)?;
            let info = reader.info();
            if [info.width, info.height] != page.size
                || info.color_type != png::ColorType::Rgba
                || info.bit_depth != png::BitDepth::Eight
                || info.animation_control.is_some()
            {
                return Err(invalid(
                    "expected a static RGBA8 PNG matching the page size",
                ));
            }
            let mut decoded = vec![0; rgba_bytes];
            reader.next_frame(&mut decoded).map_err(io::Error::other)?;
            reader.finish().map_err(io::Error::other)?;
            drop(reader);
            if self.read(&page.rgba8, rgba_bytes)? != decoded {
                return Err(invalid("PNG and raw RGBA8 payloads differ"));
            }
            if let Some(bc7) = &page.bc7 {
                let expected = rgba_bytes / 4;
                let data = self.read(bc7, expected)?;
                if data.len() != expected
                    || data.as_chunks::<16>().0.iter().any(|block| block[0] == 0)
                {
                    return Err(invalid("invalid BC7 block count or mode"));
                }
            }
        }
        for resource in &self.manifest.resources {
            match &resource.content {
                ResourceContent::Text { file } => {
                    std::str::from_utf8(&self.read(file, AUX_LIMIT)?).map_err(io::Error::other)?;
                }
                ResourceContent::Audio { file } => {
                    let bytes = self.read(file, AUX_LIMIT)?;
                    if !valid_pcm_wave(&bytes) {
                        return Err(invalid("expected a canonical PCM16 WAVE audio resource"));
                    }
                }
                ResourceContent::Image { .. } => {}
            }
        }
        Ok(())
    }
}

fn valid_pcm_wave(bytes: &[u8]) -> bool {
    let Some((header, data)) = bytes.split_at_checked(44) else {
        return false;
    };
    let le16 = |i| u16::from_le_bytes([header[i], header[i + 1]]);
    let le32 = |i| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let channels = le16(22);
    let rate = le32(24);
    let align = le16(32);
    &header[..4] == b"RIFF"
        && le32(4) as usize == bytes.len() - 8
        && &header[8..16] == b"WAVEfmt "
        && le32(16) == 16
        && le16(20) == 1
        && matches!(channels, 1 | 2)
        && rate > 0
        && rate.checked_mul(u32::from(align)) == Some(le32(28))
        && align == channels * 2
        && le16(34) == 16
        && &header[36..40] == b"data"
        && le32(40) as usize == data.len()
        && !data.is_empty()
        && data.len().is_multiple_of(usize::from(align))
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_size(size: [u32; 2]) -> bool {
    size.into_iter().all(|side| (1..=MAX_SIDE).contains(&side))
}

fn contained_path(root: &Path, relative: &str) -> io::Result<PathBuf> {
    if relative.split('/').any(|part| {
        part.is_empty()
            || part == "."
            || part == ".."
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    }) {
        return Err(invalid(format!("invalid pack-relative path {relative}")));
    }
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(invalid(format!(
            "payload outside pack or not a file: {relative}"
        )));
    }
    Ok(path)
}

fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid(format!(
            "payload exceeds limit: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_lookup_prefers_client_then_generic() {
        let manifest = serde_json::from_str(
            r#"{
                "schema_version":1,"pack_id":"test","pack_version":"1",
                "color_space":"sRGB","alpha":"straight","source_sha256":[],
                "tools":{},"pages":[],"actions":[],"resources":[
                    {"key":"label","content":{"kind":"image","page":"generic"}},
                    {"key":"label","client_id":"chatgpt","content":{"kind":"image","page":"specific"}}
                ]
            }"#,
        )
        .unwrap();
        let pack = Pack {
            root: PathBuf::new(),
            manifest,
        };
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
    fn rejects_absolute_and_traversing_paths_before_filesystem_access() {
        for relative in [
            "../outside",
            "sub/../outside",
            "/root",
            "C:/file",
            r"sub\file",
            "file:stream",
        ] {
            let error = contained_path(Path::new("nonexistent-pack"), relative).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{relative}");
        }
    }
}
