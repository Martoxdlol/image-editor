//! Input protocol: the UI forwards raw pointer/key events; all
//! interpretation happens core-side (spec §12.3 golden rule).

use serde::Deserialize;

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum PointerKind {
    Down,
    Move,
    Up,
    DoubleClick,
}

#[derive(Clone, Copy, Default, Deserialize, Debug)]
pub struct Modifiers {
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub meta: bool,
}

/// Pointer event in *screen* (canvas) coordinates; the session converts
/// through the active view transform.
#[derive(Clone, Copy, Deserialize, Debug)]
pub struct InputEvent {
    pub kind: PointerKind,
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_pressure")]
    pub pressure: f64,
    #[serde(default)]
    pub button: u8,
    #[serde(default)]
    pub mods: Modifiers,
}

fn default_pressure() -> f64 {
    1.0
}
