use std::{env, io, path::PathBuf};
#[cfg(feature = "p1b-reference")]
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    window::WindowAttributes,
};

use crate::media::Variant;

use super::config::StartupConfig;

pub(super) struct LaunchOptions {
    pub startup: StartupConfig,
    pub variant: Variant,
    pub trace_playback: bool,
    pub trace_input: bool,
}

impl LaunchOptions {
    pub fn load() -> io::Result<Self> {
        let mut args = env::args_os().skip(1);
        let dev_path = match (args.next(), args.next(), args.next()) {
            (None, None, None) => None,
            (Some(flag), Some(path), None) if flag == "--dev-config" => Some(PathBuf::from(path)),
            _ => {
                return Err(io::Error::other(
                    "usage: desktop-pet-chatgpt [--dev-config <path>]",
                ));
            }
        };
        let startup = StartupConfig::load(dev_path.as_deref())?;
        let variant = match env::var("DESKTOP_PET_FORMAT").as_deref() {
            Ok("rgba8") => Variant::Rgba8,
            Ok("bc7") => Variant::Bc7,
            #[cfg(feature = "p1b-reference")]
            Ok("ffmpeg") => Variant::Ffmpeg,
            Ok("png") | Err(_) => Variant::Png,
            Ok(other) => {
                eprintln!("Unknown asset format {other}; using PNG");
                Variant::Png
            }
        };
        Ok(Self {
            startup,
            variant,
            trace_playback: env::var_os("DESKTOP_PET_TRACE").is_some(),
            trace_input: env::var_os("DESKTOP_PET_TRACE_INPUT").is_some(),
        })
    }
}

#[cfg(feature = "p1b-reference")]
pub(super) fn reference_client_geometry(
    attributes: WindowAttributes,
) -> io::Result<WindowAttributes> {
    let value = match env::var("DESKTOP_PET_REFERENCE_CLIENT") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return Ok(attributes),
        Err(error) => return Err(io::Error::other(error)),
    };
    let fields: Vec<_> = value.split(',').collect();
    if let [side, x, y] = fields.as_slice()
        && let (Ok(side @ (256 | 320 | 512)), Ok(x), Ok(y)) =
            (side.parse(), x.parse::<i32>(), y.parse::<i32>())
    {
        return Ok(attributes
            .with_inner_size(PhysicalSize::new(side, side))
            .with_position(PhysicalPosition::new(x, y)));
    }
    Err(io::Error::other(
        "DESKTOP_PET_REFERENCE_CLIENT needs side,x,y with side 256, 320 or 512",
    ))
}
