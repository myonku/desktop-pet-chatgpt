use super::*;

#[test]
fn single_double_and_cancel_use_monotonic_deadlines() {
    let start = Instant::now();
    let mut pet = Pet::new(start);
    let point = [100, 200];
    assert_eq!(pet.click(start, point, [3, 3]), None);
    assert_eq!(
        pet.take_click(start + CLICK_WINDOW - Duration::from_millis(1)),
        None
    );
    assert_eq!(pet.take_click(start + CLICK_WINDOW), Some(Click::Interact));
    pet.edge = Some(Side::Left);
    let later = start + Duration::from_secs(1);
    assert_eq!(pet.click(later, point, [3, 3]), None);
    assert_eq!(
        pet.click(later + Duration::from_millis(100), [101, 201], [3, 3]),
        Some(Click::Overview)
    );
    assert_eq!(pet.take_click(later + CLICK_WINDOW), None);
    assert_eq!(pet.edge, Some(Side::Left));
    assert_eq!(pet.click(later, point, [3, 3]), None);
    pet.cancel_click();
    assert_eq!(pet.take_click(later + CLICK_WINDOW), None);
    pet.click(later, point, [3, 3]);
    assert_eq!(pet.take_click(later + CLICK_WINDOW), Some(Click::Peek));
}

#[test]
fn route_is_windowed_and_visibility_is_independent() {
    let now = Instant::now();
    let mut pet = Pet::new(now);
    let route = Route {
        side: Side::Left,
        start: [-500, 100],
        distance: 100,
        vertical_distance: 0,
        window_us: [100_000, 500_000],
    };
    for (ms, x) in [
        (0, -500),
        (100, -500),
        (300, -550),
        (500, -600),
        (700, -600),
    ] {
        assert_eq!(route.position(Duration::from_millis(ms)), [x, 100]);
    }
    assert_eq!(pet.ambient_deadline(false), Some(now + AUTO_WAIT));
    assert!(!pet.prefer_idle_variant);
    pet.activity = Activity::Auto(route);
    pet.finish(now);
    assert_eq!(pet.ambient_deadline(false), Some(now + AUTO_COOLDOWN));
    assert!(pet.prefer_idle_variant);
    pet.activity = Activity::Ambient;
    pet.finish(now);
    assert_eq!(pet.ambient_deadline(true), Some(now + AUTO_COOLDOWN));
    assert!(!pet.prefer_idle_variant);
    pet.manual_hidden = true;
    pet.fullscreen = true;
    pet.reset(now);
    assert!(!pet.visible());
    assert_eq!(pet.ambient_deadline(true), None);
    pet.fullscreen = false;
    assert!(!pet.visible());
    pet.manual_hidden = false;
    assert_eq!(pet.ambient_deadline(false), Some(now + AUTO_WAIT));
    pet.auto_enabled = false;
    assert_eq!(pet.ambient_deadline(false), None);
    assert_eq!(pet.ambient_deadline(true), Some(now + AUTO_WAIT));
    pet.auto_enabled = true;
    pet.edge = Some(Side::Right);
    assert_eq!(pet.ambient_deadline(true), None);
}

#[test]
fn work_area_handles_negative_origins_and_clamps_both_axes() {
    let area = WorkArea {
        min: [-1920, -30],
        max: [0, 1010],
    };
    assert_eq!(area.clamp([-2010, -300], [180, 200]), [-1920, -30]);
    assert_eq!(area.clamp([40, 1100], [180, 200]), [-180, 810]);
    assert_eq!(area.snap(-1901, 180, 20), Some(Side::Left));
    assert_eq!(area.snap(-199, 180, 20), Some(Side::Right));
    assert_eq!(area.snap(-1000, 180, 20), None);
}

#[test]
fn auto_steps_keep_moving_inward_after_leaving_either_edge() {
    let area = WorkArea {
        min: [-1920, 0],
        max: [0, 1080],
    };
    let width = 256;
    let step = 180;
    let margin = 20;
    for (edge, inward) in [(Side::Left, Side::Right), (Side::Right, Side::Left)] {
        let x = area.edge(edge, width);
        assert_eq!(
            area.auto_steps(x, width, step, margin, edge)
                .collect::<Vec<_>>(),
            [(inward, step)]
        );
        let x = x + inward.sign() * step;
        assert_eq!(
            area.auto_steps(x, width, step, margin, edge)
                .collect::<Vec<_>>(),
            [(inward, step), (edge, step)]
        );
    }
}

#[test]
fn auto_steps_allow_either_direction_when_clear_and_clamp_short_steps() {
    let area = WorkArea {
        min: [0, 0],
        max: [1000, 800],
    };
    for side in [Side::Left, Side::Right] {
        assert_eq!(
            area.auto_steps(400, 200, 144, 16, side).next(),
            Some((side, 144))
        );
    }
    assert_eq!(
        area.auto_steps(40, 900, 144, 16, Side::Right)
            .collect::<Vec<_>>(),
        [(Side::Right, 60), (Side::Left, 40)]
    );
    assert_eq!(area.auto_steps(0, 1000, 144, 16, Side::Left).next(), None);
}

#[test]
fn diagonal_steps_keep_the_horizontal_distance_and_skip_blocked_vertical_directions() {
    let area = WorkArea {
        min: [-1000, -800],
        max: [0, 0],
    };
    let height = 256;
    for (y, first, expected) in [
        (-800, Vertical::Up, (Vertical::Down, 104)),
        (-256, Vertical::Down, (Vertical::Up, -104)),
    ] {
        assert_eq!(
            area.diagonal_steps(y, height, 180, 30.0, first)
                .collect::<Vec<_>>(),
            [expected]
        );
        let route = Route {
            side: Side::Right,
            start: [-500, y],
            distance: 180,
            vertical_distance: expected.1,
            window_us: [100_000, 500_000],
        };
        assert_eq!(route.position(Duration::from_millis(50)), [-500, y]);
        assert_eq!(
            route.position(Duration::from_millis(300)),
            [-410, y + expected.1 / 2]
        );
        assert_eq!(
            route.position(Duration::from_millis(700)),
            [-320, y + expected.1]
        );
    }
    assert_eq!(
        area.diagonal_steps(-800, 800, 180, 30.0, Vertical::Down)
            .next(),
        None
    );
}

#[test]
fn initial_size_uses_physical_height_and_falls_back_to_dip() {
    assert_eq!(initial_side_dip(1080, None, 1.25, 70), 179.2);
    assert!((initial_side_dip(1080, Some(300), 1.25, 70) - 96.768).abs() < 0.001);
    assert!(initial_side_dip(1080, Some(170), 1.25, 70) > 96.768);
}
