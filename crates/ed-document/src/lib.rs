//! ed-document — tree, ops, undo, expressions, serialization (spec §12.2).
//! Zero render deps; this is what a future sync server compiles natively.

pub mod doc;
pub mod expr;
pub mod frac_index;
pub mod mirror;
pub mod node;
pub mod ops;
pub mod selection;
pub mod serialize;

pub use doc::{BlobStore, Document, PaletteEntry, MAX_ARTBOARD_DIM};
pub use node::{BitmapData, Modifier, Node, NodeKind, Stroke, StrokePoint, TILE_SIZE};
pub use ops::{Op, OpKind, TilePatch, Txn};
pub use selection::{CombineMode, PixelSelection, SelGeom, SelShape};
