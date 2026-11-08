//! CutOut core engine — platform-independent (macOS today, Windows later).
pub mod adjust;
pub mod brush;
pub mod color;
pub mod composite;
pub mod document;
pub mod engine;
pub mod filters;
pub mod geometry;
pub mod history;
pub mod image;
pub mod project;
pub mod selection;
#[cfg(feature = "ai")]
pub mod ai;
