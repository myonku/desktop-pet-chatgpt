use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "ffmpeg-decoder")]
use super::ffmpeg::{Decoder, DecoderProcess};
use super::{
    Direction, Pack, PagePixels, Variant,
    frame::{Canvas, FrameView},
};
use crate::core::{IDLE, PAGE_TIMEOUT, Side};

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);

pub struct PageResult {
    source: u64,
    generation: u64,
    index: usize,
    pixels: io::Result<PagePixels>,
}

struct Request {
    generation: u64,
    index: usize,
    variant: Variant,
    startup_reply: Option<SyncSender<PageResult>>,
}

struct FrameSource {
    requests: Option<SyncSender<Request>>,
    worker: Option<thread::JoinHandle<()>>,
    generation: Arc<AtomicU64>,
    id: u64,
    pending: Option<u64>,
    #[cfg(feature = "ffmpeg-decoder")]
    decoder_process: Option<DecoderProcess>,
}

impl FrameSource {
    fn new(
        pack: Arc<Pack>,
        #[cfg(feature = "ffmpeg-decoder")] mut decoder: Option<Decoder>,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let id = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "ffmpeg-decoder")]
        let decoder_process = decoder.as_ref().map(Decoder::process);
        let worker = thread::Builder::new()
            .name("asset-page".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let cancelled = || current.load(Ordering::Relaxed) != request.generation;
                    #[cfg(feature = "ffmpeg-decoder")]
                    let pixels = match &mut decoder {
                        Some(decoder) => {
                            decoder.load_page(request.index, request.generation, cancelled)
                        }
                        None => pack.load_page(request.index, request.variant, cancelled),
                    };
                    #[cfg(not(feature = "ffmpeg-decoder"))]
                    let pixels = pack.load_page(request.index, request.variant, cancelled);
                    let result = PageResult {
                        source: id,
                        generation: request.generation,
                        index: request.index,
                        pixels,
                    };
                    let delivered = match request.startup_reply {
                        Some(reply) => reply.send(result).is_ok(),
                        None => deliver(result),
                    };
                    if !delivered {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            worker: Some(worker),
            generation,
            id,
            pending: None,
            #[cfg(feature = "ffmpeg-decoder")]
            decoder_process,
        })
    }

    fn request(
        &mut self,
        index: usize,
        variant: Variant,
        startup_reply: Option<SyncSender<PageResult>>,
    ) -> io::Result<()> {
        if self.pending.is_none() {
            let generation = self.generation.load(Ordering::Relaxed);
            self.requests
                .as_ref()
                .unwrap()
                .try_send(Request {
                    generation,
                    index,
                    variant,
                    startup_reply,
                })
                .map_err(io::Error::other)?;
            self.pending = Some(generation);
        }
        Ok(())
    }

    fn accept(&mut self, result: PageResult) -> Option<(usize, io::Result<PagePixels>)> {
        if result.source != self.id || self.pending != Some(result.generation) {
            return None;
        }
        self.pending = None;
        (result.generation == self.generation.load(Ordering::Relaxed))
            .then_some((result.index, result.pixels))
    }

    fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "ffmpeg-decoder")]
        if let Some(process) = &self.decoder_process {
            process.stop();
        }
    }
}

impl Drop for FrameSource {
    fn drop(&mut self) {
        self.cancel();
        drop(self.requests.take());
        crate::platform::reap(self.worker.take());
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

struct Animation {
    id: String,
    timeline: Timeline,
    frames: Vec<(usize, [u32; 4])>,
    canvas: Canvas,
    movement: Option<[u64; 2]>,
    directions: [bool; 2],
}

#[derive(Clone, Copy)]
struct Selected {
    sequence: u128,
    rect: [u32; 4],
    canvas: Canvas,
}

pub struct Playback {
    source: FrameSource,
    actions: Vec<Animation>,
    base: usize,
    active: usize,
    once: bool,
    variant: Variant,
    epoch: Option<Instant>,
    waiting: Option<(Instant, Duration)>,
    displayed: Option<(usize, PagePixels)>,
    ready: Option<(usize, PagePixels)>,
    failed: Option<(usize, io::Error)>,
    selected: Option<Selected>,
    present_sequence: Option<u128>,
    pack_id: String,
    announced: bool,
}

pub struct PlaybackTick {
    pub changed: bool,
    pub deadline: Option<Instant>,
    pub elapsed: Option<Duration>,
    pub finished: bool,
}

impl Playback {
    pub fn open(
        pack: Arc<Pack>,
        variant: Variant,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let manifest = pack.manifest();
        let base = manifest
            .actions
            .iter()
            .position(|action| action.id == IDLE && action.looping)
            .or(match manifest.actions.as_slice() {
                [action]
                    if action.looping && matches!(action.id.as_str(), "move" | "reference") =>
                {
                    Some(0)
                }
                _ => None,
            })
            .ok_or_else(|| io::Error::other("asset pack needs a looping idle action"))?;
        let pages: std::collections::BTreeMap<_, _> = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| (page.id.as_str(), index))
            .collect();
        let actions = manifest
            .actions
            .iter()
            .map(|action| Animation {
                id: action.id.clone(),
                timeline: Timeline {
                    ends: action
                        .frames
                        .iter()
                        .scan(0, |elapsed, frame| {
                            *elapsed += frame.duration_us;
                            Some(*elapsed)
                        })
                        .collect(),
                    looping: action.looping,
                },
                frames: action
                    .frames
                    .iter()
                    .map(|frame| (pages[frame.page.as_str()], frame.rect))
                    .collect(),
                canvas: Canvas {
                    size: action.logical_size,
                    anchor: action.anchor,
                    offset: action.offset,
                    scale: action.scale,
                },
                movement: action.movement_us,
                directions: [
                    action
                        .directions
                        .iter()
                        .any(|value| matches!(value, Direction::Left)),
                    action
                        .directions
                        .iter()
                        .any(|value| matches!(value, Direction::Right)),
                ],
            })
            .collect();
        let pack_id = manifest.pack_id.clone();
        #[cfg(feature = "ffmpeg-decoder")]
        let decoder = (variant == Variant::Ffmpeg)
            .then(|| Decoder::new(manifest))
            .transpose()?;
        Ok(Self {
            source: FrameSource::new(
                pack,
                #[cfg(feature = "ffmpeg-decoder")]
                decoder,
                deliver,
            )?,
            actions,
            base,
            active: base,
            once: false,
            variant,
            pack_id,
            announced: false,
            epoch: None,
            waiting: None,
            displayed: None,
            ready: None,
            failed: None,
            selected: None,
            present_sequence: None,
        })
    }

    pub fn action_ids(&self) -> impl Iterator<Item = &str> {
        self.actions
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.base)
            .map(|(_, action)| action.id.as_str())
    }

    pub fn load_first_frame(&mut self) -> io::Result<()> {
        let (page, _) = self.actions[self.base].frames[0];
        let (send, receive) = mpsc::sync_channel(1);
        self.source.request(page, self.variant, Some(send))?;
        let result = receive
            .recv_timeout(PAGE_TIMEOUT)
            .map_err(io::Error::other)?;
        let (_, pixels) = self.source.accept(result).unwrap();
        self.ready = Some((page, pixels?));
        self.tick(Instant::now())?;
        // The animation clock starts when the event loop begins playback.
        self.epoch = None;
        Ok(())
    }

    pub fn contains(&self, id: &str) -> bool {
        self.actions.iter().any(|action| action.id == id)
    }

    pub fn movement(&self, side: Side) -> Option<[u64; 2]> {
        let action = self
            .actions
            .iter()
            .find(|action| action.id == side.movement())?;
        let index = usize::from(side == Side::Right);
        action.movement.filter(|[start, end]| {
            start < end && action.directions[index] && !action.timeline.looping
        })
    }

    pub fn play(&mut self, id: Option<&str>, once: bool) -> bool {
        let index = match id {
            Some(id) => self.actions.iter().position(|action| action.id == id),
            None => Some(self.base),
        };
        let Some(index) = index else {
            return false;
        };
        self.pause();
        self.active = index;
        self.once = once;
        self.present_sequence = None;
        true
    }

    pub fn active_id(&self) -> &str {
        &self.actions[self.active].id
    }

    pub fn is_base(&self) -> bool {
        self.active == self.base && !self.once
    }

    pub fn pause(&mut self) {
        self.source.cancel();
        self.epoch = None;
        self.waiting = None;
        self.ready = None;
        self.failed = None;
    }

    pub fn use_compatible_format(&mut self) {
        if self.variant == Variant::Bc7 {
            self.variant = Variant::Rgba8;
            self.pause();
            self.displayed = None;
            self.selected = None;
            self.present_sequence = None;
            eprintln!("BC7 unavailable on this device; using RGBA8");
        }
    }

    pub fn accept(&mut self, result: PageResult) {
        if let Some((index, pixels)) = self.source.accept(result) {
            match pixels {
                Ok(pixels) => self.ready = Some((index, pixels)),
                Err(error) => self.failed = Some((index, error)),
            }
        }
    }

    pub fn frame(&self) -> Option<(usize, FrameView<'_>)> {
        let (index, pixels) = self.displayed.as_ref()?;
        let selected = self.selected?;
        Some((
            *index,
            FrameView {
                page_size: pixels.size,
                rect: selected.rect,
                canvas: selected.canvas,
                rgba: &pixels.rgba,
                bc7: pixels.bc7.as_deref(),
            },
        ))
    }

    pub fn sequence(&self) -> Option<u128> {
        self.selected.map(|frame| frame.sequence)
    }

    pub fn tick(&mut self, now: Instant) -> io::Result<PlaybackTick> {
        let action = &self.actions[self.active];
        let total = Duration::from_micros(*action.timeline.ends.last().unwrap());
        if self
            .waiting
            .is_some_and(|(since, _)| now >= since + PAGE_TIMEOUT)
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "asset page wait timed out",
            ));
        }
        let elapsed = self.waiting.map_or_else(
            || self.epoch.map_or(Duration::ZERO, |start| now - start),
            |(_, elapsed)| elapsed,
        );
        let (sequence, delay) = action.timeline.at(if self.once && elapsed >= total {
            total - Duration::from_micros(1)
        } else {
            elapsed
        });
        let frame_count = action.frames.len() as u128;
        let frame = (sequence % frame_count) as usize;
        let (page, rect) = action.frames[frame];
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
                let (since, _) = *self.waiting.get_or_insert((now, elapsed));
                let deadline = since + PAGE_TIMEOUT;
                if let Some((index, error)) = self.failed.take()
                    && index == page
                {
                    return Err(error);
                }
                self.ready = None;
                self.source.request(page, self.variant, None)?;
                return Ok(PlaybackTick {
                    changed: false,
                    deadline: Some(deadline),
                    elapsed: None,
                    finished: false,
                });
            }
        }
        if !self.announced {
            self.announced = true;
            let format = self.displayed.as_ref().unwrap().1.variant.label();
            eprintln!(
                "Asset playback: {} ({format}, {} frames)",
                self.pack_id,
                action.frames.len()
            );
        }
        if self.waiting.take().is_some() || self.epoch.is_none() {
            self.epoch = Some(now - elapsed);
        }
        changed |= self
            .present_sequence
            .replace(sequence)
            .is_none_or(|previous| previous % frame_count != frame as u128);
        self.selected = Some(Selected {
            sequence,
            rect,
            canvas: action.canvas,
        });
        let finished = (self.once || !action.timeline.looping) && elapsed >= total;
        if !finished && self.ready.is_none() {
            let next = (frame + 1) % action.frames.len();
            let next_page = action.frames[next].0;
            if (next != 0 || (!self.once && action.timeline.looping))
                && next_page != page
                && self
                    .failed
                    .as_ref()
                    .is_none_or(|(index, _)| *index != next_page)
            {
                self.source.request(next_page, self.variant, None)?;
            }
        }
        Ok(PlaybackTick {
            changed,
            deadline: (!finished)
                .then_some(delay)
                .flatten()
                .map(|delay| now + delay),
            elapsed: Some(if self.once || !action.timeline.looping {
                elapsed.min(total)
            } else {
                elapsed
            }),
            finished,
        })
    }
}

#[cfg(test)]
mod tests;
