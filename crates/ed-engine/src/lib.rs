//! ed-engine — CPU compositor over tiny-skia (spec §4: the deterministic
//! CPU path; wgpu backend is a deferred presenter behind the same API).

pub mod filters;
pub mod hit;
pub mod overlay;
pub mod paint;
pub mod raster;
pub mod render;
pub mod shapes;
pub mod text;

pub use overlay::Overlay;
pub use render::{Engine, View};

pub use tiny_skia::Pixmap;
