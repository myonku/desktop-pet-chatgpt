use serde::{Deserialize, Deserializer, de};

use super::Side;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Behavior {
    Idle,
    Interact,
    Drag,
    Land,
    MoveLeft,
    MoveRight,
    MoveUpLeft,
    MoveUpRight,
    MoveDownLeft,
    MoveDownRight,
    DockLeft,
    DockRight,
    PeekLeft,
    PeekRight,
}

impl Behavior {
    pub const DIAGONAL_MOVES: [Self; 4] = [
        Self::MoveUpLeft,
        Self::MoveUpRight,
        Self::MoveDownLeft,
        Self::MoveDownRight,
    ];

    pub const ALL: [Self; 14] = [
        Self::Idle,
        Self::Interact,
        Self::Drag,
        Self::Land,
        Self::MoveLeft,
        Self::MoveRight,
        Self::MoveUpLeft,
        Self::MoveUpRight,
        Self::MoveDownLeft,
        Self::MoveDownRight,
        Self::DockLeft,
        Self::DockRight,
        Self::PeekLeft,
        Self::PeekRight,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Interact => "interact",
            Self::Drag => "drag",
            Self::Land => "land",
            Self::MoveLeft => "move.left",
            Self::MoveRight => "move.right",
            Self::MoveUpLeft => "move.up-left",
            Self::MoveUpRight => "move.up-right",
            Self::MoveDownLeft => "move.down-left",
            Self::MoveDownRight => "move.down-right",
            Self::DockLeft => "dock.left",
            Self::DockRight => "dock.right",
            Self::PeekLeft => "peek.left",
            Self::PeekRight => "peek.right",
        }
    }

    pub(crate) fn movement_side(self) -> Option<Side> {
        match self {
            Self::MoveLeft | Self::MoveUpLeft | Self::MoveDownLeft => Some(Side::Left),
            Self::MoveRight | Self::MoveUpRight | Self::MoveDownRight => Some(Side::Right),
            _ => None,
        }
    }

    pub(crate) fn is_edge(self) -> bool {
        matches!(
            self,
            Self::DockLeft | Self::DockRight | Self::PeekLeft | Self::PeekRight
        )
    }
}

impl<'de> Deserialize<'de> for Behavior {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|behavior| behavior.key() == key)
            .ok_or_else(|| de::Error::custom(format!("unknown asset behavior {key}")))
    }
}

#[derive(Clone, Copy)]
pub enum ActionChoice<'a> {
    Default,
    Random,
    Variant(&'a str),
    ExactVariant(&'a str),
}
