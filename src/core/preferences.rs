use serde::{Deserialize, Serialize};

fn deserialize_chatgpt_connection<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum LegacyMode {
        Off,
        Manual,
        Automatic,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Setting {
        Enabled(bool),
        Legacy(LegacyMode),
    }

    Ok(match Setting::deserialize(deserializer)? {
        Setting::Enabled(enabled) => enabled,
        Setting::Legacy(mode) => !matches!(mode, LegacyMode::Off),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementMode {
    Off,
    Horizontal,
    AllDirections,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub auto_move: bool,
    pub auto_diagonal: bool,
    pub fullscreen_hide: bool,
    pub start_hidden: bool,
    pub sound: bool,
    pub task_bubbles: bool,
    pub bubble_scale_percent: u16,
    pub passthrough: bool,
    #[serde(deserialize_with = "deserialize_chatgpt_connection")]
    pub chatgpt_connection: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            auto_move: true,
            auto_diagonal: true,
            fullscreen_hide: true,
            start_hidden: false,
            sound: true,
            task_bubbles: true,
            bubble_scale_percent: 100,
            passthrough: false,
            chatgpt_connection: true,
        }
    }
}

impl Preferences {
    pub fn movement_mode(&self) -> MovementMode {
        match (self.auto_move, self.auto_diagonal) {
            (false, _) => MovementMode::Off,
            (true, false) => MovementMode::Horizontal,
            (true, true) => MovementMode::AllDirections,
        }
    }

    pub fn set_movement_mode(&mut self, mode: MovementMode) {
        match mode {
            MovementMode::Off => self.auto_move = false,
            MovementMode::Horizontal | MovementMode::AllDirections => {
                self.auto_move = true;
                self.auto_diagonal = mode == MovementMode::AllDirections;
            }
        }
    }
}

pub const INITIAL_SCALE: u16 = 100;
pub const SIZE_RANGE: std::ops::RangeInclusive<u16> = 50..=200;
pub const DEFAULT_DIAGONAL_ANGLE: f64 = 25.0;
pub const DIAGONAL_ANGLE_RANGE: std::ops::RangeInclusive<f64> = 1.0..=60.0;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_mode_defaults_to_all_directions_and_off_keeps_diagonal_preference() {
        let mut settings = Preferences::default();
        assert_eq!(settings.movement_mode(), MovementMode::AllDirections);
        for (mode, diagonal) in [
            (MovementMode::Horizontal, false),
            (MovementMode::AllDirections, true),
        ] {
            settings.set_movement_mode(mode);
            assert_eq!(settings.movement_mode(), mode);
            assert!(settings.auto_move);
            assert_eq!(settings.auto_diagonal, diagonal);

            settings.set_movement_mode(MovementMode::Off);
            assert_eq!(settings.movement_mode(), MovementMode::Off);
            assert!(!settings.auto_move);
            assert_eq!(settings.auto_diagonal, diagonal);
        }
    }
}
