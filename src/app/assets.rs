use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use winit::{
    event_loop::EventLoopProxy,
    platform::windows::WindowExtWindows,
    window::{Icon, Window},
};

use crate::{
    media::{
        LoadedResource, Pack, PackIcon, Variant,
        playback::{PageResult, Playback},
    },
    platform::windows::{apply_embedded_window_icons, local_path},
    render::Renderer,
};

use super::AppEvent;

pub(super) fn load_startup_pack(
    configured: Option<&Path>,
    default: PathBuf,
    variant: Variant,
    proxy: &EventLoopProxy<AppEvent>,
    renderer: &mut Renderer,
) -> io::Result<(PathBuf, LoadedPack, String)> {
    let mut open = |root: &Path, variant| -> io::Result<LoadedPack> {
        let frames = proxy.clone();
        let pack = LoadedPack::open(root, variant, move |page| {
            frames.send_event(AppEvent::PageReady(page)).is_ok()
        })?;
        let (key, frame) = pack.playback.frame().unwrap();
        renderer.show(Some(key), frame).map_err(io::Error::other)?;
        Ok(pack)
    };
    let mut configured_failure = String::new();
    if let Some(path) = configured {
        match open(path, variant) {
            Ok(pack) => return Ok((path.to_owned(), pack, "当前素材包已加载。".into())),
            Err(error) => {
                configured_failure = format!("指定素材包 {} 加载失败：{error}\n", path.display());
                eprint!("{configured_failure}");
            }
        }
    }
    #[cfg(feature = "ffmpeg-decoder")]
    let variant = if variant == Variant::Ffmpeg {
        Variant::Png
    } else {
        variant
    };
    let pack = open(&default, variant).map_err(|error| {
        io::Error::other(format!(
            "{configured_failure}默认素材包 {} 加载失败：{error}\n没有可用素材包，启动已停止。",
            default.display()
        ))
    })?;
    let status = if configured_failure.is_empty() {
        "当前素材包已加载。"
    } else {
        "指定素材包无法加载，已加载默认素材包。"
    };
    Ok((default, pack, status.into()))
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
    pub poster: Option<egui::ColorImage>,
    pub window_icon: Option<Icon>,
}

impl CharacterAssets {
    pub fn apply_window_icon(&self, window: &Window) {
        if let Some(icon) = &self.window_icon {
            window.set_window_icon(Some(icon.clone()));
            window.set_taskbar_icon(Some(icon.clone()));
        } else {
            apply_embedded_window_icons(window);
        }
    }
}

impl LoadedPack {
    pub fn open(
        root: &Path,
        variant: Variant,
        deliver: impl Fn(PageResult) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let path = local_path(root)?;
        let path = local_path(&path.canonicalize()?)?;
        let pack = Arc::new(Pack::open(&path)?);
        let mut playback = Playback::open(pack.clone(), variant, deliver)?;
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
            Some(LoadedResource::Image(image)) => Some(egui::ColorImage::from_rgba_unmultiplied(
                image.size.map(|value| value as usize),
                &image.rgba,
            )),
            _ => None,
        };
        let click_wave = match resource("interaction.click") {
            Some(LoadedResource::Audio(wave)) => Some(wave),
            _ => None,
        };
        let window_icon = match resource("app.icon") {
            Some(LoadedResource::Icon(icon)) => icon
                .rgba_for_size(64)
                .ok()
                .and_then(|(side, rgba)| Icon::from_rgba(rgba, side, side).ok()),
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
            },
            click_wave,
            tray_icon,
        })
    }

    pub fn trace_resources(&self) {
        use sha2::{Digest, Sha256};

        let poster = self.assets.poster.as_ref().map(|image| {
            let mut hash = Sha256::new();
            for pixel in &image.pixels {
                hash.update(pixel.to_array());
            }
            format!("{:x}", hash.finalize())
        });
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
