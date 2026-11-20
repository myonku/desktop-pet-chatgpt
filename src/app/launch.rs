#[cfg(any(feature = "development", feature = "window-diagnostics"))]
use std::env;
use std::{io, path::PathBuf};

use super::config::StartupConfig;
use crate::core::preferences::DEFAULT_DIAGONAL_ANGLE;

mod geometry;
pub(super) use geometry::window_geometry;

pub(super) struct LaunchOptions {
    pub startup: StartupConfig,
    pub trace_playback: bool,
    pub trace_input: bool,
    pub diagonal_angle: f64,
}

impl LaunchOptions {
    pub fn load(root: PathBuf) -> io::Result<Self> {
        let startup = StartupConfig::load(root)?;
        #[cfg(feature = "development")]
        let (trace_playback, trace_input) = {
            let dev = startup.config.development.as_ref();
            (
                dev.and_then(|dev| dev.trace_playback)
                    .unwrap_or_else(|| env::var_os("DESKTOP_PET_TRACE").is_some()),
                dev.and_then(|dev| dev.trace_input)
                    .unwrap_or_else(|| env::var_os("DESKTOP_PET_TRACE_INPUT").is_some()),
            )
        };
        #[cfg(all(not(feature = "development"), feature = "window-diagnostics"))]
        let (trace_playback, trace_input) = (
            env::var_os("DESKTOP_PET_TRACE").is_some(),
            env::var_os("DESKTOP_PET_TRACE_INPUT").is_some(),
        );
        #[cfg(not(any(feature = "development", feature = "window-diagnostics")))]
        let (trace_playback, trace_input) = (false, false);
        #[cfg(feature = "development")]
        let diagonal_angle = startup
            .config
            .development
            .as_ref()
            .and_then(|dev| dev.diagonal_angle_degrees)
            .unwrap_or(DEFAULT_DIAGONAL_ANGLE);
        #[cfg(not(feature = "development"))]
        let diagonal_angle = DEFAULT_DIAGONAL_ANGLE;
        Ok(Self {
            startup,
            trace_playback,
            trace_input,
            diagonal_angle,
        })
    }
}
