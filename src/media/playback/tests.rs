use super::*;

#[test]
fn timeline_skips_late_frames_and_wraps_at_total_duration() {
    let mut timeline = Timeline {
        ends: vec![10, 30, 60],
        looping: true,
    };
    assert_eq!(
        timeline.at(Duration::from_micros(42)),
        (2, Some(Duration::from_micros(18)))
    );
    assert_eq!(
        timeline.at(Duration::from_micros(60)),
        (3, Some(Duration::from_micros(10)))
    );
    timeline.looping = false;
    assert_eq!(timeline.at(Duration::from_micros(61)), (2, None));
}

#[test]
fn cancelled_generation_rejects_old_completion_and_releases_request_slot() {
    let (requests, _receiver) = mpsc::sync_channel(1);
    let mut source = FrameSource {
        requests: Some(requests),
        worker: None,
        generation: Arc::new(AtomicU64::new(0)),
        id: 42,
        pending: Some(0),
        #[cfg(feature = "ffmpeg-decoder")]
        decoder_process: None,
    };
    source.cancel();
    let result = PageResult {
        source: 42,
        generation: 0,
        index: 0,
        pixels: Ok(PagePixels {
            size: [1, 1],
            rgba: vec![0; 4],
            bc7: None,
            variant: Variant::Png,
        }),
    };
    assert!(source.accept(result).is_none());
    assert_eq!(source.pending, None);
    source.request(0, Variant::Png, None).unwrap();
    assert_eq!(source.pending, Some(1));
}

#[test]
fn page_source_rejects_wrong_instance_without_releasing_current_request() {
    let (requests, _receiver) = mpsc::sync_channel(1);
    let mut source = FrameSource {
        requests: Some(requests),
        worker: None,
        generation: Arc::new(AtomicU64::new(0)),
        id: 42,
        pending: Some(0),
        #[cfg(feature = "ffmpeg-decoder")]
        decoder_process: None,
    };
    assert!(
        source
            .accept(PageResult {
                source: 41,
                generation: 0,
                index: 0,
                pixels: Err(io::Error::other("old"))
            })
            .is_none()
    );
    assert_eq!(source.pending, Some(0));
}

fn test_player() -> (Playback, mpsc::Receiver<Request>) {
    let (requests, receiver) = mpsc::sync_channel(1);
    let animation = |id: &str, looping, page| Animation {
        id: id.into(),
        timeline: Timeline {
            ends: vec![100_000, 200_000],
            looping,
        },
        frames: vec![(page, [0, 0, 1, 1]), (page + 1, [0, 0, 1, 1])],
        canvas: Canvas::untransformed([1, 1]),
        movement: Some([10, 20]),
        directions: [true, false],
    };
    let mut player = Playback {
        source: FrameSource {
            requests: Some(requests),
            worker: None,
            generation: Arc::new(AtomicU64::new(0)),
            id: 42,
            pending: None,
            #[cfg(feature = "ffmpeg-decoder")]
            decoder_process: None,
        },
        actions: vec![
            animation("move-left", false, 3),
            animation("interact", false, 2),
            animation("idle", true, 0),
        ],
        base: 2,
        active: 2,
        once: false,
        variant: Variant::Png,
        epoch: None,
        waiting: None,
        displayed: None,
        ready: None,
        failed: None,
        selected: None,
        present_sequence: None,
        pack_id: "test".into(),
        announced: false,
    };
    assert_eq!(player.movement(Side::Left), Some([10, 20]));
    assert_eq!(player.movement(Side::Right), None);
    assert!(!player.play(Some("missing"), true));
    (player, receiver)
}

fn ready(request: Request) -> PageResult {
    PageResult {
        source: 42,
        generation: request.generation,
        index: request.index,
        pixels: Ok(PagePixels {
            size: [1, 1],
            rgba: vec![255; 4],
            bc7: None,
            variant: Variant::Png,
        }),
    }
}

#[test]
fn action_switch_holds_frame_discards_stale_page_and_completes_once() {
    let (mut player, receiver) = test_player();
    let now = Instant::now();
    assert_eq!(player.tick(now).unwrap().deadline, Some(now + PAGE_TIMEOUT));
    player.accept(ready(receiver.try_recv().unwrap()));
    assert!(player.tick(now).unwrap().changed);
    let stale = receiver.try_recv().unwrap();
    assert!(player.play(Some("interact"), true));
    assert!(!player.tick(now).unwrap().changed);
    assert_eq!(player.frame().unwrap().0.index, 0);
    player.accept(ready(stale));
    assert!(!player.tick(now).unwrap().changed);
    player.accept(ready(receiver.try_recv().unwrap()));
    assert!(player.tick(now).unwrap().changed);
    assert_eq!(player.frame().unwrap().0.index, 2);
    let next = receiver.try_recv().unwrap();
    let missing_at = now + Duration::from_millis(100);
    assert_eq!(player.tick(missing_at).unwrap().elapsed, None);
    assert_eq!(player.frame().unwrap().0.index, 2);
    player.accept(ready(next));
    let resume = missing_at + Duration::from_millis(40);
    assert_eq!(
        player.tick(resume).unwrap().elapsed,
        Some(Duration::from_millis(100))
    );
    assert!(
        !player
            .tick(resume + Duration::from_millis(99))
            .unwrap()
            .finished
    );
    assert!(
        player
            .tick(resume + Duration::from_millis(100))
            .unwrap()
            .finished
    );
    player.play(None, false);
    assert_eq!(player.active_id(), "idle");
}

#[test]
fn first_page_wait_is_bounded_and_a_retry_keeps_the_prior_frame() {
    let (mut player, receiver) = test_player();
    let now = Instant::now();
    player.tick(now).unwrap();
    player.accept(ready(receiver.try_recv().unwrap()));
    player.tick(now).unwrap();
    assert!(player.play(Some("interact"), true));
    player.tick(now).unwrap();
    let Err(error) = player.tick(now + PAGE_TIMEOUT) else {
        panic!("missing page never timed out");
    };
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(player.frame().unwrap().0.index, 0);
    player.play(None, false);
    assert!(player.tick(now + PAGE_TIMEOUT).unwrap().changed);
}
