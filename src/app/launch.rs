#[cfg(any(feature = "development", feature = "window-diagnostics"))]
use std::env;
use std::{io, path::PathBuf};

use crate::media::Variant;
#[cfg(feature = "development")]
use crate::storage::AssetFormat;

use super::config::StartupConfig;

mod geometry;
pub(super) use geometry::window_geometry;

pub(super) struct LaunchOptions {
    pub startup: StartupConfig,
    pub variant: Variant,
    pub trace_playback: bool,
    pub trace_input: bool,
}

impl LaunchOptions {
    pub fn load(root: PathBuf) -> io::Result<Self> {
        let startup = StartupConfig::load(root)?;
        #[cfg(feature = "development")]
        let (variant, trace_playback, trace_input) = {
            let dev = startup.config.development.as_ref();
            let variant = match dev.and_then(|dev| dev.format.as_ref()) {
                Some(AssetFormat::Png) => Variant::Png,
                Some(AssetFormat::Rgba8) => Variant::Rgba8,
                Some(AssetFormat::Bc7) => Variant::Bc7,
                #[cfg(feature = "ffmpeg-decoder")]
                Some(AssetFormat::Ffmpeg) => Variant::Ffmpeg,
                None => development_format(),
            };
            (
                variant,
                dev.and_then(|dev| dev.trace_playback)
                    .unwrap_or_else(|| env::var_os("DESKTOP_PET_TRACE").is_some()),
                dev.and_then(|dev| dev.trace_input)
                    .unwrap_or_else(|| env::var_os("DESKTOP_PET_TRACE_INPUT").is_some()),
            )
        };
        #[cfg(all(not(feature = "development"), feature = "window-diagnostics"))]
        let (variant, trace_playback, trace_input) = (
            Variant::Png,
            env::var_os("DESKTOP_PET_TRACE").is_some(),
            env::var_os("DESKTOP_PET_TRACE_INPUT").is_some(),
        );
        #[cfg(not(any(feature = "development", feature = "window-diagnostics")))]
        let (variant, trace_playback, trace_input) = (Variant::Png, false, false);
        Ok(Self {
            startup,
            variant,
            trace_playback,
            trace_input,
        })
    }
}

#[cfg(feature = "development")]
fn development_format() -> Variant {
    match env::var("DESKTOP_PET_FORMAT").as_deref() {
        Ok("rgba8") => Variant::Rgba8,
        Ok("bc7") => Variant::Bc7,
        #[cfg(feature = "ffmpeg-decoder")]
        Ok("ffmpeg") => Variant::Ffmpeg,
        Ok("png") | Err(_) => Variant::Png,
        Ok(other) => {
            eprintln!("Unknown asset format {other}; using PNG");
            Variant::Png
        }
    }
}
