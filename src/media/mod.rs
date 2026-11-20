mod icon;
mod manifest;
mod pack;
pub use icon::PackIcon;

pub mod frame;
pub mod playback;

pub use manifest::{
    Clip, Direction, Frame, Manifest, Page, PagePayload, Payload, Resource, ResourceContent,
};
pub use pack::{LoadedResource, PLAYBACK_PAGE_LIMIT, Pack};

pub struct PagePixels {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}
