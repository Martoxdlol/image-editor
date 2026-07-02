//! ed-tools — tool state machines and the editor session (spec §6.8,
//! §12.1). Tools receive input events and emit transactional ops +
//! overlay draw commands + cursor; the session hosts the multi-document
//! registry, shared blob store, and internal clipboard.

pub mod area;
pub mod fragment;
pub mod input;
pub mod session;
pub mod tools;

pub use input::{InputEvent, Modifiers, PointerKind};
pub use session::{DocState, Session};
pub use tools::ToolKind;
