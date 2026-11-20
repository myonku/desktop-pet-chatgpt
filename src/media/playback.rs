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

use super::{
    Direction, Pack, PagePixels,
    frame::{Canvas, FrameView},
    manifest::ActionBindings,
};
use crate::core::{ActionChoice, Behavior, PAGE_TIMEOUT, Side, Vertical};

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
    startup_reply: Option<SyncSender<PageResult>>,
}

struct FrameSource {
    requests: Option<SyncSender<Request>>,
    worker: Option<thread::JoinHandle<()>>,
    generation: Arc<AtomicU64>,
    id: u64,
    pending: Option<u64>,
}

impl FrameSource {
    fn new(
        pack: Arc<Pack>,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let id = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
        let worker = thread::Builder::new()
            .name("asset-page".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let cancelled = || current.load(Ordering::Relaxed) != request.generation;
                    let pixels = pack.load_page(request.index, cancelled);
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
        })
    }

    fn request(
        &mut self,
        index: usize,
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
    }
}

impl Drop for FrameSource {
    fn drop(&mut self) {
        self.cancel();
        drop(self.requests.take());
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_millis(100);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                eprintln!("Timed out waiting for asset-page worker");
            }
        }
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
    preview_name: Option<String>,
    timeline: Timeline,
    frames: Vec<(usize, [u32; 4])>,
    canvas: Canvas,
    movement: Option<[u64; 2]>,
    directions: [bool; 2],
}

impl Animation {
    fn next_page(&self, frame: usize, once: bool) -> Option<usize> {
        let page = self.frames[frame].0;
        self.frames[frame + 1..]
            .iter()
            .find(|next| next.0 != page)
            .map(|next| next.0)
            .or_else(|| {
                let first = self.frames[0].0;
                (!once && self.timeline.looping && first != page).then_some(first)
            })
    }

    fn shares_pose_frames(&self, other: &Self) -> bool {
        self.frames == other.frames
            && self.timeline.ends == other.timeline.ends
            && self.canvas.size == other.canvas.size
            && self.canvas.scale == other.canvas.scale
            && self.canvas.anchor == other.canvas.anchor
            && self.canvas.offset[1] == other.canvas.offset[1]
    }
}

#[derive(Clone, Copy)]
struct Selected {
    edge_pose: bool,
    sequence: u128,
    rect: [u32; 4],
    canvas: Canvas,
}

const POSE_SLIDE: Duration = Duration::from_millis(180);
const POSE_UPDATE: Duration = Duration::from_millis(16);

struct PoseSlide {
    from_x: f32,
    start: Option<Instant>,
}

pub struct Playback {
    source: FrameSource,
    actions: Vec<Animation>,
    bindings: ActionBindings,
    base: usize,
    active: usize,
    once: bool,
    edge_pose: bool,
    epoch: Option<Instant>,
    waiting: Option<(Instant, Duration)>,
    displayed: Option<(usize, PagePixels)>,
    ready: Option<(usize, PagePixels)>,
    failed: Option<(usize, io::Error)>,
    selected: Option<Selected>,
    present_sequence: Option<u128>,
    pose_slide: Option<PoseSlide>,
    pack_id: String,
    announced: bool,
}

pub struct PlaybackTick {
    pub changed: bool,
    pub deadline: Option<Instant>,
    pub elapsed: Option<Duration>,
    pub finished: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageKey {
    source: u64,
    pub index: usize,
}

impl Playback {
    pub fn open(
        pack: Arc<Pack>,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let manifest = pack.manifest();
        let base = pack.startup_action();
        let pages: std::collections::BTreeMap<_, _> = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| (page.id.as_str(), index))
            .collect();
        let actions = manifest
            .clips
            .iter()
            .map(|action| Animation {
                id: action.id.clone(),
                preview_name: action.preview_name.clone(),
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
        let bindings = pack.bindings().clone();
        Ok(Self {
            source: FrameSource::new(pack, deliver)?,
            actions,
            bindings,
            base,
            active: base,
            once: false,
            edge_pose: false,
            pack_id,
            announced: false,
            epoch: None,
            waiting: None,
            displayed: None,
            ready: None,
            failed: None,
            selected: None,
            present_sequence: None,
            pose_slide: None,
        })
    }

    pub fn clip_previews(&self) -> impl Iterator<Item = (usize, &str)> {
        self.actions
            .iter()
            .enumerate()
            .map(|(index, action)| (index, action.preview_name.as_deref().unwrap_or(&action.id)))
    }

    pub fn load_first_frame(&mut self) -> io::Result<()> {
        let (page, _) = self.actions[self.base].frames[0];
        let (send, receive) = mpsc::sync_channel(1);
        self.source.request(page, Some(send))?;
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

    pub fn action_index(&self, behavior: Behavior) -> Option<usize> {
        self.bindings.get(&behavior).map(|group| group.default)
    }

    pub fn select_action(
        &self,
        behavior: Behavior,
        choice: ActionChoice<'_>,
        available: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        self.bindings.get(&behavior)?.select(choice, available)
    }

    pub fn has_idle_variant(&self, available: impl Fn(usize) -> bool) -> bool {
        self.bindings[&Behavior::Idle]
            .variants
            .values()
            .copied()
            .any(|index| index != self.base && available(index))
    }

    pub fn select_diagonal(
        &self,
        side: Side,
        vertical: Vertical,
        available: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        self.select_action(side.diagonal(vertical), ActionChoice::Random, &available)
            .or_else(|| {
                self.select_action(
                    side.movement(),
                    ActionChoice::ExactVariant("fly"),
                    available,
                )
            })
    }

    pub fn has_diagonal(&self) -> bool {
        Behavior::DIAGONAL_MOVES
            .into_iter()
            .any(|behavior| self.bindings.contains_key(&behavior))
            || [Side::Left, Side::Right].into_iter().any(|side| {
                self.select_action(side.movement(), ActionChoice::ExactVariant("fly"), |_| true)
                    .is_some()
            })
    }

    pub fn movement(&self, index: usize, side: Side) -> Option<[u64; 2]> {
        let action = &self.actions[index];
        let direction = usize::from(side == Side::Right);
        action.movement.filter(|[start, end]| {
            start < end && action.directions[direction] && !action.timeline.looping
        })
    }

    pub fn play(&mut self, id: Option<Behavior>, once: bool) -> bool {
        let index = match id {
            Some(id) => self.action_index(id),
            None => Some(self.base),
        };
        let Some(index) = index else {
            return false;
        };
        self.play_index(index, once);
        self.edge_pose = id.is_some_and(Behavior::is_edge);
        true
    }

    pub fn play_index(&mut self, index: usize, once: bool) {
        self.pause();
        self.active = index;
        self.once = once;
        self.edge_pose = false;
        self.present_sequence = None;
    }

    pub fn switch_pose(&mut self, target: Behavior, previous: Behavior, now: Instant) -> bool {
        let Some(index) = self.action_index(target) else {
            return false;
        };
        if !self.once && self.edge_pose && self.active == index {
            return true;
        }
        let source = &self.actions[self.active];
        let destination = &self.actions[index];
        if self.once
            || !self.edge_pose
            || self.action_index(previous) != Some(self.active)
            || !source.timeline.looping
            || !destination.timeline.looping
            || source.timeline.ends != destination.timeline.ends
            || self.present_sequence.is_none()
        {
            self.play_index(index, false);
            self.edge_pose = true;
            return true;
        }
        let shared_frames = source.frames == destination.frames;
        let from = self.selected.unwrap().canvas;
        let to = destination.canvas;
        let compatible_slide = source.shares_pose_frames(destination);
        if !shared_frames || self.waiting.is_some() {
            // Some clips bake each crop into separate pages. Keep their phase while
            // the matching page loads, but leave their authored geometry intact.
            let epoch = self
                .waiting
                .map_or(self.epoch, |(_, elapsed)| now.checked_sub(elapsed));
            self.pause();
            self.epoch = epoch;
        }
        self.active = index;
        self.present_sequence = None;
        self.pose_slide =
            (compatible_slide && from.offset[0] != to.offset[0]).then_some(PoseSlide {
                from_x: from.offset[0],
                start: Some(now),
            });
        true
    }

    pub fn enter_pose(&mut self, edge: Behavior, peek: Behavior) {
        if self.play(Some(edge), false) {
            let edge = &self.actions[self.active];
            self.pose_slide = self
                .action_index(peek)
                .map(|index| &self.actions[index])
                .filter(|action| {
                    edge.shares_pose_frames(action)
                        && edge.canvas.offset[0] != action.canvas.offset[0]
                })
                .map(|action| PoseSlide {
                    from_x: action.canvas.offset[0],
                    start: None,
                });
        }
    }

    pub fn sliding_pose(&self) -> bool {
        self.pose_slide.is_some()
    }

    pub fn selected_is_edge_pose(&self) -> bool {
        self.selected.is_some_and(|frame| frame.edge_pose)
    }

    pub fn active_index(&self) -> usize {
        self.active
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
        self.pose_slide = None;
    }

    pub fn accept(&mut self, result: PageResult) {
        if let Some((index, pixels)) = self.source.accept(result) {
            match pixels {
                Ok(pixels) => self.ready = Some((index, pixels)),
                Err(error) => self.failed = Some((index, error)),
            }
        }
    }

    pub fn frame(&self) -> Option<(PageKey, FrameView<'_>)> {
        let (index, pixels) = self.displayed.as_ref()?;
        let selected = self.selected?;
        Some((
            PageKey {
                source: self.source.id,
                index: *index,
            },
            FrameView {
                page_size: pixels.size,
                rect: selected.rect,
                canvas: selected.canvas,
                rgba: &pixels.rgba,
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
                self.source.request(page, None)?;
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
            eprintln!(
                "Asset playback: {} (PNG, {} frames)",
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
        let mut canvas = action.canvas;
        let mut slide_deadline = None;
        if let Some(slide) = &mut self.pose_slide {
            let elapsed = now - *slide.start.get_or_insert(now);
            if elapsed >= POSE_SLIDE {
                self.pose_slide = None;
            } else {
                let t = elapsed.as_secs_f32() / POSE_SLIDE.as_secs_f32();
                let eased = t * t * (3.0 - 2.0 * t);
                canvas.offset[0] = slide.from_x + (canvas.offset[0] - slide.from_x) * eased;
                slide_deadline = Some(now + POSE_UPDATE.min(POSE_SLIDE - elapsed));
            }
        }
        changed |= self
            .selected
            .is_some_and(|selected| selected.canvas.offset != canvas.offset);
        self.selected = Some(Selected {
            edge_pose: self.edge_pose,
            sequence,
            rect,
            canvas,
        });
        let finished = (self.once || !action.timeline.looping) && elapsed >= total;
        if !finished
            && self.ready.is_none()
            && let Some(next_page) = action.next_page(frame, self.once)
            && self
                .failed
                .as_ref()
                .is_none_or(|(index, _)| *index != next_page)
        {
            self.source.request(next_page, None)?;
        }
        Ok(PlaybackTick {
            changed,
            deadline: (!finished)
                .then_some(delay)
                .flatten()
                .map(|delay| now + delay)
                .into_iter()
                .chain(slide_deadline)
                .min(),
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
