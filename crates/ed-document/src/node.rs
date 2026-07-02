//! Node tree model (spec §2.2, §15).
//!
//! Every node carries a generic, typed param map (spec §3.4: params are
//! schema-driven so panels auto-render); heavy payloads (bitmap tiles,
//! freehand strokes) live in dedicated fields, referenced from ops by
//! content-addressed blobs.

use ed_core::{BlendMode, Color, NodeId, Value, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    /// Artboard: top-level container on the pasteboard (spec §2.1).
    /// Params: `x y w h dpi background bg-color`.
    Artboard,
    Group,
    /// Layer = group with blend semantics surfaced in UI (spec §2.2).
    Layer,
    /// Parametric primitive (spec §6.2). Params: `shape` (rect|ellipse|
    /// polygon|star|line|arrow) + geometry + fill/stroke params.
    Shape,
    /// Bézier path. Param `d` holds SVG path data; fill/stroke params.
    Path,
    /// Rich text block (scoped §7). Params: `text font-size font-family
    /// align bold italic fill-color x y w h auto-size`.
    Text,
    /// Raster data, tile-based sparse storage (spec §2.5).
    Bitmap,
    /// Freehand strokes stored as re-editable stroke data.
    StrokeSet,
    /// Parametric gradient region (spec §2.2).
    GradientFill,
    /// Instance of a component (spec §6.7). Param `component` = node id.
    Reference,
}

impl NodeKind {
    pub fn is_container(self) -> bool {
        matches!(self, NodeKind::Artboard | NodeKind::Group | NodeKind::Layer)
    }
}

/// Modifier on a node's ordered stack (spec §2.2). Generic kind + params so
/// the op schema (`ModifierAttach`/`ParamSet`) is uniform across all kinds.
/// Kinds: `transform`, `mask`, `clip`, plus `filter.*` and `adjust.*`.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Modifier {
    pub id: u64,
    pub kind: String,
    pub enabled: bool,
    pub params: BTreeMap<String, Value>,
}

/// A 256×256 RGBA8 tile (spec §4.4, RGBA8 pragmatic stand-in for f16).
pub const TILE_SIZE: u32 = 256;

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
pub struct BitmapData {
    pub width: u32,
    pub height: u32,
    /// Sparse tiles keyed by tile coords; absent tiles are transparent
    /// (spec §4.4 `Uniform`/`Absent` residency states).
    #[serde(skip)]
    pub tiles: BTreeMap<(u32, u32), Vec<u8>>,
    /// Content stamp for render caches (spec §4.1 per-node caches keyed by
    /// content): bumped on every pixel mutation.
    #[serde(skip)]
    pub rev: u64,
}

impl BitmapData {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, tiles: BTreeMap::new(), rev: 0 }
    }

    pub fn tiles_across(&self) -> u32 {
        self.width.div_ceil(TILE_SIZE)
    }

    pub fn tiles_down(&self) -> u32 {
        self.height.div_ceil(TILE_SIZE)
    }

    pub fn get_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let key = (x / TILE_SIZE, y / TILE_SIZE);
        match self.tiles.get(&key) {
            None => [0; 4],
            Some(t) => {
                let i = (((y % TILE_SIZE) * TILE_SIZE + (x % TILE_SIZE)) * 4) as usize;
                [t[i], t[i + 1], t[i + 2], t[i + 3]]
            }
        }
    }

    pub fn tile_mut(&mut self, tx: u32, ty: u32) -> &mut Vec<u8> {
        self.rev += 1;
        self.tiles
            .entry((tx, ty))
            .or_insert_with(|| vec![0u8; (TILE_SIZE * TILE_SIZE * 4) as usize])
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let t = self.tile_mut(x / TILE_SIZE, y / TILE_SIZE);
        let i = (((y % TILE_SIZE) * TILE_SIZE + (x % TILE_SIZE)) * 4) as usize;
        t[i..i + 4].copy_from_slice(&rgba);
    }

    /// Fill from a contiguous RGBA8 buffer (import path).
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Self {
        let mut bm = Self::new(width, height);
        for ty in 0..bm.tiles_down() {
            for tx in 0..bm.tiles_across() {
                let tile = bm.tile_mut(tx, ty);
                let mut any = false;
                for row in 0..TILE_SIZE {
                    let y = ty * TILE_SIZE + row;
                    if y >= height {
                        break;
                    }
                    let cols = TILE_SIZE.min(width - tx * TILE_SIZE);
                    let src = ((y * width + tx * TILE_SIZE) * 4) as usize;
                    let dst = (row * TILE_SIZE * 4) as usize;
                    let n = (cols * 4) as usize;
                    tile[dst..dst + n].copy_from_slice(&rgba[src..src + n]);
                    if !any {
                        any = tile[dst..dst + n].iter().any(|&b| b != 0);
                    }
                }
                if !any {
                    bm.tiles.remove(&(tx, ty));
                }
            }
        }
        bm
    }

    /// Flatten to a contiguous RGBA8 buffer (export path).
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = vec![0u8; (self.width * self.height * 4) as usize];
        for (&(tx, ty), tile) in &self.tiles {
            for row in 0..TILE_SIZE {
                let y = ty * TILE_SIZE + row;
                if y >= self.height {
                    break;
                }
                let x0 = tx * TILE_SIZE;
                if x0 >= self.width {
                    break;
                }
                let cols = TILE_SIZE.min(self.width - x0);
                let dst = ((y * self.width + x0) * 4) as usize;
                let src = (row * TILE_SIZE * 4) as usize;
                out[dst..dst + (cols * 4) as usize]
                    .copy_from_slice(&tile[src..src + (cols * 4) as usize]);
            }
        }
        out
    }
}

/// One freehand stroke — intent, not pixels (spec §3.2 `PaintStroke`).
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Stroke {
    pub color: Color,
    pub size: f64,
    pub hardness: f64,
    pub opacity: f64,
    pub erase: bool,
    /// Document-space points with pressure.
    pub points: Vec<StrokePoint>,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug)]
pub struct StrokePoint {
    pub pos: Vec2,
    pub pressure: f64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    /// `None` = top level (artboard or shared-space pasteboard node).
    pub parent: Option<NodeId>,
    /// Fractional index among siblings (spec §3.1).
    pub frac: String,
    pub params: BTreeMap<String, Value>,
    pub modifiers: Vec<Modifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitmap: Option<BitmapData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<Stroke>,
}

impl Node {
    pub fn new(id: NodeId, kind: NodeKind) -> Self {
        let mut params = BTreeMap::new();
        params.insert("name".into(), Value::Str(default_name(kind).into()));
        params.insert("visible".into(), Value::Bool(true));
        params.insert("locked".into(), Value::Bool(false));
        params.insert("opacity".into(), Value::F64(1.0));
        params.insert("blend".into(), Value::Str("normal".into()));
        Node {
            id,
            kind,
            parent: None,
            frac: String::new(),
            params,
            modifiers: Vec::new(),
            bitmap: if kind == NodeKind::Bitmap { Some(BitmapData::default()) } else { None },
            strokes: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        self.params.get("name").and_then(|v| v.as_str()).unwrap_or("node")
    }

    pub fn visible(&self) -> bool {
        self.params.get("visible").and_then(|v| v.as_bool()).unwrap_or(true)
    }

    pub fn locked(&self) -> bool {
        self.params.get("locked").and_then(|v| v.as_bool()).unwrap_or(false)
    }

    pub fn opacity(&self) -> f64 {
        self.params.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0).clamp(0.0, 1.0)
    }

    pub fn blend(&self) -> BlendMode {
        self.params
            .get("blend")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_value(serde_json::Value::String(s.into())).ok())
            .unwrap_or_default()
    }

    /// Read a param addressed by path: `"x"`, `"mod.<id>.<key>"`.
    pub fn get_param(&self, path: &str) -> Option<Value> {
        if let Some(rest) = path.strip_prefix("mod.") {
            let (id, key) = rest.split_once('.')?;
            let id: u64 = id.parse().ok()?;
            let m = self.modifiers.iter().find(|m| m.id == id)?;
            if key == "enabled" {
                return Some(Value::Bool(m.enabled));
            }
            return m.params.get(key).cloned();
        }
        self.params.get(path).cloned()
    }

    /// Write a param addressed by path; returns the previous value.
    pub fn set_param(&mut self, path: &str, value: Value) -> Option<Value> {
        if let Some(rest) = path.strip_prefix("mod.") {
            let (id, key) = rest.split_once('.')?;
            let id: u64 = id.parse().ok()?;
            let m = self.modifiers.iter_mut().find(|m| m.id == id)?;
            if key == "enabled" {
                let prev = Value::Bool(m.enabled);
                m.enabled = value.as_bool().unwrap_or(true);
                return Some(prev);
            }
            return m.params.insert(key.to_string(), value);
        }
        self.params.insert(path.to_string(), value)
    }
}

fn default_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Artboard => "Artboard",
        NodeKind::Group => "Group",
        NodeKind::Layer => "Layer",
        NodeKind::Shape => "Shape",
        NodeKind::Path => "Path",
        NodeKind::Text => "Text",
        NodeKind::Bitmap => "Bitmap",
        NodeKind::StrokeSet => "Strokes",
        NodeKind::GradientFill => "Gradient",
        NodeKind::Reference => "Instance",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_tiles_sparse_roundtrip() {
        let mut bm = BitmapData::new(600, 300);
        assert_eq!(bm.tiles_across(), 3);
        assert_eq!(bm.tiles_down(), 2);
        bm.set_pixel(0, 0, [1, 2, 3, 4]);
        bm.set_pixel(599, 299, [9, 8, 7, 6]);
        assert_eq!(bm.tiles.len(), 2); // sparse: only touched tiles allocated
        assert_eq!(bm.get_pixel(0, 0), [1, 2, 3, 4]);
        assert_eq!(bm.get_pixel(599, 299), [9, 8, 7, 6]);
        assert_eq!(bm.get_pixel(300, 150), [0, 0, 0, 0]);

        let flat = bm.to_rgba();
        let back = BitmapData::from_rgba(600, 300, &flat);
        assert_eq!(back.get_pixel(0, 0), [1, 2, 3, 4]);
        assert_eq!(back.get_pixel(599, 299), [9, 8, 7, 6]);
    }

    #[test]
    fn param_paths() {
        let mut n = Node::new(NodeId::new(1, 1), NodeKind::Shape);
        assert_eq!(n.name(), "Shape");
        n.set_param("x", Value::F64(5.0));
        assert_eq!(n.get_param("x").unwrap().as_f64(), Some(5.0));

        n.modifiers.push(Modifier {
            id: 3,
            kind: "filter.gaussian-blur".into(),
            enabled: true,
            params: BTreeMap::from([("radius".into(), Value::F64(4.0))]),
        });
        assert_eq!(n.get_param("mod.3.radius").unwrap().as_f64(), Some(4.0));
        let prev = n.set_param("mod.3.radius", Value::F64(8.0)).unwrap();
        assert_eq!(prev.as_f64(), Some(4.0));
        assert_eq!(n.get_param("mod.3.enabled").unwrap().as_bool(), Some(true));
    }
}
