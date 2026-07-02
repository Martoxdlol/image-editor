//! ed-core — ids, geometry, color, params, errors (spec §12.2).
//! Zero dependencies beyond serde; every other crate builds on this.

pub mod blend;
pub mod color;
pub mod geometry;
pub mod ids;
pub mod value;

pub use blend::BlendMode;
pub use color::Color;
pub use geometry::{Mat3, Rect, Vec2};
pub use ids::{ActorId, BlobHash, NodeId, OpId, TxnId};
pub use value::Value;
