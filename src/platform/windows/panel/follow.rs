use std::time::{Duration, Instant};

use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::GetWindowRect,
};

use super::{QuickPlacement, place_quick, quick_placement};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const SPRING_RATE: f64 = 26.0;

#[derive(Default)]
pub(crate) struct QuickFollow {
    state: Option<FollowState>,
    pub(crate) reanchor: bool,
}

struct FollowState {
    target: QuickPlacement,
    axes: [Axis; 2],
    applied: RECT,
    last_frame: Instant,
    next_frame: Option<Instant>,
}

struct Axis {
    position: f64,
    velocity: f64,
}

impl Axis {
    fn at(position: i32) -> Self {
        Self {
            position: f64::from(position),
            velocity: 0.0,
        }
    }

    fn advance(&mut self, target: i32, min: i32, max: i32, seconds: f64) -> bool {
        let target = f64::from(target);
        let displacement = self.position - target;
        // Exact step of a critically damped spring, including carried velocity on retarget.
        let slope = self.velocity + SPRING_RATE * displacement;
        let decay = (-SPRING_RATE * seconds).exp();
        let position = target + (displacement + slope * seconds) * decay;
        self.velocity = (self.velocity - SPRING_RATE * slope * seconds) * decay;
        self.position = position.clamp(f64::from(min), f64::from(max));
        if self.position != position {
            self.velocity = 0.0;
        }
        if (self.position - target).abs() < 0.5 && self.velocity.abs() < 8.0 {
            self.position = target;
            self.velocity = 0.0;
            return false;
        }
        true
    }
}

impl QuickFollow {
    pub(crate) fn is_pending(&self) -> bool {
        self.reanchor
            || self
                .state
                .as_ref()
                .is_some_and(|state| state.next_frame.is_some())
    }

    pub(crate) fn tick(
        &mut self,
        hwnd: HWND,
        pet: HWND,
        width: i32,
        height: i32,
        now: Instant,
    ) -> windows::core::Result<Option<Instant>> {
        if std::mem::take(&mut self.reanchor) {
            let target = quick_placement(pet, width, height)?;
            if let Some(state) = &mut self.state {
                if state.target != target {
                    if state.target.monitor != target.monitor || state.target.dpi != target.dpi {
                        // Change screens and DPI together so the panel never travels across the gap.
                        state.axes = [Axis::at(target.bounds.left), Axis::at(target.bounds.top)];
                        state.next_frame = Some(now);
                        state.last_frame = now;
                    } else if state.next_frame.is_none() {
                        state.next_frame = Some(now);
                        state.last_frame = now;
                    }
                    state.target = target;
                }
            } else {
                let mut applied = RECT::default();
                unsafe { GetWindowRect(hwnd, &mut applied)? };
                self.state = Some(FollowState {
                    target,
                    axes: [Axis::at(applied.left), Axis::at(applied.top)],
                    applied,
                    last_frame: now,
                    next_frame: Some(now),
                });
            }
        }
        let Some(state) = &mut self.state else {
            return Ok(None);
        };
        if state.next_frame.is_some_and(|at| at <= now) {
            state.step(hwnd, now)?;
        }
        Ok(state.next_frame)
    }
}

impl FollowState {
    fn step(&mut self, hwnd: HWND, now: Instant) -> windows::core::Result<()> {
        let QuickPlacement { bounds, work, .. } = self.target;
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        let seconds = now.saturating_duration_since(self.last_frame).as_secs_f64();
        let moving_x = self.axes[0].advance(bounds.left, work.left, work.right - width, seconds);
        let moving_y = self.axes[1].advance(bounds.top, work.top, work.bottom - height, seconds);
        let left = self.axes[0].position.round() as i32;
        let top = self.axes[1].position.round() as i32;
        let next = RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        if next != self.applied {
            place_quick(hwnd, next)?;
            self.applied = next;
        }
        self.last_frame = now;
        self.next_frame = (moving_x || moving_y).then_some(now + FRAME_INTERVAL);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Axis;

    #[test]
    fn follows_with_lag_and_settles_without_more_anchor_updates() {
        let mut axis = Axis::at(0);
        assert!(axis.advance(100, 0, 200, 0.016));
        assert!(axis.position > 0.0 && axis.position < 20.0);
        for _ in 0..20 {
            axis.advance(100, 0, 200, 0.016);
        }
        assert!(!axis.advance(100, 0, 200, 0.016));
        assert_eq!(axis.position, 100.0);
    }

    #[test]
    fn carries_velocity_on_reversal_and_clamps_to_the_work_area() {
        let mut axis = Axis::at(100);
        assert!(axis.advance(200, 0, 200, 0.032));
        let before = axis.position;
        assert!(axis.advance(0, 0, 200, 0.004));
        assert!(axis.position > before);
        assert!(!axis.advance(90, 0, 90, 0.0));
        assert_eq!(axis.position, 90.0);
        assert_eq!(axis.velocity, 0.0);
    }
}
