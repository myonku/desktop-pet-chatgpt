mod manifest;
mod pack;

#[cfg(all(windows, feature = "ffmpeg-decoder"))]
mod ffmpeg;
#[cfg(windows)]
pub(crate) mod frame;
#[cfg(windows)]
pub(crate) mod playback;

pub use manifest::{Action, Direction, Frame, Manifest, Page, Payload, Resource, ResourceContent};
pub use pack::{LoadedResource, PLAYBACK_PAGE_LIMIT, Pack};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Png,
    Rgba8,
    Bc7,
    #[cfg(feature = "ffmpeg-decoder")]
    Ffmpeg,
}

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Rgba8 => "RGBA8",
            Self::Bc7 => "BC7",
            #[cfg(feature = "ffmpeg-decoder")]
            Self::Ffmpeg => "FFMPEG",
        }
    }
}

pub struct PagePixels {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
    pub bc7: Option<Vec<u8>>,
    pub variant: Variant,
}
