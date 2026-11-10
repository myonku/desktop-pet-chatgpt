pub mod media;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
pub use app::run;
