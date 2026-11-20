mod behavior;
pub mod preferences;
pub mod settings;

pub use behavior::{ActionChoice, Behavior};
use std::time::{Duration, Instant};

pub const CLICK_WINDOW: Duration = Duration::from_millis(250);
pub const PAGE_TIMEOUT: Duration = Duration::from_millis(750);
const AUTO_WAIT: Duration = Duration::from_secs(15);
const AUTO_COOLDOWN: Duration = Duration::from_secs(10);
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const MOVE_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn movement(self) -> Behavior {
        match self {
            Self::Left => Behavior::MoveLeft,
            Self::Right => Behavior::MoveRight,
        }
    }

    pub fn diagonal(self, vertical: Vertical) -> Behavior {
        match (self, vertical) {
            (Self::Left, Vertical::Up) => Behavior::MoveUpLeft,
            (Self::Right, Vertical::Up) => Behavior::MoveUpRight,
            (Self::Left, Vertical::Down) => Behavior::MoveDownLeft,
            (Self::Right, Vertical::Down) => Behavior::MoveDownRight,
        }
    }

    pub fn pose(self, peek: bool) -> Behavior {
        match (self, peek) {
            (Self::Left, false) => Behavior::DockLeft,
            (Self::Left, true) => Behavior::PeekLeft,
            (Self::Right, false) => Behavior::DockRight,
            (Self::Right, true) => Behavior::PeekRight,
        }
    }

    pub fn sign(self) -> i32 {
        match self {
            Self::Left => -1,
            Self::Right => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vertical {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkArea {
    pub min: [i32; 2],
    pub max: [i32; 2],
}

impl WorkArea {
    pub fn clamp(self, position: [i32; 2], size: [i32; 2]) -> [i32; 2] {
        std::array::from_fn(|axis| {
            position[axis].clamp(
                self.min[axis],
                (self.max[axis] - size[axis]).max(self.min[axis]),
            )
        })
    }

    pub fn edge(self, side: Side, width: i32) -> i32 {
        match side {
            Side::Left => self.min[0],
            Side::Right => (self.max[0] - width).max(self.min[0]),
        }
    }

    pub fn auto_steps(
        self,
        x: i32,
        width: i32,
        max_step: i32,
        min_step: i32,
        tie_breaker: Side,
    ) -> impl Iterator<Item = (Side, i32)> {
        let left = x - self.edge(Side::Left, width);
        let right = self.edge(Side::Right, width) - x;
        // Prefer a full step that still leaves room before the destination edge.
        let clearance = max_step + min_step;
        let first = match (left >= clearance, right >= clearance) {
            (true, false) => Side::Left,
            (false, true) => Side::Right,
            _ => tie_breaker,
        };
        let left = (Side::Left, left);
        let right = (Side::Right, right);
        match first {
            Side::Left => [left, right],
            Side::Right => [right, left],
        }
        .into_iter()
        .filter_map(move |(side, room)| (room >= min_step).then_some((side, room.min(max_step))))
    }

    pub fn diagonal_steps(
        self,
        y: i32,
        height: i32,
        horizontal: i32,
        angle: f64,
        first: Vertical,
    ) -> impl Iterator<Item = (Vertical, i32)> {
        let rise = (f64::from(horizontal) * angle.to_radians().tan()).round() as i32;
        let top = self.min[1];
        let bottom = (self.max[1] - height).max(top);
        let up = (Vertical::Up, -rise);
        let down = (Vertical::Down, rise);
        match first {
            Vertical::Up => [up, down],
            Vertical::Down => [down, up],
        }
        .into_iter()
        .filter(move |&(_, dy)| rise > 0 && (top..=bottom).contains(&y.saturating_add(dy)))
    }

    pub fn snap(self, x: i32, width: i32, threshold: i32) -> Option<Side> {
        let left = x.abs_diff(self.edge(Side::Left, width));
        let right = x.abs_diff(self.edge(Side::Right, width));
        (left.min(right) <= threshold as u32).then_some(if left <= right {
            Side::Left
        } else {
            Side::Right
        })
    }
}

pub fn initial_side_dip(
    screen_height_px: i32,
    height_mm: Option<u16>,
    dpi_scale: f64,
    scale_percent: u16,
) -> f64 {
    let base = match height_mm {
        Some(mm) if screen_height_px > 0 => {
            let height = f64::from(mm);
            let target_mm = (height * 0.16).clamp(36.0, 90.0);
            target_mm * f64::from(screen_height_px) / height / dpi_scale
        }
        _ => 256.0,
    };
    base * f64::from(scale_percent) / 100.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Base,
    Ambient,
    Manual,
    Auto(Route),
    DockTravel(Route, Instant, bool),
    DockPeek(Side),
    Drag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub side: Side,
    pub start: [i32; 2],
    pub distance: i32,
    pub vertical_distance: i32,
    pub window_us: [u64; 2],
}

impl Route {
    pub fn position(self, elapsed: Duration) -> [i32; 2] {
        let [first, last] = self.window_us;
        let travelled =
            ((elapsed.as_micros() as f64 - first as f64) / (last - first) as f64).clamp(0.0, 1.0);
        let distances = [self.side.sign() * self.distance, self.vertical_distance];
        std::array::from_fn(|axis| {
            self.start[axis] + (f64::from(distances[axis]) * travelled).round() as i32
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Interact,
    Peek,
    Overview,
}

pub struct Pet {
    pub activity: Activity,
    pub edge: Option<Side>,
    pub peek: bool,
    pub auto_enabled: bool,
    pub manual_hidden: bool,
    pub fullscreen: bool,
    pub prefer_idle_variant: bool,
    auto_at: Instant,
    click_at: Option<(Instant, [i32; 2])>,
}

impl Pet {
    pub fn new(now: Instant) -> Self {
        Self {
            activity: Activity::Base,
            edge: None,
            peek: false,
            auto_enabled: true,
            manual_hidden: false,
            fullscreen: false,
            prefer_idle_variant: false,
            auto_at: now + AUTO_WAIT,
            click_at: None,
        }
    }

    pub fn visible(&self) -> bool {
        !self.manual_hidden && !self.fullscreen
    }

    pub fn reset(&mut self, now: Instant) {
        self.cancel_click();
        self.activity = Activity::Base;
        self.prefer_idle_variant = false;
        self.auto_at = now + AUTO_WAIT;
    }

    pub fn finish(&mut self, now: Instant) {
        let delay = match self.activity {
            Activity::Auto(_) => {
                self.prefer_idle_variant = true;
                AUTO_COOLDOWN
            }
            Activity::Ambient => {
                self.prefer_idle_variant = false;
                AUTO_COOLDOWN
            }
            _ => AUTO_WAIT,
        };
        self.activity = Activity::Base;
        self.auto_at = now + delay;
    }

    pub fn cancel_click(&mut self) {
        self.click_at = None;
    }

    pub fn click(&mut self, now: Instant, point: [i32; 2], tolerance: [u32; 2]) -> Option<Click> {
        let pending = self.click_at.take();
        if let Some((until, previous)) = pending
            && now <= until
            && previous[0].abs_diff(point[0]) <= tolerance[0]
            && previous[1].abs_diff(point[1]) <= tolerance[1]
        {
            return Some(Click::Overview);
        }
        self.click_at = Some((now + CLICK_WINDOW, point));
        pending.map(|_| self.single())
    }

    pub fn take_click(&mut self, now: Instant) -> Option<Click> {
        if self.click_at.is_some_and(|(until, _)| now >= until) {
            self.cancel_click();
            return Some(self.single());
        }
        None
    }

    fn single(&self) -> Click {
        if self.edge.is_some() {
            Click::Peek
        } else {
            Click::Interact
        }
    }

    pub fn click_deadline(&self) -> Option<Instant> {
        self.click_at.map(|(until, _)| until)
    }

    pub fn ambient_deadline(&self, has_idle_variants: bool) -> Option<Instant> {
        (self.visible()
            && self.edge.is_none()
            && (self.auto_enabled || has_idle_variants)
            && self.activity == Activity::Base
            && self.click_at.is_none())
        .then_some(self.auto_at)
    }
}

#[cfg(test)]
mod tests;
