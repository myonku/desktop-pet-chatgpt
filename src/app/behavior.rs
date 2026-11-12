use std::time::{Duration, Instant};

use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event_loop::ActiveEventLoop,
    window::Window,
};

use crate::{
    core::{Activity, Click, INTERACT, MOVE_INTERVAL, POLL_INTERVAL, Route, Side},
    platform::windows::{Command, Display, Gesture},
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
        self.pet.fullscreen = display.is_some_and(|display| self.input.fullscreen(display));
        self.apply_visibility(now);

        if self.reference_geometry
            || self.input.gesture_in_progress()
            || self.pet.activity == Activity::Drag
        {
            return;
        }
        if changed && !same_monitor && self.pet.edge.take().is_some() {
            self.pet.peek = false;
            self.pet.reset(now);
            self.play_base();
        } else if changed && matches!(self.pet.activity, Activity::Auto(_)) {
            self.finish(now);
        }
        let size = Self::target_size(&self.window, display, self.side_dip);
        if self.applied_size != Some(size) || self.window.inner_size() != size {
            if matches!(self.pet.activity, Activity::Auto(_)) {
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

    fn preview_action(&mut self, id: &str) {
        if self.visible && self.pet.activity != Activity::Drag && self.available(id) {
            self.pet.cancel_click();
            self.pet.activity = Activity::Manual;
            self.playback.as_mut().unwrap().play(Some(id), true);
        }
    }

    fn click(&mut self, click: Click) {
        match click {
            Click::Quota => {
                if self.trace.is_some() {
                    eprintln!("P2 quota: connection required");
                }
                self.input.connection_hint();
            }
            Click::Interact => self.preview_action(INTERACT),
            Click::Peek => {
                self.pet.peek = !self.pet.peek;
                self.finish(Instant::now());
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
                if matches!(self.pet.activity, Activity::Auto(_)) {
                    self.finish(now);
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
                    self.pet.edge = display.work.snap(point[0], size[0], threshold);
                    if self.trace.is_some() {
                        eprintln!("P2 drop: {:?} at {point:?}", self.pet.edge);
                    }
                }
                self.finish(now);
                self.update_display(now);
                if self.pet.edge.is_none() {
                    self.preview_action("land");
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
            if !minimized && self.visible {
                self.window.request_redraw();
            }
        }
        if !self.visible || minimized {
            return Ok(Some(deadline));
        }
        if !self.input.gesture_in_progress() {
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
                    if matches!(self.pet.activity, Activity::Base | Activity::Drag) {
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
        if self.pet.activity == Activity::Drag {
            self.play_base();
        } else {
            self.finish(now);
        }
        Ok(())
    }

    pub(super) fn menu(
        &mut self,
        event_loop: &ActiveEventLoop,
        position: Option<PhysicalPosition<i32>>,
    ) -> bool {
        let now = Instant::now();
        self.pet.cancel_click();
        if matches!(self.pet.activity, Activity::Auto(_)) {
            self.finish(now);
        }
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
                    "edge-left" => "左侧吸边",
                    "edge-right" => "右侧吸边",
                    "peek-left" => "左侧探头",
                    "peek-right" => "右侧探头",
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
        match self.input.menu(
            position,
            self.passthrough,
            !self.pet.manual_hidden,
            self.pet.auto_enabled,
            self.pet.edge.is_some(),
            &entries,
        ) {
            Ok(Some(Command::TogglePassthrough)) => self.set_passthrough(!self.passthrough),
            Ok(Some(Command::ToggleVisibility)) => {
                self.pet.manual_hidden = !self.pet.manual_hidden;
                self.update_display(Instant::now());
            }
            Ok(Some(Command::ToggleAuto)) => {
                self.pet.auto_enabled = !self.pet.auto_enabled;
                if self.pet.activity == Activity::Base {
                    self.pet.finish(Instant::now());
                }
            }
            Ok(Some(Command::Expand)) => self.expand(Instant::now()),
            Ok(Some(Command::Action(index))) => self.preview_action(&ids[index]),
            Ok(Some(Command::OpenPreview)) => return true,
            Ok(Some(Command::Exit)) => event_loop.exit(),
            Ok(None) => {}
            Err(error) => eprintln!("Could not open menu: {error}"),
        }
        false
    }
}
