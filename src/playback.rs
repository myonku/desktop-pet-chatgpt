use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "p1b-reference")]
use crate::reference::{Decoder, DecoderProcess};
use crate::sample::FrameView;
use desktop_pet_chatgpt::media::{Pack, PagePixels, Variant};

pub struct PageResult {
    generation: u64,
    index: usize,
    pixels: io::Result<PagePixels>,
}

struct Request {
    generation: u64,
    index: usize,
    variant: Variant,
}

struct FrameSource {
    requests: SyncSender<Request>,
    generation: Arc<AtomicU64>,
    pending: bool,
    #[cfg(feature = "p1b-reference")]
    decoder_process: Option<DecoderProcess>,
}

impl FrameSource {
    fn new(
        pack: Arc<Pack>,
        #[cfg(feature = "p1b-reference")] mut decoder: Option<Decoder>,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        #[cfg(feature = "p1b-reference")]
        let decoder_process = decoder.as_ref().map(Decoder::process);
        thread::Builder::new()
            .name("asset-page".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let cancelled = || current.load(Ordering::Relaxed) != request.generation;
                    #[cfg(feature = "p1b-reference")]
                    let pixels = match &mut decoder {
                        Some(decoder) => {
                            decoder.load_page(request.index, request.generation, cancelled)
                        }
                        None => pack.load_page(request.index, request.variant, cancelled),
                    };
                    #[cfg(not(feature = "p1b-reference"))]
                    let pixels = pack.load_page(request.index, request.variant, cancelled);
                    let result = PageResult {
                        generation: request.generation,
                        index: request.index,
                        pixels,
                    };
                    if !deliver(result) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            generation,
            pending: false,
            #[cfg(feature = "p1b-reference")]
            decoder_process,
        })
    }

    fn request(&mut self, index: usize, variant: Variant) -> io::Result<()> {
        if !self.pending {
            self.requests
                .try_send(Request {
                    generation: self.generation.load(Ordering::Relaxed),
                    index,
                    variant,
                })
                .map_err(io::Error::other)?;
            self.pending = true;
        }
        Ok(())
    }

    fn accept(&mut self, result: PageResult) -> Option<(usize, io::Result<PagePixels>)> {
        self.pending = false;
        (result.generation == self.generation.load(Ordering::Relaxed))
            .then_some((result.index, result.pixels))
    }

    fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "p1b-reference")]
        if let Some(process) = &self.decoder_process {
            process.stop();
        }
    }
}

impl Drop for FrameSource {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Timeline {
    ends: Vec<u64>,
    looping: bool,
}

impl Timeline {
    fn at(&self, elapsed: Duration) -> (u128, Option<Duration>) {
        let total = *self.ends.last().unwrap() as u128;
        let elapsed = elapsed.as_micros();
        if !self.looping && elapsed >= total {
            return ((self.ends.len() - 1) as u128, None);
        }
        let position = (elapsed % total) as u64;
        let index = self.ends.partition_point(|&end| end <= position);
        (
            elapsed / total * self.ends.len() as u128 + index as u128,
            Some(Duration::from_micros(self.ends[index] - position)),
        )
    }
}

pub struct Playback {
    source: FrameSource,
    timeline: Timeline,
    frames: Vec<(usize, [u32; 4])>,
    variant: Variant,
    epoch: Option<Instant>,
    displayed: Option<(usize, PagePixels)>,
    ready: Option<(usize, PagePixels)>,
    selected: Option<u128>,
    pack_id: String,
}

impl Playback {
    pub fn open(
        path: &Path,
        variant: Variant,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let pack = Arc::new(Pack::open(path)?);
        let manifest = pack.manifest();
        let action = &manifest.actions[0];
        let pages: std::collections::BTreeMap<_, _> = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| (page.id.as_str(), index))
            .collect();
        let frames = action
            .frames
            .iter()
            .map(|frame| (pages[frame.page.as_str()], frame.rect))
            .collect();
        let ends = action
            .frames
            .iter()
            .scan(0, |elapsed, frame| {
                *elapsed += frame.duration_us;
                Some(*elapsed)
            })
            .collect();
        let timeline = Timeline {
            ends,
            looping: action.looping,
        };
        let pack_id = manifest.pack_id.clone();
        #[cfg(feature = "p1b-reference")]
        let decoder = (variant == Variant::Ffmpeg)
            .then(|| Decoder::new(manifest))
            .transpose()?;
        Ok(Self {
            source: FrameSource::new(
                pack,
                #[cfg(feature = "p1b-reference")]
                decoder,
                deliver,
            )?,
            timeline,
            frames,
            variant,
            pack_id,
            epoch: None,
            displayed: None,
            ready: None,
            selected: None,
        })
    }

    pub fn pause(&mut self) {
        self.source.cancel();
        self.epoch = None;
        self.ready = None;
    }

    pub fn use_compatible_format(&mut self) {
        if self.variant == Variant::Bc7 {
            self.variant = Variant::Rgba8;
            self.pause();
            self.displayed = None;
            self.selected = None;
            eprintln!("BC7 unavailable on this device; using RGBA8");
        }
    }

    pub fn invalidate_frame(&mut self) {
        self.selected = None;
    }

    pub fn accept(&mut self, result: PageResult) -> io::Result<()> {
        if let Some((index, pixels)) = self.source.accept(result) {
            self.ready = Some((index, pixels?));
        }
        Ok(())
    }

    pub fn frame(&self) -> Option<(usize, FrameView<'_>)> {
        let (_, pixels) = self.displayed.as_ref()?;
        let (index, rect) = self.frames[(self.selected? % self.frames.len() as u128) as usize];
        Some((
            index,
            FrameView {
                page_size: pixels.size,
                rect,
                rgba: &pixels.rgba,
                bc7: pixels.bc7.as_deref(),
            },
        ))
    }

    pub fn sequence(&self) -> Option<u128> {
        self.selected
    }

    pub fn tick(&mut self, now: Instant) -> io::Result<(bool, Option<Instant>)> {
        let (sequence, delay) = self
            .timeline
            .at(self.epoch.map_or(Duration::ZERO, |start| now - start));
        let frame_count = self.frames.len() as u128;
        let frame = (sequence % frame_count) as usize;
        let (page, _) = self.frames[frame];
        let mut changed = false;
        if self
            .displayed
            .as_ref()
            .is_none_or(|(index, _)| *index != page)
        {
            if self.ready.as_ref().is_some_and(|(index, _)| *index == page) {
                let actual = self.ready.as_ref().unwrap().1.variant;
                if actual != self.variant
                    && self
                        .displayed
                        .as_ref()
                        .is_none_or(|(_, pixels)| pixels.variant != actual)
                {
                    eprintln!(
                        "Asset playback format fallback: requested {}; using {}",
                        self.variant.label(),
                        actual.label()
                    );
                }
                self.displayed = self.ready.take();
                changed = true;
            } else {
                self.ready = None;
                self.source.request(page, self.variant)?;
                return Ok((false, None));
            }
        }
        if self.selected.is_none() {
            let pixels = &self.displayed.as_ref().unwrap().1;
            let format = pixels.variant.label();
            eprintln!(
                "Asset playback: {} ({format}, {} frames)",
                self.pack_id,
                self.frames.len()
            );
        }
        self.epoch.get_or_insert(now);
        changed |= self
            .selected
            .replace(sequence)
            .is_none_or(|previous| previous % frame_count != frame as u128);
        if delay.is_some() && self.ready.is_none() {
            let next = (frame + 1) % self.frames.len();
            let (next_page, _) = self.frames[next];
            if (next != 0 || self.timeline.looping) && next_page != page {
                self.source.request(next_page, self.variant)?;
            }
        }
        Ok((changed, delay.and_then(|delay| now.checked_add(delay))))
    }
}

#[cfg(test)]
mod tests {
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
}
