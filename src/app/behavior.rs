use std::{
    hash::{BuildHasher, RandomState},
    time::{Duration, Instant},
};

use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    window::Window,
};

use crate::{
    core::{
        ActionChoice, Activity, Behavior, Click, MOVE_INTERVAL, POLL_INTERVAL, Route, Side,
        Vertical,
    },
    platform::windows::{Command, Display, Gesture, MenuState},
};

use super::Scene;

pub(super) enum ActionPreview {
    Pose(Side, bool),
    Clip(usize),
}

const DIAGONAL_CHANCE: u64 = 4;
const AUTO_STEP_DIP: f64 = 144.0;
const WALK_STEP_DIP: f64 = 72.0;

impl Scene {
    pub(super) fn target_size(
        window: &Window,
        display: Option<Display>,
        side_dip: f64,
    ) -> PhysicalSize<u32> {
        let pixels = (side_dip * window.scale_factor()).round().max(1.0) as u32;
        let side = display.map_or(pixels, |display| {
            pixels
                .min((display.work.max[0] - display.work.min[0]).max(1) as u32)
                .min((display.work.max[1] - display.work.min[1]).max(1) as u32)
        });
        PhysicalSize::new(side, side)
    }

    fn position(&self) -> Option<[i32; 2]> {
        self.window
            .outer_position()
            .ok()
            .map(|point| [point.x, point.y])
    }

    fn size(&self) -> [i32; 2] {
        let size = self.window.outer_size();
        [size.width as i32, size.height as i32]
    }

    fn place(&self, point: [i32; 2]) {
        if self.position() != Some(point) {
            self.window
                .set_outer_position(PhysicalPosition::new(point[0], point[1]));
        }
    }

    fn constrain(&self) {
        if let (Some(display), Some(position)) = (self.display, self.position()) {
            let size = self.size();
            let mut point = display.work.clamp(position, size);
            if let Some(side) = self.pet.edge {
                point[0] = display.work.edge(side, size[0]);
            }
            self.place(point);
        }
    }

    pub(super) fn update_display(&mut self, now: Instant) {
        let display = self.input.display();
        let changed = self.display != display;
        let same_monitor = self
            .display
            .zip(display)
            .is_some_and(|(old, new)| old.device == new.device);
        self.display = display;
        self.pet.fullscreen = self.runtime.preferences.fullscreen_hide
            && self.input.has_tray()
            && display.is_some_and(|display| self.input.fullscreen(display));
        self.apply_visibility(now);

        if self.fixed_geometry
            || self.input.gesture_in_progress()
            || self.pet.activity == Activity::Drag
        {
            return;
        }
        if changed && !same_monitor && self.pet.edge.take().is_some() {
            self.pet.peek = false;
            self.pet.reset(now);
            self.play_base();
        } else if changed && self.moving() {
            self.finish(now);
        }
        let size = Self::target_size(&self.window, display, self.runtime.size.dip());
        if self.applied_size != Some(size) || self.window.inner_size() != size {
            if self.moving() {
                self.finish(now);
            }
            let _ = self.window.request_inner_size(size);
            self.applied_size = Some(size);
        }
        self.constrain();
    }

    fn available(&self, behavior: Behavior) -> bool {
        self.playback
            .action_index(behavior)
            .is_some_and(|index| !self.unavailable.contains(&index))
    }

    fn select_action(&self, behavior: Behavior, choice: ActionChoice<'_>) -> Option<usize> {
        self.playback
            .select_action(behavior, choice, |index| !self.unavailable.contains(&index))
    }

    pub(super) fn play_base(&mut self) {
        let edge = self.pet.edge.map(|side| side.pose(self.pet.peek));
        let id = edge.filter(|&id| self.available(id)).or_else(|| {
            self.pet
                .edge
                .map(|side| side.pose(false))
                .filter(|&id| self.available(id))
        });
        self.playback.play(id, false);
    }

    fn finish(&mut self, now: Instant) {
        self.pet.finish(now);
        self.play_base();
    }

    fn moving(&self) -> bool {
        matches!(
            self.pet.activity,
            Activity::Auto(_) | Activity::DockTravel(..)
        )
    }

    fn set_peek(&mut self, side: Side, peek: bool, now: Instant) {
        let previous = side.pose(self.pet.peek);
        self.pet.peek = peek;
        self.pet.finish(now);
        let target = side.pose(peek);
        if self.available(target) {
            self.playback.switch_pose(target, previous, now);
        } else {
            self.play_base();
        }
    }

    fn preview_dock(&mut self, side: Side, peek: bool, now: Instant) {
        let (Some(display), Some(position)) = (self.input.display(), self.position()) else {
            return;
        };
        self.pet.cancel_click();
        if self.pet.edge == Some(side) {
            self.set_peek(side, peek, now);
            return;
        }
        self.display = Some(display);
        let size = self.size();
        let start = display.work.clamp(position, size);
        self.pet.edge = None;
        self.pet.peek = false;
        self.play_base();
        let distance = start[0].abs_diff(display.work.edge(side, size[0]));
        let seconds = (f64::from(distance) / (850.0 * self.window.scale_factor())).clamp(0.25, 3.0);
        self.travel_to_edge(
            side,
            peek,
            start,
            distance,
            Duration::from_secs_f64(seconds),
            now,
        );
    }

    fn travel_to_edge(
        &mut self,
        side: Side,
        peek: bool,
        start: [i32; 2],
        distance: u32,
        duration: Duration,
        now: Instant,
    ) {
        let route = Route {
            side,
            start,
            distance: distance as i32,
            vertical_distance: 0,
            window_us: [
                0,
                if distance == 0 {
                    1
                } else {
                    duration.as_micros() as u64
                },
            ],
        };
        self.pet.activity = Activity::DockTravel(route, now, peek);
    }

    fn arrive_at_edge(&mut self, side: Side, peek: bool, now: Instant) {
        self.pet.edge = Some(side);
        self.pet.peek = peek && !self.available(side.pose(false));
        self.pet.finish(now);
        let edge = side.pose(false);
        if self.available(edge) {
            self.playback.enter_pose(edge, side.pose(true));
            if peek {
                self.pet.activity = Activity::DockPeek(side);
            }
        } else {
            self.play_base();
        }
    }

    pub(super) fn preview_action(&mut self, preview: &ActionPreview) {
        if !self.visible || self.pet.activity == Activity::Drag {
            return;
        }
        match *preview {
            ActionPreview::Pose(side, peek) if self.available(side.pose(peek)) => {
                self.preview_dock(side, peek, Instant::now());
            }
            ActionPreview::Clip(index) if !self.unavailable.contains(&index) => {
                self.play_once(index);
                if let Some(trace) = &mut self.trace {
                    trace.action.clear();
                    eprintln!("Preview clip: {}", self.playback.active_id());
                }
            }
            _ => {}
        }
    }

    fn play_once(&mut self, index: usize) {
        self.pet.cancel_click();
        self.pet.activity = Activity::Manual;
        self.playback.play_index(index, true);
    }

    fn click(&mut self, click: Click) {
        match click {
            Click::Overview => {}
            Click::Interact => {
                self.play_click_sound();
                if let Some(index) = self.select_action(Behavior::Interact, ActionChoice::Random) {
                    self.play_once(index);
                }
            }
            Click::Peek => {
                self.play_click_sound();
                let Some(side) = self.pet.edge else {
                    return;
                };
                self.set_peek(side, !self.pet.peek, Instant::now());
            }
        }
    }

    pub(super) fn handle_gesture(&mut self, gesture: Gesture) -> bool {
        if !self.visible || self.passthrough {
            return false;
        }
        let now = Instant::now();
        match gesture {
            Gesture::Press => {
                if self.moving() {
                    self.finish(now);
                } else if matches!(self.pet.activity, Activity::DockPeek(_)) {
                    self.pet.finish(now);
                }
            }
            Gesture::Click(at, point, tolerance) => {
                if let Some(click) = self.pet.click(at, point, tolerance) {
                    if click == Click::Overview {
                        return true;
                    }
                    self.click(click);
                }
            }
            Gesture::DragStarted => {
                self.pet.cancel_click();
                self.pet.edge = None;
                self.pet.peek = false;
                self.pet.activity = Activity::Drag;
                if self.available(Behavior::Drag) {
                    self.playback.play(Some(Behavior::Drag), false);
                } else {
                    self.play_base();
                }
            }
            Gesture::Drop => {
                self.display = self.input.display();
                if let (Some(display), Some(point)) = (self.display, self.position()) {
                    let size = self.size();
                    let point = display.work.clamp(point, size);
                    let threshold = (16.0 * self.window.scale_factor()).round() as i32;
                    let side = display.work.snap(point[0], size[0], threshold);
                    if self.trace.is_some() {
                        eprintln!("Drop: {side:?} at {point:?}");
                    }
                    if let Some(side) = side {
                        let distance = point[0].abs_diff(display.work.edge(side, size[0]));
                        self.travel_to_edge(
                            side,
                            false,
                            point,
                            distance,
                            Duration::from_millis(120),
                            now,
                        );
                        return false;
                    }
                }
                self.finish(now);
                self.update_display(now);
                if let Some(index) = self.select_action(Behavior::Land, ActionChoice::Default) {
                    self.play_once(index);
                }
            }
            Gesture::Cancel => {
                self.pet.cancel_click();
                if self.pet.activity == Activity::Drag {
                    self.finish(now);
                    self.update_display(now);
                }
            }
        }
        false
    }

    pub(super) fn expand(&mut self, now: Instant) {
        if let Some(side) = self.pet.edge.take() {
            if let Some(mut point) = self.position() {
                let inset = (32.0 * self.window.scale_factor()).round() as i32;
                point[0] -= side.sign() * inset;
                self.place(point);
            }
            self.pet.peek = false;
            self.finish(now);
            self.constrain();
        }
    }

    fn auto_route(&self) -> Option<(Route, usize)> {
        let display = self.display?;
        let position = self.position()?;
        let [width, height] = self.size();
        let scale = self.window.scale_factor();
        let max_distance = (AUTO_STEP_DIP * scale).round() as i32;
        let minimum = (16.0 * scale).round() as i32;
        let roll = RandomState::new().hash_one(position);
        let tie_breaker = if roll & 1 == 0 {
            Side::Left
        } else {
            Side::Right
        };
        let try_diagonal =
            self.runtime.preferences.auto_diagonal && (roll >> 2).is_multiple_of(DIAGONAL_CHANCE);
        let first_vertical = if roll & 2 == 0 {
            Vertical::Up
        } else {
            Vertical::Down
        };
        display
            .work
            .auto_steps(position[0], width, max_distance, minimum, tie_breaker)
            .find_map(|(side, distance)| {
                let diagonal = try_diagonal
                    .then(|| {
                        display
                            .work
                            .diagonal_steps(
                                position[1],
                                height,
                                distance,
                                self.diagonal_angle,
                                first_vertical,
                            )
                            .find_map(|(vertical, dy)| {
                                self.playback
                                    .select_diagonal(side, vertical, |index| {
                                        !self.unavailable.contains(&index)
                                    })
                                    .map(|index| (index, dy))
                            })
                    })
                    .flatten();
                let (index, distance, vertical_distance) = if let Some((index, dy)) = diagonal {
                    (index, distance, dy)
                } else {
                    let movement = side.movement();
                    let index = self.select_action(movement, ActionChoice::Random)?;
                    let walk = self.select_action(movement, ActionChoice::ExactVariant("walk"));
                    let distance = if walk == Some(index) {
                        distance.min((WALK_STEP_DIP * scale).round() as i32)
                    } else {
                        distance
                    };
                    (index, distance, 0)
                };
                let window_us = self.playback.movement(index, side)?;
                Some((
                    Route {
                        side,
                        start: position,
                        distance,
                        vertical_distance,
                        window_us,
                    },
                    index,
                ))
            })
    }

    fn start_ambient(&mut self, now: Instant) {
        let route = self.pet.auto_enabled.then(|| self.auto_route()).flatten();
        let idle_default = self.playback.action_index(Behavior::Idle);
        let idle = self
            .playback
            .select_action(Behavior::Idle, ActionChoice::Random, |index| {
                Some(index) != idle_default && !self.unavailable.contains(&index)
            });
        if let Some((route, index)) = route
            && (!self.pet.prefer_idle_variant || idle.is_none())
        {
            self.pet.activity = Activity::Auto(route);
            self.playback.play_index(index, true);
        } else if let Some(index) = idle {
            self.pet.activity = Activity::Ambient;
            self.playback.play_index(index, true);
        } else {
            self.pet.finish(now);
        }
    }

    pub(super) fn playback_deadline(&mut self) -> Result<Option<Instant>, String> {
        let now = Instant::now();
        if now >= self.screen_poll {
            self.screen_poll = now + POLL_INTERVAL;
            self.update_display(now);
        }
        let mut deadline = self.screen_poll;
        let minimized = self.window.is_minimized() == Some(true);
        if self.minimized != minimized {
            self.minimized = minimized;
            self.input.cancel_gesture();
            self.pet.reset(now);
            self.play_base();
            if minimized {
                self.sound.stop();
            } else if self.visible {
                self.window.request_redraw();
            }
        }
        if !self.visible || minimized {
            return Ok(Some(deadline));
        }
        if !self.input.gesture_in_progress() {
            if let Activity::DockTravel(route, started, peek) = self.pet.activity {
                if self.input.edge_frame_presented() {
                    // The previous pose can still be on screen while idle loads.
                    self.pet.activity = Activity::DockTravel(route, now, peek);
                    deadline = deadline.min(now + MOVE_INTERVAL);
                } else {
                    let elapsed = now.saturating_duration_since(started);
                    self.place(route.position(elapsed));
                    if elapsed >= Duration::from_micros(route.window_us[1]) {
                        self.arrive_at_edge(route.side, peek, now);
                    } else {
                        deadline = deadline.min(now + MOVE_INTERVAL);
                    }
                }
            }
            if let Some(click) = self.pet.take_click(now) {
                self.click(click);
            }
            if let Some(at) = self.pet.click_deadline() {
                deadline = deadline.min(at);
            }
            let has_idle_variant = !self.pet.auto_enabled
                && self
                    .playback
                    .has_idle_variant(|index| !self.unavailable.contains(&index));
            if let Some(at) = self.pet.ambient_deadline(has_idle_variant) {
                if now >= at {
                    self.start_ambient(now);
                } else {
                    deadline = deadline.min(at);
                }
            }
        }
        match self.playback.tick(now) {
            Ok(tick) => {
                if tick.changed
                    && let Err(error) = self.refresh_frame()
                {
                    self.playback_failed(now, error)?;
                    return Ok(Some(now));
                }
                if let Activity::DockPeek(side) = self.pet.activity
                    && tick.elapsed.is_some()
                    && !self.input.gesture_in_progress()
                    && !self.playback.sliding_pose()
                {
                    self.set_peek(side, true, now);
                    return Ok(Some(now));
                }
                if let (Activity::Auto(route), Some(elapsed)) = (self.pet.activity, tick.elapsed)
                    && !self.input.gesture_in_progress()
                {
                    self.place(route.position(elapsed));
                    let [start, end] = route.window_us.map(Duration::from_micros);
                    if elapsed < end {
                        let update = if elapsed < start {
                            start - elapsed
                        } else {
                            MOVE_INTERVAL
                        };
                        deadline = deadline.min(now + update);
                    }
                }
                if tick.finished {
                    if matches!(
                        self.pet.activity,
                        Activity::Base | Activity::Drag | Activity::DockTravel(..)
                    ) {
                        // A non-looping edge asset holds its last pose until the next gesture.
                        return Ok(Some(deadline));
                    }
                    self.finish(now);
                    return Ok(Some(now));
                }
                Ok(Some(tick.deadline.map_or(deadline, |at| deadline.min(at))))
            }
            Err(error) => {
                self.playback_failed(now, error)?;
                Ok(Some(now))
            }
        }
    }

    fn playback_failed(
        &mut self,
        now: Instant,
        error: impl std::fmt::Display,
    ) -> Result<(), String> {
        let playback = &self.playback;
        if playback.is_base() {
            return Err(format!("素材包的基础待机动作无法继续播放：{error}"));
        }
        let id = playback.active_id();
        eprintln!("Action {id} unavailable: {error}");
        self.unavailable.insert(playback.active_index());
        if let Activity::DockPeek(side) = self.pet.activity {
            self.set_peek(side, true, now);
            return Ok(());
        }
        if matches!(self.pet.activity, Activity::Drag | Activity::DockTravel(..)) {
            self.play_base();
        } else {
            self.finish(now);
        }
        Ok(())
    }

    pub(super) fn action_previews(&self) -> (Vec<ActionPreview>, Vec<(String, bool)>) {
        let poses = [
            (Side::Left, false, "左侧吸边"),
            (Side::Right, false, "右侧吸边"),
            (Side::Left, true, "左侧探头"),
            (Side::Right, true, "右侧探头"),
        ];
        let pose_index = |side: Side, peek| self.playback.action_index(side.pose(peek));
        let poses_in_pack = poses.into_iter().filter_map(|(side, peek, label)| {
            let index = pose_index(side, peek)?;
            Some((
                ActionPreview::Pose(side, peek),
                (
                    label.to_owned(),
                    self.visible && !self.unavailable.contains(&index),
                ),
            ))
        });
        let clips = self.playback.clip_previews().filter_map(|(index, label)| {
            let is_pose = poses
                .iter()
                .any(|(side, peek, _)| pose_index(*side, *peek) == Some(index));
            (!is_pose).then(|| {
                (
                    ActionPreview::Clip(index),
                    (
                        label.to_owned(),
                        self.visible && !self.unavailable.contains(&index),
                    ),
                )
            })
        });
        poses_in_pack.chain(clips).unzip()
    }

    pub(super) fn tray_menu(&self, position: PhysicalPosition<i32>) {
        let state = MenuState {
            passthrough: self.passthrough,
            visible: !self.pet.manual_hidden,
            auto_move: self.runtime.preferences.auto_move,
            sound: self.runtime.preferences.sound,
            fullscreen_hide: self.runtime.preferences.fullscreen_hide,
        };
        if let Err(error) = self.input.menu(position, state) {
            eprintln!("Could not open tray menu: {error}");
        }
    }

    pub(super) fn menu_selected(&mut self, command: Command) -> Option<Command> {
        match command {
            Command::ToggleVisibility => {
                self.pet.manual_hidden = !self.pet.manual_hidden;
                self.update_display(Instant::now());
            }
            command => return Some(command),
        }
        None
    }
}
