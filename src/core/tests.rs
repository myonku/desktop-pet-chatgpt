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
        Some(Click::Quota)
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
    assert_eq!(pet.auto_deadline(), Some(now + AUTO_WAIT));
    pet.activity = Activity::Auto(route);
    pet.finish(now);
    assert_eq!(pet.auto_deadline(), Some(now + AUTO_COOLDOWN));
    pet.manual_hidden = true;
    pet.fullscreen = true;
    pet.reset(now);
    assert!(!pet.visible());
    assert_eq!(pet.auto_deadline(), None);
    pet.fullscreen = false;
    assert!(!pet.visible());
    pet.manual_hidden = false;
    assert_eq!(pet.auto_deadline(), Some(now + AUTO_WAIT));
    pet.auto_enabled = false;
    assert_eq!(pet.auto_deadline(), None);
    pet.auto_enabled = true;
    pet.edge = Some(Side::Right);
    assert_eq!(pet.auto_deadline(), None);
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
fn initial_size_uses_physical_height_and_falls_back_to_dip() {
    assert_eq!(initial_side_dip(1080, None, 1.25, 70), 179.2);
    assert!((initial_side_dip(1080, Some(300), 1.25, 70) - 96.768).abs() < 0.001);
    assert!(initial_side_dip(1080, Some(170), 1.25, 70) > 96.768);
}
