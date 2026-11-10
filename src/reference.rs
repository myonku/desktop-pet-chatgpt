//! Development-only VP9 alpha reference for the bounded page worker.

use std::{
    io::{self, Read},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
};

use desktop_pet_chatgpt::media::{Manifest, PLAYBACK_PAGE_LIMIT, PagePixels, Variant};
use sha2::{Digest, Sha256};
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    window::WindowAttributes,
};

pub fn client_geometry(attributes: WindowAttributes) -> io::Result<WindowAttributes> {
    let value = match std::env::var("DESKTOP_PET_REFERENCE_CLIENT") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(attributes),
        Err(error) => return Err(io::Error::other(error)),
    };
    let fields: Vec<_> = value.split(',').collect();
    if let [side, x, y] = fields.as_slice()
        && let (Ok(side @ (256 | 320 | 512)), Ok(x), Ok(y)) =
            (side.parse(), x.parse::<i32>(), y.parse::<i32>())
    {
        return Ok(attributes
            .with_inner_size(PhysicalSize::new(side, side))
            .with_position(PhysicalPosition::new(x, y)));
    }
    Err(io::Error::other(
        "DESKTOP_PET_REFERENCE_CLIENT needs side,x,y with side 256, 320 or 512",
    ))
}

#[derive(Clone, Default)]
pub struct DecoderProcess(Arc<Mutex<Option<Child>>>);

impl DecoderProcess {
    pub fn stop(&self) {
        if let Some(mut child) = self.0.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Page {
    first: usize,
    count: usize,
    digest: Option<String>,
}

pub struct Decoder {
    exe: PathBuf,
    video: PathBuf,
    size: [u32; 2],
    pages: Vec<Option<Page>>,
    frame_count: usize,
    next: usize,
    stdout: Option<(u64, ChildStdout)>,
    process: DecoderProcess,
}

impl Decoder {
    pub fn new(manifest: &Manifest) -> io::Result<Self> {
        let action = &manifest.actions[0];
        let [width, height] = action.logical_size;
        let invalid = || {
            io::Error::other("reference needs a looping action with consecutive full-frame strips")
        };
        if !action.looping {
            return Err(invalid());
        }
        let mut pages: Vec<Option<Page>> = (0..manifest.pages.len()).map(|_| None).collect();
        let indices: std::collections::BTreeMap<_, _> = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(i, page)| (page.id.as_str(), i))
            .collect();
        let mut first = 0;
        let verify = std::env::var_os("DESKTOP_PET_REFERENCE_VERIFY").is_some();
        for group in action.frames.chunk_by(|a, b| a.page == b.page) {
            let i = indices[group[0].page.as_str()];
            let page = &manifest.pages[i];
            if pages[i].is_some()
                || page.size != [width * group.len() as u32, height]
                || page.size[0] as usize * height as usize * 4 > PLAYBACK_PAGE_LIMIT
                || group
                    .iter()
                    .enumerate()
                    .any(|(slot, frame)| frame.rect != [slot as u32 * width, 0, width, height])
            {
                return Err(invalid());
            }
            pages[i] = Some(Page {
                first,
                count: group.len(),
                digest: if verify {
                    Some(
                        page.rgba8
                            .as_ref()
                            .ok_or_else(|| {
                                io::Error::other(
                                    "reference verification needs an RGBA8 page digest",
                                )
                            })?
                            .sha256
                            .clone(),
                    )
                } else {
                    None
                },
            });
            first += group.len();
        }
        let path = |var| {
            std::env::var_os(var)
                .ok_or_else(|| io::Error::other(format!("reference needs {var}")))
                .and_then(|path| PathBuf::from(path).canonicalize())
        };
        Ok(Self {
            exe: path("DESKTOP_PET_REFERENCE_FFMPEG")?,
            video: path("DESKTOP_PET_REFERENCE_VIDEO")?,
            size: action.logical_size,
            pages,
            frame_count: first,
            next: 0,
            stdout: None,
            process: DecoderProcess::default(),
        })
    }

    pub fn process(&self) -> DecoderProcess {
        self.process.clone()
    }

    pub fn load_page(
        &mut self,
        index: usize,
        generation: u64,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<PagePixels> {
        let check = || {
            if cancelled() {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "reference cancelled",
                ))
            } else {
                Ok(())
            }
        };
        let [width, height] = self.size;
        if self
            .stdout
            .as_ref()
            .is_none_or(|(current, _)| *current != generation)
        {
            let mut process = self.process.0.lock().unwrap();
            check()?;
            let filter = format!("scale={width}:{height}:flags=bicubic,format=rgba");
            let mut child = Command::new(&self.exe)
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-threads",
                    "2",
                    "-stream_loop",
                    "-1",
                    "-c:v",
                    "libvpx-vp9",
                    "-i",
                ])
                .arg(&self.video)
                .args([
                    "-map",
                    "0:v:0",
                    "-an",
                    "-sn",
                    "-dn",
                    "-filter_threads",
                    "1",
                    "-vf",
                    &filter,
                    "-fps_mode",
                    "passthrough",
                    "-f",
                    "rawvideo",
                    "-threads",
                    "1",
                    "pipe:1",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()?;
            self.stdout = Some((generation, child.stdout.take().unwrap()));
            *process = Some(child);
            self.next = 0;
        }
        let page = self.pages[index]
            .as_ref()
            .ok_or_else(|| io::Error::other("unknown reference page"))?;
        let stdout = &mut self.stdout.as_mut().unwrap().1;
        let stride = width as usize * 4;
        let mut frame = vec![0; stride * height as usize];
        while self.next != page.first {
            check()?;
            stdout.read_exact(&mut frame)?;
            self.next = (self.next + 1) % self.frame_count;
        }
        let mut rgba = vec![0; frame.len() * page.count];
        for slot in 0..page.count {
            check()?;
            stdout.read_exact(&mut frame)?;
            for (row, pixels) in frame.chunks_exact(stride).enumerate() {
                let offset = (row * page.count + slot) * stride;
                rgba[offset..offset + stride].copy_from_slice(pixels);
            }
        }
        self.next = (self.next + page.count) % self.frame_count;
        if let Some(digest) = &page.digest
            && format!("{:x}", Sha256::digest(&rgba)) != *digest
        {
            return Err(io::Error::other(
                "FFmpeg reference differs from the pack's RGBA page",
            ));
        }
        check()?;
        Ok(PagePixels {
            size: [width * page.count as u32, height],
            rgba,
            bc7: None,
            variant: Variant::Ffmpeg,
        })
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.process.stop();
    }
}
