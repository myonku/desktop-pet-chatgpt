use std::time::{Duration, Instant};

use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    window::Window,
};

use crate::{
    core::{Activity, Click, INTERACT, MOVE_INTERVAL, POLL_INTERVAL, Route, Side},
    platform::windows::{Command, Display, Gesture, MenuState},
};

use super::Scene;

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

    fn available(&self, id: &str) -> bool {
        !self.unavailable.contains(id)
            && self
                .playback
                .as_ref()
                .is_some_and(|player| player.contains(id))
    }

    pub(super) fn play_base(&mut self) {
        let edge = self.pet.edge.map(|side| side.pose(self.pet.peek));
        let id = edge.filter(|id| self.available(id)).or_else(|| {
            self.pet
                .edge
                .map(|side| side.pose(false))
                .filter(|id| self.available(id))
        });
        if let Some(playback) = &mut self.playback {
            playback.play(id, false);
        }
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
            self.playback
                .as_mut()
                .unwrap()
                .switch_pose(target, previous, now);
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
            self.playback
                .as_mut()
                .unwrap()
                .enter_pose(edge, side.pose(true));
            if peek {
                self.pet.activity = Activity::DockPeek(side);
            }
        } else {
            self.play_base();
        }
    }

    fn preview_action(&mut self, id: &str) {
        if self.visible && self.pet.activity != Activity::Drag && self.available(id) {
            for side in [Side::Left, Side::Right] {
                for peek in [false, true] {
                    if id == side.pose(peek) {
                        self.preview_dock(side, peek, Instant::now());
                        return;
                    }
                }
            }
            self.pet.cancel_click();
            self.pet.activity = Activity::Manual;
            self.playback.as_mut().unwrap().play(Some(id), true);
        }
    }

    fn click(&mut self, click: Click) {
        match click {
            Click::Quota => {
                if self.trace.is_some() {
                    eprintln!("Quota: connection required");
                }
                self.input.connection_hint();
            }
            Click::Interact => {
                self.play_click_sound();
                self.preview_action(INTERACT);
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

    pub(super) fn handle_gesture(&mut self, gesture: Gesture) {
        if !self.visible || self.passthrough {
            return;
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
                    self.click(click);
                }
            }
            Gesture::DragStarted => {
                self.pet.cancel_click();
                self.pet.edge = None;
                self.pet.peek = false;
                self.pet.activity = Activity::Drag;
                if self.available("drag") {
                    self.playback.as_mut().unwrap().play(Some("drag"), false);
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
                        return;
                    }
                }
                self.finish(now);
                self.update_display(now);
                self.preview_action("land");
            }
            Gesture::Cancel => {
                self.pet.cancel_click();
                if self.pet.activity == Activity::Drag {
                    self.finish(now);
                    self.update_display(now);
                }
            }
        }
    }

    fn expand(&mut self, now: Instant) {
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

    fn start_auto(&mut self, now: Instant) {
        let (Some(display), Some(position), Some(playback)) =
            (self.display, self.position(), &self.playback)
        else {
            self.pet.finish(now);
            return;
        };
        let width = self.size()[0];
        let max_distance = (144.0 * self.window.scale_factor()).round() as i32;
        let minimum = (16.0 * self.window.scale_factor()).round() as i32;
        let route = [Side::Left, Side::Right]
            .into_iter()
            .filter_map(|side| {
                let id = side.movement();
                let distance = (side.sign() * (display.work.edge(side, width) - position[0]))
                    .min(max_distance);
                if distance < minimum || !self.available(id) {
                    return None;
                }
                playback.movement(side).map(|window_us| Route {
                    side,
                    start: position,
                    distance,
                    window_us,
                })
            })
            .max_by_key(|route| route.distance);
        if let Some(route) = route {
            self.pet.activity = Activity::Auto(route);
            self.playback
                .as_mut()
                .unwrap()
                .play(Some(route.side.movement()), true);
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
            if let Some(at) = self.pet.auto_deadline() {
                if now >= at {
                    self.start_auto(now);
                } else {
                    deadline = deadline.min(at);
                }
            }
        }
        let Some(playback) = &mut self.playback else {
            return Ok(Some(deadline));
        };
        match playback.tick(now) {
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
                    && !self.playback.as_ref().unwrap().sliding_pose()
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
        let playback = self.playback.as_ref().unwrap();
        if playback.is_base() {
            return self.fallback(error);
        }
        let id = playback.active_id().to_owned();
        eprintln!("Action {id} unavailable: {error}");
        self.unavailable.insert(id);
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

    pub(super) fn menu(&mut self, position: Option<PhysicalPosition<i32>>) {
        self.pet.cancel_click();
        let mut ids = vec![
            INTERACT,
            Side::Left.movement(),
            Side::Right.movement(),
            Side::Left.pose(false),
            Side::Right.pose(false),
            Side::Left.pose(true),
            Side::Right.pose(true),
            "drag",
            "land",
        ];
        if let Some(playback) = &self.playback {
            for id in playback.action_ids() {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        let ids: Vec<_> = ids.into_iter().map(str::to_owned).collect();
        let entries: Vec<_> = ids
            .iter()
            .map(|id| {
                let name = match id.as_str() {
                    INTERACT => "互动",
                    "move-left" => "向左移动(&L)",
                    "move-right" => "向右移动(&R)",
                    "edge-left" => "左侧吸边(&1)",
                    "edge-right" => "右侧吸边(&2)",
                    "peek-left" => "左侧探头(&3)",
                    "peek-right" => "右侧探头(&4)",
                    "drag" => "拖拽",
                    "land" => "落地",
                    _ => id,
                };
                let present = self.available(id);
                let label = if !present {
                    format!("{name}（素材不可用）")
                } else if !self.visible {
                    format!("{name}（桌宠已隐藏）")
                } else {
                    name.to_owned()
                };
                (label, present && self.visible)
            })
            .collect();
        if let Err(error) = self.input.menu(
            position,
            MenuState {
                tray: self.input.has_tray(),
                passthrough: self.passthrough,
                visible: !self.pet.manual_hidden,
                attached: self.pet.edge.is_some(),
                auto_move: self.pet.auto_enabled,
                sound: self.runtime.preferences.sound,
                fullscreen_hide: self.runtime.preferences.fullscreen_hide,
                actions: entries,
            },
            ids,
        ) {
            eprintln!("Could not open menu: {error}");
        }
    }

    pub(super) fn menu_selected(&mut self, command: Command, ids: &[String]) -> Option<Command> {
        match command {
            Command::ToggleVisibility => {
                self.pet.manual_hidden = !self.pet.manual_hidden;
                self.update_display(Instant::now());
            }
            Command::Expand => self.expand(Instant::now()),
            Command::Action(index) => {
                if let Some(id) = ids.get(index) {
                    self.preview_action(id);
                }
            }
            command => return Some(command),
        }
        None
    }
}
