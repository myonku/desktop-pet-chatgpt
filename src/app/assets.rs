use std::{io, path::Path, sync::Arc};

use crate::{
    media::{
        LoadedResource, Pack, Variant,
        playback::{PageResult, Playback},
    },
    platform::windows::local_path,
};

pub(crate) struct LoadedPack {
    pub playback: Playback,
    pub assets: CharacterAssets,
    pub click_wave: Option<Vec<u8>>,
}

#[derive(Default)]
pub(crate) struct CharacterAssets {
    pub name: Option<String>,
    pub poster: Option<egui::ColorImage>,
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
        let playback = Playback::open(pack.clone(), variant, deliver)?;
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
        Ok(Self {
            playback,
            assets: CharacterAssets { name, poster },
            click_wave,
        })
    }
}
