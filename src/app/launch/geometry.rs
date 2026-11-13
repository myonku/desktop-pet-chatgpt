use std::io;

use winit::window::WindowAttributes;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupGeometry {
    Product,
    #[cfg(feature = "window-diagnostics")]
    DipOnly,
    #[cfg(feature = "window-diagnostics")]
    Fixed,
}

impl StartupGeometry {
    pub fn probe_edid(self) -> bool {
        self == Self::Product
    }

    pub fn fixed(self) -> bool {
        #[cfg(feature = "window-diagnostics")]
        return self == Self::Fixed;
        #[cfg(not(feature = "window-diagnostics"))]
        false
    }
}

pub(crate) fn window_geometry(
    attributes: WindowAttributes,
) -> io::Result<(WindowAttributes, StartupGeometry)> {
    #[cfg(feature = "window-diagnostics")]
    if let Some(value) = std::env::var_os("DESKTOP_PET_WINDOW_GEOMETRY") {
        return parse_override(attributes, &value.to_string_lossy());
    }
    Ok((attributes, StartupGeometry::Product))
}

#[cfg(feature = "window-diagnostics")]
fn parse_override(
    attributes: WindowAttributes,
    value: &str,
) -> io::Result<(WindowAttributes, StartupGeometry)> {
    use winit::dpi::{PhysicalPosition, PhysicalSize};
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
        "DESKTOP_PET_WINDOW_GEOMETRY needs mode,x,y with mode auto, dip, 256, 320 or 512",
    ))
}
