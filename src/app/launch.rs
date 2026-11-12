#[cfg(feature = "development")]
use std::env;
use std::{io, path::PathBuf};
#[cfg(feature = "p1b-reference")]
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    window::WindowAttributes,
};

use crate::media::Variant;
#[cfg(feature = "development")]
use crate::storage::AssetFormat;

use super::config::StartupConfig;

pub(super) struct LaunchOptions {
    pub startup: StartupConfig,
    pub variant: Variant,
    pub trace_playback: bool,
    pub trace_input: bool,
}

impl LaunchOptions {
    pub fn load(root: PathBuf) -> io::Result<Self> {
        if std::env::args_os().nth(1).is_some() {
            return Err(io::Error::other("usage: desktop-pet-chatgpt"));
        }
        let startup = StartupConfig::load(root)?;
        #[cfg(feature = "development")]
        let (variant, trace_playback, trace_input) = {
            let dev = startup.config.development.as_ref();
            let variant = match dev.and_then(|dev| dev.format.as_ref()) {
                Some(AssetFormat::Png) => Variant::Png,
                Some(AssetFormat::Rgba8) => Variant::Rgba8,
                Some(AssetFormat::Bc7) => Variant::Bc7,
                #[cfg(feature = "p1b-reference")]
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
        #[cfg(not(feature = "development"))]
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
        #[cfg(feature = "p1b-reference")]
        Ok("ffmpeg") => Variant::Ffmpeg,
        Ok("png") | Err(_) => Variant::Png,
        Ok(other) => {
            eprintln!("Unknown asset format {other}; using PNG");
            Variant::Png
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupGeometry {
    Product,
    #[cfg(feature = "p1b-reference")]
    DipOnly,
    #[cfg(feature = "p1b-reference")]
    Fixed,
}

impl StartupGeometry {
    pub fn probe_edid(self) -> bool {
        self == Self::Product
    }

    pub fn fixed(self) -> bool {
        #[cfg(feature = "p1b-reference")]
        return self == Self::Fixed;
        #[cfg(not(feature = "p1b-reference"))]
        false
    }
}

#[cfg(feature = "p1b-reference")]
pub(super) fn reference_client_geometry(
    attributes: WindowAttributes,
) -> io::Result<(WindowAttributes, StartupGeometry)> {
    let value = match env::var("DESKTOP_PET_REFERENCE_CLIENT") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return Ok((attributes, StartupGeometry::Product)),
        Err(error) => return Err(io::Error::other(error)),
    };
    let fields: Vec<_> = value.split(',').collect();
    if let [mode, x, y] = fields.as_slice()
        && let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>())
    {
        let attributes = attributes.with_position(PhysicalPosition::new(x, y));
        match *mode {
            "auto" => return Ok((attributes, StartupGeometry::Product)),
            "dip" => return Ok((attributes, StartupGeometry::DipOnly)),
            "256" | "320" | "512" => {
                let side: u32 = mode.parse().unwrap();
                return Ok((
                    attributes.with_inner_size(PhysicalSize::new(side, side)),
                    StartupGeometry::Fixed,
                ));
            }
            _ => {}
        }
    }
    Err(io::Error::other(
        "DESKTOP_PET_REFERENCE_CLIENT needs mode,x,y with mode auto, dip, 256, 320 or 512",
    ))
}
