use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use winit::{event_loop::EventLoopProxy, window::Window};

use crate::{
    media::{
        LoadedResource, Pack, PackIcon, PagePixels,
        playback::{PageResult, Playback},
    },
    platform::windows::{WindowIcons, local_path},
    render::Renderer,
};

use super::AppEvent;

pub(super) fn open_local_pack(root: &Path) -> io::Result<Pack> {
    let path = local_path(root)?;
    let path = local_path(&path.canonicalize()?)?;
    Pack::open(&path)
}

pub(super) fn load_startup_pack(
    configured: Option<&Path>,
    default: PathBuf,
    proxy: &EventLoopProxy<AppEvent>,
    renderer: &mut Renderer,
) -> io::Result<(PathBuf, LoadedPack, Option<String>)> {
    let mut configured_failure = String::new();
    if let Some(path) = configured {
        match load_ready_pack(path, proxy, Some(renderer)) {
            Ok(pack) => return Ok((path.to_owned(), pack, None)),
            Err(error) => {
                configured_failure = format!("指定素材包 {} 加载失败：{error}\n", path.display());
                eprint!("{configured_failure}");
            }
        }
    }
    let pack = load_ready_pack(&default, proxy, Some(renderer)).map_err(|error| {
        io::Error::other(format!(
            "{configured_failure}默认素材包 {} 加载失败：{error}\n没有可用素材包，启动已停止。",
            default.display()
        ))
    })?;
    let status =
        (!configured_failure.is_empty()).then(|| "指定素材包无法加载，已加载默认素材包。".into());
    Ok((default, pack, status))
}

pub(super) fn load_ready_pack(
    root: &Path,
    proxy: &EventLoopProxy<AppEvent>,
    renderer: Option<&mut Renderer>,
) -> io::Result<LoadedPack> {
    let frames = proxy.clone();
    let pack = LoadedPack::open(root, move |page| {
        frames.send_event(AppEvent::PageReady(page)).is_ok()
    })?;
    if let Some(renderer) = renderer {
        let (key, frame) = pack.playback.frame().unwrap();
        renderer.show(key, frame).map_err(io::Error::other)?;
    }
    Ok(pack)
}

pub(crate) struct LoadedPack {
    pub playback: Playback,
    pub assets: CharacterAssets,
    pub click_wave: Option<Vec<u8>>,
    pub tray_icon: Option<PackIcon>,
}

#[derive(Default)]
pub(crate) struct CharacterAssets {
    pub name: Option<String>,
    pub poster: Option<PagePixels>,
    window_icon: Option<PackIcon>,
    window_icons: Option<WindowIcons>,
}

impl CharacterAssets {
    pub fn apply_window_icon(&mut self, window: &Window) {
        self.window_icons = WindowIcons::install(window, self.window_icon.take().as_ref());
    }

    pub fn settings_poster(&self) -> io::Result<Option<Vec<u8>>> {
        let Some(image) = &self.poster else {
            return Ok(None);
        };
        let [width, height] = image.size;
        let pixels = &image.rgba;
        let long_side = width.max(height);
        let [out_width, out_height] =
            [width, height].map(|side| (side * 128 / long_side).max(1).min(side));
        let mut thumbnail = Vec::with_capacity((out_width * out_height * 4) as usize);
        for y in 0..out_height {
            for x in 0..out_width {
                let start =
                    ((y * height / out_height * width + x * width / out_width) * 4) as usize;
                thumbnail.extend_from_slice(&pixels[start..start + 4]);
            }
        }
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, out_width, out_height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&thumbnail)?;
        Ok(Some(bytes))
    }
}

impl LoadedPack {
    pub fn open(
        root: &Path,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let pack = Arc::new(open_local_pack(root)?);
        let mut playback = Playback::open(pack.clone(), deliver)?;
        playback.load_first_frame()?;
        let resource = |key| match pack.load_resource(key, None) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("Optional asset {key} unavailable: {error}");
                None
            }
        };
        let name = match resource("character.name") {
            Some(LoadedResource::Text(text)) if !text.trim().is_empty() => {
                Some(text.trim().chars().take(120).collect())
            }
            _ => None,
        };
        let poster = match resource("character.poster") {
            Some(LoadedResource::Image(image)) => Some(image),
            _ => None,
        };
        let click_wave = match resource("interaction.click") {
            Some(LoadedResource::Audio(wave)) => Some(wave),
            _ => None,
        };
        let window_icon = match resource("app.icon") {
            Some(LoadedResource::Icon(icon)) => Some(icon),
            _ => None,
        };
        let tray_icon = match resource("tray.icon") {
            Some(LoadedResource::Icon(icon)) => Some(icon),
            _ => None,
        };
        Ok(Self {
            playback,
            assets: CharacterAssets {
                name,
                poster,
                window_icon,
                window_icons: None,
            },
            click_wave,
            tray_icon,
        })
    }

    pub fn trace_resources(&self) {
        use sha2::{Digest, Sha256};

        let poster = self
            .assets
            .poster
            .as_ref()
            .map(|image| format!("{:x}", Sha256::digest(&image.rgba)));
        let name = self
            .assets
            .name
            .as_ref()
            .map(|name| format!("{:x}", Sha256::digest(name.as_bytes())));
        let wave = self
            .click_wave
            .as_ref()
            .map(|wave| format!("{:x}", Sha256::digest(wave)));
        eprintln!("Asset resources: {poster:?} {wave:?} {name:?}");
    }
}
