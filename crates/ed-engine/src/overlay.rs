//! Overlay draw commands (spec §6.8): tools emit these; the engine draws
//! them in screen space on top of the composited frame.

use ed_core::{Rect, Vec2};

#[derive(Clone, Debug)]
pub enum Overlay {
    /// Dashed/solid rectangle outline, doc coords.
    RectOutline { rect: Rect, dashed: bool },
    EllipseOutline { rect: Rect, dashed: bool },
    PolyOutline { points: Vec<Vec2>, close: bool, dashed: bool },
    /// Selection bbox + 8 scale handles + rotation hint, doc coords.
    Handles { rect: Rect, rotation: f64 },
    /// Round brush cursor, doc coords + doc-space diameter.
    BrushCursor { pos: Vec2, size: f64 },
    Line { from: Vec2, to: Vec2, dashed: bool },
    /// Pen tool preview: SVG path data in doc coords.
    PathPreview { d: String },
    /// Anchor points (pen tool node editing).
    Anchors { points: Vec<Vec2>, active: Option<usize> },
    /// Crosshair (eyedropper etc.), doc coords.
    Crosshair { pos: Vec2 },
}
