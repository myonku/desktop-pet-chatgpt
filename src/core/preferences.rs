use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub auto_move: bool,
    pub fullscreen_hide: bool,
    pub sound: bool,
    pub task_bubbles: bool,
    pub bubble_scale_percent: u16,
    pub passthrough: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            auto_move: true,
            fullscreen_hide: true,
            sound: true,
            task_bubbles: true,
            bubble_scale_percent: 100,
            passthrough: false,
        }
    }
}

pub const INITIAL_SCALE: u16 = 70;
pub const SIZE_RANGE: std::ops::RangeInclusive<u16> = 50..=200;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSize {
    pub base_dip: f64,
    pub percent: u16,
}

impl SavedSize {
    pub fn valid(self) -> bool {
        (1.0..=4096.0).contains(&self.base_dip) && SIZE_RANGE.contains(&self.percent)
    }

    pub fn dip(self) -> f64 {
        self.base_dip * f64::from(self.percent) / 100.0
    }
}
