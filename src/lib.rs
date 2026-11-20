pub mod core;
pub mod media;
pub mod storage;

#[cfg(windows)]
pub mod integrations;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod render;

#[cfg(windows)]
pub use app::run;
