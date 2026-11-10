use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::{Manifest, PagePixels, Payload, Resource, ResourceContent, Variant};

const MANIFEST_LIMIT: usize = 1024 * 1024;
const PAGE_LIMIT: usize = 32 * 1024 * 1024;
const AUX_LIMIT: usize = 1024 * 1024;
const MAX_SIDE: u32 = 4096;
pub const PLAYBACK_PAGE_LIMIT: usize = 6 * 1024 * 1024;
const PNG_STAGING_LIMIT: usize = 3 * 1024 * 1024;

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
            for payload in std::iter::once(&page.png)
                .chain(&page.rgba8)
                .chain(&page.bc7)
            {
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

    pub fn load_page(
        &self,
        index: usize,
        variant: Variant,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<PagePixels> {
        let page = self
            .manifest
            .pages
            .get(index)
            .ok_or_else(|| invalid("unknown page"))?;
        let [width, height] = page.size;
        let bytes = width as usize * height as usize * 4;
        let (variant, payload) = match (variant, &page.bc7, &page.rgba8) {
            (Variant::Bc7, Some(bc7), _) => (Variant::Bc7, bc7),
            (Variant::Rgba8 | Variant::Bc7, _, Some(rgba8)) => (Variant::Rgba8, rgba8),
            #[cfg(feature = "p1b-reference")]
            (Variant::Ffmpeg, _, _) => return Err(invalid("FFmpeg needs the reference source")),
            _ => (Variant::Png, &page.png),
        };
        let extra = if variant == Variant::Bc7 {
            bytes / 4
        } else {
            0
        };
        if bytes + extra > PLAYBACK_PAGE_LIMIT {
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
        let (rgba, bc7) = if variant == Variant::Bc7 {
            let data = self.read(payload, extra)?;
            check_bc7(&data, extra)?;
            check_cancel()?;
            let mut rgba = vec![0; bytes];
            let stride = width as usize * 4;
            let blocks_per_row = width as usize / 4;
            for (y, block_row) in data.chunks_exact(blocks_per_row * 16).enumerate() {
                check_cancel()?;
                for (x, block) in block_row.as_chunks::<16>().0.iter().enumerate() {
                    let offset = y * 4 * stride + x * 16;
                    bcdec_rs::bc7(block, &mut rgba[offset..], stride);
                }
            }
            (rgba, Some(data))
        } else if variant == Variant::Png {
            let png = self.read(payload, PNG_STAGING_LIMIT)?;
            check_cancel()?;
            (decode_png(png, page.size, PNG_STAGING_LIMIT)?, None)
        } else {
            let rgba = self.read(payload, bytes)?;
            if rgba.len() != bytes {
                return Err(invalid("invalid RGBA8 page length"));
            }
            (rgba, None)
        };
        check_cancel()?;
        Ok(PagePixels {
            size: page.size,
            rgba,
            bc7,
            variant,
        })
    }

    pub fn validate_payloads(&self) -> io::Result<()> {
        for page in &self.manifest.pages {
            let [w, h] = page.size;
            let rgba_bytes = w as usize * h as usize * 4;
            let png = self.read(&page.png, PAGE_LIMIT)?;
            let decoded = decode_png(png, page.size, PAGE_LIMIT)?;
            if let Some(rgba8) = &page.rgba8
                && self.read(rgba8, rgba_bytes)? != decoded
            {
                return Err(invalid("PNG and raw RGBA8 payloads differ"));
            }
            if let Some(bc7) = &page.bc7 {
                let expected = rgba_bytes / 4;
                let data = self.read(bc7, expected)?;
                check_bc7(&data, expected)?;
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

fn decode_png(data: Vec<u8>, size: [u32; 2], limit: usize) -> io::Result<Vec<u8>> {
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

fn check_bc7(data: &[u8], expected: usize) -> io::Result<()> {
    if data.len() != expected || data.as_chunks::<16>().0.iter().any(|block| block[0] == 0) {
        return Err(invalid("invalid BC7 block count or mode"));
    }
    Ok(())
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
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > limit as u64 {
        return Err(invalid(format!(
            "payload exceeds limit: {}",
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

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;
