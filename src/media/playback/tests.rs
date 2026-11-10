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
        requests,
        generation: Arc::new(AtomicU64::new(0)),
        pending: true,
        #[cfg(feature = "p1b-reference")]
        decoder_process: None,
    };
    source.cancel();
    let result = PageResult {
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
    assert!(!source.pending);
    source.request(0, Variant::Png).unwrap();
    assert!(source.pending);
}
