use std::io;

use super::pack::{decode_png, invalid};

/// A Windows ICO whose entries are bounded, static straight-alpha RGBA8 PNGs.
pub struct PackIcon {
    bytes: Vec<u8>,
    entries: Vec<(u32, std::ops::Range<usize>)>,
}

impl PackIcon {
    pub(super) fn parse(bytes: Vec<u8>) -> io::Result<Self> {
        let fail = || invalid("expected an ICO with square, unique RGBA8 PNG entries (16–256 px)");
        let header = bytes.get(..6).ok_or_else(fail)?;
        if header[..4] != [0, 0, 1, 0] {
            return Err(fail());
        }
        let count = usize::from(u16::from_le_bytes([header[4], header[5]]));
        let table_end = 6 + 16 * count;
        if !(1..=8).contains(&count) || table_end > bytes.len() {
            return Err(fail());
        }
        let mut entries: Vec<(u32, std::ops::Range<usize>)> = Vec::with_capacity(count);
        for entry in bytes[6..table_end].as_chunks::<16>().0 {
            let size = if entry[0] == 0 {
                256
            } else {
                u32::from(entry[0])
            };
            let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            let end = start.checked_add(len).ok_or_else(fail)?;
            if entry[0] != entry[1]
                || size < 16
                || entry[2..8] != [0, 0, 1, 0, 32, 0]
                || start < table_end
                || end > bytes.len()
                || entries
                    .iter()
                    .any(|(other, range)| *other == size || start < range.end && range.start < end)
            {
                return Err(fail());
            }
            decode_png(bytes[start..end].to_vec(), [size, size], 1024 * 1024)?;
            entries.push((size, start..end));
        }
        Ok(Self { bytes, entries })
    }

    pub fn png_for_size(&self, size: u32) -> &[u8] {
        let (_, range) = self.entry(size);
        &self.bytes[range.clone()]
    }

    #[cfg(windows)]
    pub(crate) fn rgba_for_size(&self, size: u32) -> io::Result<(u32, Vec<u8>)> {
        let (side, range) = self.entry(size);
        Ok((
            *side,
            decode_png(self.bytes[range.clone()].to_vec(), [*side; 2], 1024 * 1024)?,
        ))
    }

    fn entry(&self, size: u32) -> &(u32, std::ops::Range<usize>) {
        self.entries
            .iter()
            .min_by_key(|(side, _)| (*side < size, side.abs_diff(size)))
            .unwrap()
    }
}
