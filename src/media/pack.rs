use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::core::{ActionChoice, Behavior};

use super::manifest::ActionBindings;
use super::{Manifest, PackIcon, PagePixels, Payload, Resource, ResourceContent};

const MANIFEST_LIMIT: usize = 1024 * 1024;
const PAGE_LIMIT: usize = 32 * 1024 * 1024;
const AUX_LIMIT: usize = 1024 * 1024;
const MAX_SIDE: u32 = 4096;
pub const PLAYBACK_PAGE_LIMIT: usize = 6 * 1024 * 1024;
const PNG_STAGING_LIMIT: usize = 3 * 1024 * 1024;

pub struct Pack {
    root: PathBuf,
    manifest: Manifest,
    bindings: ActionBindings,
}

pub enum LoadedResource {
    Image(PagePixels),
    Audio(Vec<u8>),
    Icon(PackIcon),
    Text(String),
}
impl Pack {
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let manifest: Manifest = serde_json::from_slice(&read_bounded(
            &contained_path(&root, "manifest.json")?,
            MANIFEST_LIMIT,
        )?)
        .map_err(|error| invalid(error.to_string()))?;
        let bindings = manifest.action_bindings()?;
        let pack = Self {
            root,
            manifest,
            bindings,
        };
        pack.validate_manifest()?;
        Ok(pack)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn startup_action(&self) -> usize {
        self.bindings[&Behavior::Idle].default
    }

    pub fn action_index(&self, behavior: Behavior, choice: ActionChoice<'_>) -> Option<usize> {
        self.bindings.get(&behavior)?.select(choice, |_| true)
    }

    pub(super) fn bindings(&self) -> &ActionBindings {
        &self.bindings
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

    pub fn load_resource(
        &self,
        key: &str,
        client_id: Option<&str>,
    ) -> io::Result<Option<LoadedResource>> {
        let Some(resource) = self.resource(key, client_id) else {
            return Ok(None);
        };
        let loaded = match &resource.content {
            ResourceContent::Image { page } => {
                let index = self
                    .manifest
                    .pages
                    .iter()
                    .position(|item| item.id == *page)
                    .ok_or_else(|| invalid("unknown image page"))?;
                LoadedResource::Image(self.load_page(index, || false)?)
            }
            ResourceContent::Audio { file } => {
                let bytes = self.read(file, AUX_LIMIT)?;
                if !valid_pcm_wave(&bytes) {
                    return Err(invalid("expected a canonical PCM16 WAVE audio resource"));
                }
                LoadedResource::Audio(bytes)
            }
            ResourceContent::Text { file } => LoadedResource::Text(
                String::from_utf8(self.read(file, AUX_LIMIT)?).map_err(io::Error::other)?,
            ),
            ResourceContent::Icon { file } => {
                LoadedResource::Icon(PackIcon::parse(self.read(file, AUX_LIMIT)?)?)
            }
        };
        Ok(Some(loaded))
    }
    fn validate_manifest(&self) -> io::Result<()> {
        let m = &self.manifest;
        if m.pack_id.trim().is_empty()
            || m.pack_version.trim().is_empty()
            || m.color_space != "sRGB"
            || m.alpha != "straight"
            || m.source_sha256.is_empty()
            || !m.source_sha256.iter().all(|hash| valid_hash(hash))
            || m.tools.is_empty()
            || m.tools.values().any(|value| value.trim().is_empty())
            || m.pages.is_empty()
            || m.clips.is_empty()
        {
            return Err(invalid("unsupported schema or missing pack metadata"));
        }
        let clip_pages: BTreeSet<_> = m
            .clips
            .iter()
            .flat_map(|clip| clip.frames.iter().map(|frame| frame.page.as_str()))
            .collect();
        let mut pages = BTreeMap::new();
        for page in &m.pages {
            let [w, h] = page.size;
            if page.id.trim().is_empty()
                || pages.insert(page.id.as_str(), page.size).is_some()
                || !valid_size(page.size)
                || w as usize * h as usize * 4 > PAGE_LIMIT
                || page.payload.bytes == 0
                || page.payload.bytes > PAGE_LIMIT as u64
            {
                return Err(invalid(format!("invalid or duplicate page {}", page.id)));
            }
            validate_payload_descriptor(&page.payload.path, &page.payload.sha256)?;
            if clip_pages.contains(page.id.as_str()) {
                contained_path(&self.root, &page.payload.path)?;
            }
        }
        let mut actions = BTreeSet::new();
        for action in &m.clips {
            if action.id.trim().is_empty()
                || !actions.insert(&action.id)
                || action.preview_name.as_ref().is_some_and(|name| {
                    name.trim().is_empty()
                        || name.chars().count() > 24
                        || name.chars().any(char::is_control)
                })
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
            if matches!(resource.key.as_str(), "app.icon" | "tray.icon")
                && (resource.client_id.is_some()
                    || !matches!(resource.content, ResourceContent::Icon { .. }))
            {
                return Err(invalid(format!(
                    "{} must be a generic icon resource",
                    resource.key
                )));
            }
            match &resource.content {
                ResourceContent::Image { page } if !pages.contains_key(page.as_str()) => {
                    return Err(invalid(format!("unknown image page {page}")));
                }
                ResourceContent::Image { .. } => {}
                ResourceContent::Audio { file }
                | ResourceContent::Icon { file }
                | ResourceContent::Text { file } => {
                    validate_payload_descriptor(&file.path, &file.sha256)?;
                    if matches!(resource.content, ResourceContent::Icon { .. })
                        && !file.path.ends_with(".ico")
                    {
                        return Err(invalid("icon payload path must end in .ico"));
                    }
                }
            }
        }
        Ok(())
    }

    fn read(&self, payload: &Payload, limit: usize) -> io::Result<Vec<u8>> {
        self.read_payload(&payload.path, &payload.sha256, limit, None)
    }

    fn read_payload(
        &self,
        path: &str,
        sha256: &str,
        limit: usize,
        expected_size: Option<u64>,
    ) -> io::Result<Vec<u8>> {
        let bytes = read_bounded_exact(&contained_path(&self.root, path)?, limit, expected_size)?;
        if format!("{:x}", Sha256::digest(&bytes)) != sha256 {
            return Err(invalid(format!("checksum mismatch for {path}")));
        }
        Ok(bytes)
    }

    fn read_page(&self, page: &super::Page, limit: usize) -> io::Result<Vec<u8>> {
        self.read_payload(
            &page.payload.path,
            &page.payload.sha256,
            limit,
            Some(page.payload.bytes),
        )
    }

    pub fn load_page(&self, index: usize, cancelled: impl Fn() -> bool) -> io::Result<PagePixels> {
        let page = self
            .manifest
            .pages
            .get(index)
            .ok_or_else(|| invalid("unknown page"))?;
        let [width, height] = page.size;
        if width as usize * height as usize * 4 > PLAYBACK_PAGE_LIMIT {
            return Err(invalid("page exceeds playback staging budget"));
        }
        let check_cancel = || {
            if cancelled() {
                Err(io::Error::new(io::ErrorKind::Interrupted, "page cancelled"))
            } else {
                Ok(())
            }
        };
        check_cancel()?;
        let png = self.read_page(page, PNG_STAGING_LIMIT)?;
        check_cancel()?;
        let rgba = decode_png(png, page.size, PNG_STAGING_LIMIT)?;
        check_cancel()?;
        Ok(PagePixels {
            size: page.size,
            rgba,
        })
    }

    pub fn validate_payloads(&self) -> io::Result<()> {
        self.validate_all(false)
    }

    pub fn validate_playback(&self) -> io::Result<()> {
        self.validate_all(true)
    }

    fn validate_all(&self, playback: bool) -> io::Result<()> {
        for (index, page) in self.manifest.pages.iter().enumerate() {
            if playback {
                self.load_page(index, || false)?;
            } else {
                decode_png(self.read_page(page, PAGE_LIMIT)?, page.size, PAGE_LIMIT)?;
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
                ResourceContent::Icon { file } => {
                    PackIcon::parse(self.read(file, AUX_LIMIT)?)?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn decode_png(data: Vec<u8>, size: [u32; 2], limit: usize) -> io::Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_limits(png::Limits { bytes: limit });
    let mut reader = decoder.read_info().map_err(io::Error::other)?;
    let info = reader.info();
    if [info.width, info.height] != size
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
        || info.animation_control.is_some()
    {
        return Err(invalid(
            "expected a static RGBA8 PNG matching the page size",
        ));
    }
    let mut decoded = vec![0; size[0] as usize * size[1] as usize * 4];
    reader.next_frame(&mut decoded).map_err(io::Error::other)?;
    reader.finish().map_err(io::Error::other)?;
    Ok(decoded)
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

fn validate_payload_descriptor(path: &str, sha256: &str) -> io::Result<()> {
    if !valid_hash(sha256) {
        return Err(invalid("invalid SHA-256 digest"));
    }
    validate_relative_path(path)
}

fn validate_relative_path(relative: &str) -> io::Result<()> {
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
    Ok(())
}

fn contained_path(root: &Path, relative: &str) -> io::Result<PathBuf> {
    validate_relative_path(relative)?;
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(invalid(format!(
            "payload outside pack or not a file: {relative}"
        )));
    }
    Ok(path)
}

fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    read_bounded_exact(path, limit, None)
}

fn read_bounded_exact(
    path: &Path,
    limit: usize,
    expected_size: Option<u64>,
) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > limit as u64 {
        return Err(invalid(format!(
            "payload exceeds limit: {}",
            path.display()
        )));
    }
    if expected_size.is_some_and(|expected| expected != size) {
        return Err(invalid(format!(
            "payload length mismatch: {}",
            path.display()
        )));
    }
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)?;
    if file.read(&mut [0])? != 0 {
        return Err(invalid("payload grew during read"));
    }
    Ok(bytes)
}

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;
