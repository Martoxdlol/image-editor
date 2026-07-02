//! Tool state machines (spec §6.8): `handle(InputEvent) → ops + overlays
//! + cursor`. Drag previews mutate transient state; pointer-up commits a
//! single transaction (spec §3.3).

use crate::input::{InputEvent, Modifiers, PointerKind};
use crate::session::Session;
use ed_core::{NodeId, Rect, Value, Vec2};
use ed_document::{CombineMode, NodeKind, OpKind, PixelSelection, SelGeom, Stroke, StrokePoint};
use ed_engine::raster::{self, BrushParams, SelMask};
use ed_engine::Overlay;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    Select,
    Rect,
    Ellipse,
    Polygon,
    Star,
    Line,
    Arrow,
    Pen,
    Text,
    Brush,
    Pencil,
    Eraser,
    Fill,
    Eyedropper,
    Gradient,
    SelRect,
    SelEllipse,
    Lasso,
    Wand,
    Pan,
    Zoom,
}

pub enum DragState {
    MoveNodes {
        start: Vec2,
        /// (id, param-key, original value) — restored before the commit txn.
        origs: Vec<(NodeId, String, f64)>,
        orig_strokes: Vec<(NodeId, Vec<Stroke>)>,
        orig_paths: Vec<(NodeId, String)>,
        moved: bool,
    },
    ResizeNodes {
        handle: usize,
        start_bounds: Rect,
        origs: Vec<(NodeId, Rect)>,
    },
    Marquee {
        start: Vec2,
        cur: Vec2,
    },
    DrawShape {
        shape: &'static str,
        start: Vec2,
        cur: Vec2,
        constrain: bool,
    },
    Paint {
        node: NodeId,
        offset: Vec2,
        before: BTreeMap<(u32, u32), Option<Vec<u8>>>,
        last: Vec2,
        carry: f64,
        label: &'static str,
        mask: Option<SelMask>,
    },
    PaintStrokes {
        node: NodeId,
        prev: Vec<Stroke>,
        points: Vec<StrokePoint>,
    },
    SelectShape {
        kind: &'static str,
        start: Vec2,
        cur: Vec2,
        mode: CombineMode,
    },
    Lasso {
        points: Vec<Vec2>,
        mode: CombineMode,
    },
    Pan {
        start_pan: Vec2,
        start_screen: Vec2,
    },
    GradientDrag {
        start: Vec2,
        cur: Vec2,
    },
}

/// Pen tool accumulates across clicks (not a drag) — lives in Session via
/// this state object.
#[derive(Default)]
pub struct PenState {
    pub points: Vec<Vec2>,
}

fn combine_mode(mods: Modifiers) -> CombineMode {
    match (mods.shift, mods.alt) {
        (true, true) => CombineMode::Intersect,
        (true, false) => CombineMode::Add,
        (false, true) => CombineMode::Subtract,
        (false, false) => CombineMode::Replace,
    }
}

impl Session {
    pub fn cancel_drag(&mut self) {
        if self.drag.is_some() {
            // revert previews
            if let Some(DragState::MoveNodes { origs, orig_strokes, orig_paths, .. }) = self.drag.take() {
                for (id, key, v) in origs {
                    self.doc_mut().preview_param(id, &key, Value::F64(v));
                }
                for (id, strokes) in orig_strokes {
                    if let Some(s) = self.doc_mut().strokes_mut(id) {
                        *s = strokes;
                    }
                }
                for (id, d) in orig_paths {
                    self.doc_mut().preview_param(id, "d", Value::Str(d));
                }
            }
            self.drag = None;
        }
        self.overlays.clear();
        self.pen.points.clear();
        self.dirty_frame();
    }

    pub fn pointer(&mut self, ev: InputEvent) {
        let p = self.view().screen_to_doc(Vec2::new(ev.x, ev.y));
        self.pointer_doc = p;
        // brush cursor overlay for paint tools
        match ev.kind {
            PointerKind::Down => self.pointer_down(ev, p),
            PointerKind::Move => self.pointer_move(ev, p),
            PointerKind::Up => self.pointer_up(ev, p),
            PointerKind::DoubleClick => self.double_click(ev, p),
        }
        self.refresh_passive_overlays(p);
    }

    fn refresh_passive_overlays(&mut self, p: Vec2) {
        // keep brush cursor visible while hovering with paint tools
        if self.drag.is_none() {
            self.overlays.clear();
            if let Some(size) = self.brush_cursor_size() {
                self.overlays.push(Overlay::BrushCursor { pos: p, size });
            }
            if !self.pen.points.is_empty() {
                let mut pts = self.pen.points.clone();
                pts.push(p);
                self.overlays.push(Overlay::PolyOutline { points: pts.clone(), close: false, dashed: false });
                self.overlays.push(Overlay::Anchors { points: self.pen.points.clone(), active: None });
            }
            if self.tool == ToolKind::Select && !self.doc().selected_nodes.is_empty() {
                let b = self.selection_bounds_pub();
                if b.w > 0.0 || b.h > 0.0 {
                    self.overlays.push(Overlay::Handles { rect: b, rotation: 0.0 });
                }
            }
        }
        self.dirty_frame();
    }

    pub fn selection_bounds_pub(&self) -> Rect {
        let mut acc = Rect::default();
        for &id in &self.doc().selected_nodes {
            if let Some(b) = self.engine.node_bounds(self.doc(), id) {
                acc = acc.union(&b);
            }
        }
        acc
    }

    fn brush_cursor_size(&self) -> Option<f64> {
        match self.tool {
            ToolKind::Brush => Some(self.tool_f64("brush.size", 16.0)),
            ToolKind::Pencil => Some(self.tool_f64("pencil.size", 1.0)),
            ToolKind::Eraser => Some(self.tool_f64("eraser.size", 24.0)),
            _ => None,
        }
    }

    // ------------------------------------------------------------ down

    fn pointer_down(&mut self, ev: InputEvent, p: Vec2) {
        match self.tool {
            ToolKind::Select => self.select_down(ev, p),
            ToolKind::Rect => self.shape_down("rect", ev, p),
            ToolKind::Ellipse => self.shape_down("ellipse", ev, p),
            ToolKind::Polygon => self.shape_down("polygon", ev, p),
            ToolKind::Star => self.shape_down("star", ev, p),
            ToolKind::Line => self.shape_down("line", ev, p),
            ToolKind::Arrow => self.shape_down("arrow", ev, p),
            ToolKind::Pen => self.pen_click(ev, p),
            ToolKind::Text => self.text_click(p),
            ToolKind::Brush => self.paint_down(ev, p, false),
            ToolKind::Pencil => self.paint_down(ev, p, true),
            ToolKind::Eraser => self.erase_down(ev, p),
            ToolKind::Fill => self.fill_click(ev, p),
            ToolKind::Eyedropper => self.eyedrop(ev, p),
            ToolKind::Gradient => {
                self.drag = Some(DragState::GradientDrag { start: p, cur: p });
            }
            ToolKind::SelRect => {
                self.drag = Some(DragState::SelectShape {
                    kind: "rect",
                    start: p,
                    cur: p,
                    mode: combine_mode(ev.mods),
                });
            }
            ToolKind::SelEllipse => {
                self.drag = Some(DragState::SelectShape {
                    kind: "ellipse",
                    start: p,
                    cur: p,
                    mode: combine_mode(ev.mods),
                });
            }
            ToolKind::Lasso => {
                self.drag = Some(DragState::Lasso { points: vec![p], mode: combine_mode(ev.mods) });
            }
            ToolKind::Wand => self.wand_click(ev, p),
            ToolKind::Pan => {
                self.drag = Some(DragState::Pan {
                    start_pan: self.view().pan,
                    start_screen: Vec2::new(ev.x, ev.y),
                });
            }
            ToolKind::Zoom => {
                let factor = if ev.mods.alt { 1.0 / 1.5 } else { 1.5 };
                let v = self.view_mut();
                let anchor = Vec2::new(p.x, p.y);
                v.zoom = (v.zoom * factor).clamp(0.01, 64.0);
                v.pan = Vec2::new(anchor.x - ev.x / v.zoom, anchor.y - ev.y / v.zoom);
            }
        }
    }

    fn select_down(&mut self, ev: InputEvent, p: Vec2) {
        // handle hit first (resize)
        let sel_bounds = self.selection_bounds_pub();
        if !self.doc().selected_nodes.is_empty() && (sel_bounds.w > 0.0 || sel_bounds.h > 0.0) {
            if let Some(handle) = self.hit_handle(&sel_bounds, ev) {
                let origs = self
                    .doc()
                    .selected_nodes
                    .iter()
                    .filter_map(|&id| {
                        let n = self.doc().node(id)?;
                        if n.get_param("w").is_some() || n.kind == NodeKind::Shape {
                            Some((
                                id,
                                Rect::new(
                                    self.doc().param_f64(n, "x", 0.0),
                                    self.doc().param_f64(n, "y", 0.0),
                                    self.doc().param_f64(n, "w", 0.0),
                                    self.doc().param_f64(n, "h", 0.0),
                                ),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                if !origs.is_empty() {
                    self.drag = Some(DragState::ResizeNodes { handle, start_bounds: sel_bounds, origs });
                    return;
                }
            }
        }

        let hit = self.engine.hit_test(self.doc(), p, ev.mods.meta || ev.mods.ctrl);
        match hit {
            Some(id) => {
                let already = self.doc().selected_nodes.contains(&id);
                if ev.mods.shift {
                    let doc = self.doc_mut();
                    if already {
                        doc.selected_nodes.retain(|&x| x != id);
                    } else {
                        doc.selected_nodes.push(id);
                    }
                } else if !already {
                    self.doc_mut().selected_nodes = vec![id];
                }
                // start move drag with captured originals
                let ids = self.doc().selected_nodes.clone();
                let mut origs = Vec::new();
                let mut orig_strokes = Vec::new();
                let mut orig_paths = Vec::new();
                for id in ids {
                    // collect movable leaves in the subtree (groups move kids)
                    let mut subtree = Vec::new();
                    self.doc().walk(id, &mut subtree);
                    for nid in subtree {
                        let Some(n) = self.doc().node(nid) else { continue };
                        match n.kind {
                            NodeKind::StrokeSet => {
                                orig_strokes.push((nid, n.strokes.clone()));
                            }
                            NodeKind::Path => {
                                orig_paths.push((nid, self.doc().param_str(n, "d", "")));
                            }
                            NodeKind::Group | NodeKind::Layer | NodeKind::Artboard => {}
                            _ => {
                                for key in ["x", "y", "x2", "y2"] {
                                    if let Some(v) =
                                        n.get_param(key).and_then(|v| self.doc().resolve(&v).as_f64())
                                    {
                                        origs.push((nid, key.to_string(), v));
                                    }
                                }
                            }
                        }
                    }
                }
                self.drag = Some(DragState::MoveNodes {
                    start: p,
                    origs,
                    orig_strokes,
                    orig_paths,
                    moved: false,
                });
            }
            None => {
                if !ev.mods.shift {
                    self.doc_mut().selected_nodes.clear();
                }
                self.drag = Some(DragState::Marquee { start: p, cur: p });
            }
        }
    }

    fn hit_handle(&self, bounds: &Rect, ev: InputEvent) -> Option<usize> {
        let m = self.view().doc_to_screen();
        let r = bounds.transform(&m);
        let pts = [
            (r.x, r.y),
            (r.x + r.w / 2.0, r.y),
            (r.x + r.w, r.y),
            (r.x + r.w, r.y + r.h / 2.0),
            (r.x + r.w, r.y + r.h),
            (r.x + r.w / 2.0, r.y + r.h),
            (r.x, r.y + r.h),
            (r.x, r.y + r.h / 2.0),
        ];
        for (i, (hx, hy)) in pts.iter().enumerate() {
            if (ev.x - hx).abs() <= 6.0 && (ev.y - hy).abs() <= 6.0 {
                return Some(i);
            }
        }
        None
    }

    fn shape_down(&mut self, shape: &'static str, ev: InputEvent, p: Vec2) {
        self.drag = Some(DragState::DrawShape { shape, start: p, cur: p, constrain: ev.mods.shift });
    }

    fn pen_click(&mut self, _ev: InputEvent, p: Vec2) {
        // close the path when clicking near the first anchor
        if self.pen.points.len() >= 3 {
            let first = self.pen.points[0];
            let close_px = 8.0 / self.view().zoom;
            if first.distance(p) < close_px {
                self.finish_pen(true);
                return;
            }
        }
        self.pen.points.push(p);
    }

    pub fn finish_pen(&mut self, close: bool) {
        let pts = std::mem::take(&mut self.pen.points);
        if pts.len() < 2 {
            self.dirty_frame();
            return;
        }
        let d = ed_engine::shapes::polyline_to_path_data(&pts, close);
        let parent = self.engine.artboard_at(self.doc(), pts[0]);
        let stroke_w = self.tool_f64("pen.stroke-width", 2.0);
        let fg = self.fg;
        let blobs = std::mem::take(&mut self.blobs);
        {
            let doc = self.doc_mut();
            doc.begin_txn("Pen path");
            let mut params = BTreeMap::new();
            params.insert("d".into(), Value::Str(d));
            params.insert("fill".into(), Value::Str(if close { "solid".into() } else { "none".to_string() }));
            params.insert("fill-color".into(), Value::Color(fg));
            params.insert("stroke".into(), Value::Str(if close && stroke_w <= 0.0 { "none".into() } else { "solid".to_string() }));
            params.insert("stroke-color".into(), Value::Color(ed_core::Color::BLACK));
            params.insert("stroke-width".into(), Value::F64(if close { stroke_w } else { stroke_w.max(1.0) }));
            if let Ok(id) = doc.create_node(NodeKind::Path, parent, params, &blobs) {
                doc.selected_nodes = vec![id];
            }
            doc.commit_txn();
        }
        self.blobs = blobs;
        self.dirty_frame();
    }

    fn text_click(&mut self, p: Vec2) {
        let parent = self.engine.artboard_at(self.doc(), p);
        let size = self.tool_f64("text.size", 24.0);
        let fg = self.fg;
        let blobs = std::mem::take(&mut self.blobs);
        {
            let doc = self.doc_mut();
            doc.begin_txn("Add text");
            let mut params = BTreeMap::new();
            params.insert("text".into(), Value::Str("Text".into()));
            params.insert("x".into(), Value::F64(p.x));
            params.insert("y".into(), Value::F64(p.y));
            params.insert("w".into(), Value::F64(200.0));
            params.insert("h".into(), Value::F64(size * 1.5));
            params.insert("font-size".into(), Value::F64(size));
            params.insert("align".into(), Value::Str("left".into()));
            params.insert("auto-size".into(), Value::Bool(true));
            params.insert("fill-color".into(), Value::Color(fg));
            if let Ok(id) = doc.create_node(NodeKind::Text, parent, params, &blobs) {
                doc.selected_nodes = vec![id];
            }
            doc.commit_txn();
        }
        self.blobs = blobs;
        self.dirty_frame();
    }

    // painting ---------------------------------------------------------

    fn paint_brush_params(&self, pixel_perfect: bool, erase: bool) -> BrushParams {
        if erase {
            BrushParams {
                size: self.tool_f64("eraser.size", 24.0),
                hardness: self.tool_f64("eraser.hardness", 1.0),
                opacity: 1.0,
                flow: 1.0,
                erase: true,
                pixel_perfect: false,
            }
        } else if pixel_perfect {
            BrushParams {
                size: self.tool_f64("pencil.size", 1.0),
                hardness: 1.0,
                opacity: 1.0,
                flow: 1.0,
                erase: false,
                pixel_perfect: true,
            }
        } else {
            BrushParams {
                size: self.tool_f64("brush.size", 16.0),
                hardness: self.tool_f64("brush.hardness", 0.8),
                opacity: self.tool_f64("brush.opacity", 1.0),
                flow: self.tool_f64("brush.flow", 1.0),
                erase: false,
                pixel_perfect: false,
            }
        }
    }

    /// Find or create the paint target bitmap. Spec §2.5: painting is
    /// destructive within a Bitmap node; "paint as strokes" records a
    /// StrokeSet instead.
    fn paint_target(&mut self, p: Vec2) -> Option<(NodeId, Vec2)> {
        // selected bitmap?
        let sel = self.doc().selected_nodes.clone();
        for id in sel {
            if let Some(n) = self.doc().node(id) {
                if n.kind == NodeKind::Bitmap && n.visible() && !n.locked() {
                    let off = self.doc().node_position(id);
                    return Some((id, off));
                }
            }
        }
        // bitmap under cursor?
        if let Some(id) = self.engine.hit_test(self.doc(), p, true) {
            if let Some(n) = self.doc().node(id) {
                if n.kind == NodeKind::Bitmap {
                    let off = self.doc().node_position(id);
                    return Some((id, off));
                }
            }
        }
        // create a new paint layer covering the artboard (or 1024² at point)
        let ab = self.engine.artboard_at(self.doc(), p);
        let rect = ab
            .and_then(|a| self.doc().artboard_rect(a))
            .unwrap_or(Rect::new(p.x - 512.0, p.y - 512.0, 1024.0, 1024.0));
        let blobs = std::mem::take(&mut self.blobs);
        let id = {
            let doc = self.doc_mut();
            doc.begin_txn("Paint");
            let mut params = BTreeMap::new();
            params.insert("name".into(), Value::Str("Paint layer".into()));
            params.insert("x".into(), Value::F64(rect.x));
            params.insert("y".into(), Value::F64(rect.y));
            let id = doc.create_node(NodeKind::Bitmap, ab, params, &blobs).ok();
            if let Some(id) = id {
                if let Some(n) = doc.nodes.get_mut(&id) {
                    n.bitmap = Some(ed_document::BitmapData::new(
                        rect.w.ceil() as u32,
                        rect.h.ceil() as u32,
                    ));
                }
                doc.selected_nodes = vec![id];
            }
            // txn stays open: the stroke commits into the same txn
            id
        };
        self.blobs = blobs;
        id.map(|id| (id, Vec2::new(rect.x, rect.y)))
    }

    fn build_sel_mask(&self, offset: Vec2, node: NodeId) -> Option<SelMask> {
        let sel = self.doc().pixel_selection.as_ref()?;
        if sel.is_empty() {
            return None;
        }
        let n = self.doc().node(node)?;
        let bm = n.bitmap.as_ref()?;
        // rasterize selection over the bitmap's doc-space rect, then store
        // in bitmap-local coords
        let x0 = offset.x.floor() as i64;
        let y0 = offset.y.floor() as i64;
        let data = sel.rasterize(x0, y0, bm.width, bm.height);
        Some(SelMask { x0: 0, y0: 0, w: bm.width, h: bm.height, data })
    }

    fn paint_down(&mut self, ev: InputEvent, p: Vec2, pixel_perfect: bool) {
        // "paint as strokes" records re-editable stroke data (spec §2.5)
        if !pixel_perfect && self.tool_bool("brush.as-strokes", false) {
            self.strokes_down(ev, p);
            return;
        }
        let Some((node, offset)) = self.paint_target(p) else { return };
        if self.doc().open_txn_label().is_none() {
            self.doc_mut().begin_txn(if pixel_perfect { "Pencil" } else { "Brush" });
        }
        let mask = self.build_sel_mask(offset, node);
        let params = self.paint_brush_params(pixel_perfect, false);
        let color = self.fg.to_srgb8();
        let local = p - offset;
        let mut before = BTreeMap::new();
        if let Some(bm) = self.doc_mut().bitmap_mut(node) {
            capture_before(bm, local, params.size, &mut before);
            let mut prms = params;
            prms.size = params.size * ev.pressure.max(0.05);
            raster::stroke_segment(bm, local, local, &prms, color, 0.0, mask.as_ref());
        }
        self.drag = Some(DragState::Paint {
            node,
            offset,
            before,
            last: local,
            carry: 0.0,
            label: if pixel_perfect { "Pencil" } else { "Brush" },
            mask,
        });
    }

    fn erase_down(&mut self, ev: InputEvent, p: Vec2) {
        let Some((node, offset)) = self.paint_target(p) else { return };
        if self.doc().open_txn_label().is_none() {
            self.doc_mut().begin_txn("Eraser");
        }
        let mask = self.build_sel_mask(offset, node);
        let params = self.paint_brush_params(false, true);
        let local = p - offset;
        let mut before = BTreeMap::new();
        if let Some(bm) = self.doc_mut().bitmap_mut(node) {
            capture_before(bm, local, params.size, &mut before);
            let mut prms = params;
            prms.size = params.size * ev.pressure.max(0.05);
            raster::stroke_segment(bm, local, local, &prms, [0, 0, 0, 0], 0.0, mask.as_ref());
        }
        self.drag = Some(DragState::Paint {
            node,
            offset,
            before,
            last: local,
            carry: 0.0,
            label: "Eraser",
            mask,
        });
    }

    fn strokes_down(&mut self, ev: InputEvent, p: Vec2) {
        // target: selected StrokeSet or new one
        let sel = self.doc().selected_nodes.clone();
        let mut target = None;
        for id in sel {
            if let Some(n) = self.doc().node(id) {
                if n.kind == NodeKind::StrokeSet && !n.locked() {
                    target = Some(id);
                    break;
                }
            }
        }
        let node = match target {
            Some(id) => id,
            None => {
                let ab = self.engine.artboard_at(self.doc(), p);
                let blobs = std::mem::take(&mut self.blobs);
                let id = {
                    let doc = self.doc_mut();
                    doc.begin_txn("Brush strokes");
                    let id = doc.create_node(NodeKind::StrokeSet, ab, BTreeMap::new(), &blobs).ok();
                    if let Some(id) = id {
                        doc.selected_nodes = vec![id];
                    }
                    id
                };
                self.blobs = blobs;
                match id {
                    Some(id) => id,
                    None => return,
                }
            }
        };
        let prev = self.doc().node(node).map(|n| n.strokes.clone()).unwrap_or_default();
        let pt = StrokePoint { pos: p, pressure: ev.pressure };
        // live preview: push the in-progress stroke directly
        let stroke = Stroke {
            color: self.fg,
            size: self.tool_f64("brush.size", 16.0),
            hardness: self.tool_f64("brush.hardness", 0.8),
            opacity: self.tool_f64("brush.opacity", 1.0),
            erase: false,
            points: vec![pt],
        };
        if let Some(strokes) = self.doc_mut().strokes_mut(node) {
            strokes.push(stroke);
        }
        self.drag = Some(DragState::PaintStrokes { node, prev, points: vec![pt] });
    }

    fn fill_click(&mut self, ev: InputEvent, p: Vec2) {
        let tolerance = self.tool_f64("fill.tolerance", 0.1);
        let contiguous = self.tool_bool("fill.contiguous", true);
        let color = if ev.mods.alt { self.bg } else { self.fg };
        let hit = self.engine.hit_test(self.doc(), p, true);
        match hit.and_then(|id| self.doc().node(id).map(|n| (id, n.kind))) {
            Some((id, NodeKind::Bitmap)) => {
                let offset = self.doc().node_position(id);
                let local = p - offset;
                let mask = self.build_sel_mask(offset, id);
                let rgba = color.to_srgb8();
                let mut before = BTreeMap::new();
                let mut changed = false;
                if let Some(bm) = self.doc_mut().bitmap_mut(id) {
                    // capture the whole bitmap's touched tiles: fill can
                    // reach anywhere, snapshot all existing tiles + empties
                    for ty in 0..bm.tiles_down() {
                        for tx in 0..bm.tiles_across() {
                            before.insert((tx, ty), bm.tiles.get(&(tx, ty)).cloned());
                        }
                    }
                    if local.x >= 0.0 && local.y >= 0.0 {
                        changed = fill_with_mask(
                            bm,
                            (local.x as u32, local.y as u32),
                            rgba,
                            tolerance,
                            contiguous,
                            mask.as_ref(),
                        );
                    }
                }
                if changed {
                    let mut blobs = std::mem::take(&mut self.blobs);
                    {
                        let doc = self.doc_mut();
                        doc.begin_txn("Fill");
                        let _ = doc.commit_paint(id, &before, &mut blobs);
                        doc.commit_txn();
                    }
                    self.blobs = blobs;
                } else {
                    // revert (no visible change)
                }
                self.dirty_frame();
            }
            Some((id, NodeKind::Shape | NodeKind::Path | NodeKind::Text | NodeKind::GradientFill)) => {
                let blobs = std::mem::take(&mut self.blobs);
                {
                    let doc = self.doc_mut();
                    doc.begin_txn("Fill color");
                    let _ = doc.apply(
                        OpKind::ParamSet {
                            node_id: id,
                            path: "fill-color".into(),
                            value: Value::Color(color),
                            prev: None,
                        },
                        &blobs,
                    );
                    let _ = doc.apply(
                        OpKind::ParamSet {
                            node_id: id,
                            path: "fill".into(),
                            value: Value::Str("solid".into()),
                            prev: None,
                        },
                        &blobs,
                    );
                    doc.commit_txn();
                }
                self.blobs = blobs;
                self.dirty_frame();
            }
            _ => {}
        }
    }

    fn eyedrop(&mut self, ev: InputEvent, p: Vec2) {
        // sample the composite: render 1x1 region around the artboard point
        if let Some(ab) = self.engine.artboard_at(self.doc(), p) {
            if let Some(rect) = self.doc().artboard_rect(ab) {
                let doc = &self.docs[self.active].doc;
                if let Some(pm) = self.engine.render_artboard(doc, ab, 1.0, true) {
                    let x = (p.x - rect.x).floor() as u32;
                    let y = (p.y - rect.y).floor() as u32;
                    if x < pm.width() && y < pm.height() {
                        if let Some(px) = pm.pixel(x, y) {
                            let c = px.demultiply();
                            let col = ed_core::Color::from_srgb8(c.red(), c.green(), c.blue(), 255);
                            if ev.mods.alt {
                                self.bg = col;
                            } else {
                                self.fg = col;
                            }
                        }
                    }
                }
            }
        }
    }

    fn wand_click(&mut self, ev: InputEvent, p: Vec2) {
        let tolerance = self.tool_f64("wand.tolerance", 0.15);
        let contiguous = self.tool_bool("wand.contiguous", true);
        let Some(ab) = self.engine.artboard_at(self.doc(), p) else { return };
        let Some(rect) = self.doc().artboard_rect(ab) else { return };
        let doc = &self.docs[self.active].doc;
        let Some(pm) = self.engine.render_artboard(doc, ab, 1.0, true) else { return };
        let rgba = crate::session::demultiply(&pm);
        let sx = (p.x - rect.x).floor().max(0.0) as u32;
        let sy = (p.y - rect.y).floor().max(0.0) as u32;
        let mask = raster::magic_wand(&rgba, pm.width(), pm.height(), (sx, sy), tolerance, contiguous);
        let geom = SelGeom::Mask {
            x: rect.x as i64,
            y: rect.y as i64,
            w: pm.width(),
            h: pm.height(),
            data: mask,
        };
        let mode = combine_mode(ev.mods);
        let doc = self.doc_mut();
        match doc.pixel_selection.as_mut() {
            Some(sel) if mode != CombineMode::Replace => sel.combine(geom, mode),
            _ => doc.pixel_selection = Some(PixelSelection::single(geom)),
        }
        self.dirty_frame();
    }

    // ------------------------------------------------------------ move

    fn pointer_move(&mut self, ev: InputEvent, p: Vec2) {
        let Some(drag) = self.drag.as_mut() else { return };
        match drag {
            DragState::MoveNodes { start, origs, orig_strokes, orig_paths, moved } => {
                let mut delta = p - *start;
                if ev.mods.shift {
                    // axis constrain
                    if delta.x.abs() > delta.y.abs() {
                        delta.y = 0.0;
                    } else {
                        delta.x = 0.0;
                    }
                }
                *moved = *moved || delta.length() > 0.5;
                let updates: Vec<(NodeId, String, f64)> = origs
                    .iter()
                    .map(|(id, key, v)| {
                        let d = if key.starts_with('x') { delta.x } else { delta.y };
                        (*id, key.clone(), v + d)
                    })
                    .collect();
                let stroke_updates: Vec<(NodeId, Vec<Stroke>)> = orig_strokes
                    .iter()
                    .map(|(id, strokes)| {
                        let mut s2 = strokes.clone();
                        for s in &mut s2 {
                            for pt in &mut s.points {
                                pt.pos = pt.pos + delta;
                            }
                        }
                        (*id, s2)
                    })
                    .collect();
                let path_updates: Vec<(NodeId, String)> = orig_paths
                    .iter()
                    .map(|(id, d)| (*id, offset_path_data(d, delta.x, delta.y)))
                    .collect();
                for (id, key, v) in updates {
                    self.doc_mut().preview_param(id, &key, Value::F64(v));
                }
                for (id, strokes) in stroke_updates {
                    if let Some(s) = self.doc_mut().strokes_mut(id) {
                        *s = strokes;
                    }
                }
                for (id, d) in path_updates {
                    self.doc_mut().preview_param(id, "d", Value::Str(d));
                }
            }
            DragState::ResizeNodes { handle, start_bounds, origs } => {
                let sb = *start_bounds;
                let (mut x0, mut y0, mut x1, mut y1) = (sb.x, sb.y, sb.x + sb.w, sb.y + sb.h);
                match handle {
                    0 => {
                        x0 = p.x;
                        y0 = p.y;
                    }
                    1 => y0 = p.y,
                    2 => {
                        x1 = p.x;
                        y0 = p.y;
                    }
                    3 => x1 = p.x,
                    4 => {
                        x1 = p.x;
                        y1 = p.y;
                    }
                    5 => y1 = p.y,
                    6 => {
                        x0 = p.x;
                        y1 = p.y;
                    }
                    _ => x0 = p.x,
                }
                if ev.mods.shift && sb.w > 0.0 && sb.h > 0.0 {
                    // proportional
                    let sx = (x1 - x0) / sb.w;
                    let sy = (y1 - y0) / sb.h;
                    let s = sx.abs().max(sy.abs());
                    let sx = s * sx.signum();
                    let sy = s * sy.signum();
                    x1 = x0 + sb.w * sx;
                    y1 = y0 + sb.h * sy;
                }
                let new = Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs().max(1.0), (y1 - y0).abs().max(1.0));
                let updates: Vec<(NodeId, Rect)> = origs
                    .iter()
                    .map(|(id, r)| {
                        let fx = if sb.w > 0.0 { (r.x - sb.x) / sb.w } else { 0.0 };
                        let fy = if sb.h > 0.0 { (r.y - sb.y) / sb.h } else { 0.0 };
                        let fw = if sb.w > 0.0 { r.w / sb.w } else { 1.0 };
                        let fh = if sb.h > 0.0 { r.h / sb.h } else { 1.0 };
                        (
                            *id,
                            Rect::new(
                                new.x + fx * new.w,
                                new.y + fy * new.h,
                                fw * new.w,
                                fh * new.h,
                            ),
                        )
                    })
                    .collect();
                for (id, r) in updates {
                    let doc = self.doc_mut();
                    doc.preview_param(id, "x", Value::F64(r.x));
                    doc.preview_param(id, "y", Value::F64(r.y));
                    doc.preview_param(id, "w", Value::F64(r.w));
                    doc.preview_param(id, "h", Value::F64(r.h));
                }
            }
            DragState::Marquee { cur, .. } => {
                *cur = p;
                let (s, c) = match self.drag.as_ref() {
                    Some(DragState::Marquee { start, cur }) => (*start, *cur),
                    _ => unreachable!(),
                };
                self.overlays = vec![Overlay::RectOutline {
                    rect: Rect::from_points(s, c),
                    dashed: true,
                }];
            }
            DragState::DrawShape { shape, start, cur, constrain } => {
                *cur = p;
                *constrain = ev.mods.shift;
                let (shape, s, mut c, con) = match self.drag.as_ref() {
                    Some(DragState::DrawShape { shape, start, cur, constrain }) => {
                        (*shape, *start, *cur, *constrain)
                    }
                    _ => unreachable!(),
                };
                if con && shape != "line" && shape != "arrow" {
                    let dx = c.x - s.x;
                    let dy = c.y - s.y;
                    let d = dx.abs().max(dy.abs());
                    c = Vec2::new(s.x + d * dx.signum(), s.y + d * dy.signum());
                }
                self.overlays = match shape {
                    "ellipse" => vec![Overlay::EllipseOutline { rect: Rect::from_points(s, c), dashed: false }],
                    "line" | "arrow" => vec![Overlay::Line { from: s, to: c, dashed: false }],
                    _ => vec![Overlay::RectOutline { rect: Rect::from_points(s, c), dashed: false }],
                };
            }
            DragState::Paint { node, offset, before, last, carry, mask, .. } => {
                let (node, offset) = (*node, *offset);
                let local = p - offset;
                let prev = *last;
                let carry_in = *carry;
                *last = local;
                // move heavy state out so the drag borrow can end
                let seg_mask = std::mem::take(mask);
                let mut new_before = std::mem::take(before);
                let params = match self.tool {
                    ToolKind::Pencil => self.paint_brush_params(true, false),
                    ToolKind::Eraser => self.paint_brush_params(false, true),
                    _ => self.paint_brush_params(false, false),
                };
                let mut prms = params;
                prms.size = params.size * ev.pressure.max(0.05);
                let color = if params.erase { [0, 0, 0, 0] } else { self.fg.to_srgb8() };
                let mut new_carry = carry_in;
                if let Some(bm) = self.doc_mut().bitmap_mut(node) {
                    capture_before_segment(bm, prev, local, prms.size, &mut new_before);
                    new_carry = raster::stroke_segment(
                        bm,
                        prev,
                        local,
                        &prms,
                        color,
                        carry_in,
                        seg_mask.as_ref(),
                    );
                }
                if let Some(DragState::Paint { before, carry, mask, .. }) = self.drag.as_mut() {
                    *before = new_before;
                    *carry = new_carry;
                    *mask = seg_mask;
                }
                self.overlays = vec![Overlay::BrushCursor { pos: p, size: params.size }];
            }
            DragState::PaintStrokes { node, points, .. } => {
                let node = *node;
                let pt = StrokePoint { pos: p, pressure: ev.pressure };
                points.push(pt);
                if let Some(strokes) = self.doc_mut().strokes_mut(node) {
                    if let Some(last) = strokes.last_mut() {
                        last.points.push(pt);
                    }
                }
            }
            DragState::SelectShape { kind, start, cur, .. } => {
                *cur = p;
                let (kind, s, c) = match self.drag.as_ref() {
                    Some(DragState::SelectShape { kind, start, cur, .. }) => (*kind, *start, *cur),
                    _ => unreachable!(),
                };
                self.overlays = if kind == "ellipse" {
                    vec![Overlay::EllipseOutline { rect: Rect::from_points(s, c), dashed: true }]
                } else {
                    vec![Overlay::RectOutline { rect: Rect::from_points(s, c), dashed: true }]
                };
            }
            DragState::Lasso { points, .. } => {
                if points.last().map(|l| l.distance(p) > 1.0).unwrap_or(true) {
                    points.push(p);
                }
                let pts = points.clone();
                self.overlays = vec![Overlay::PolyOutline { points: pts, close: true, dashed: true }];
            }
            DragState::Pan { start_pan, start_screen } => {
                let sp = *start_pan;
                let ss = *start_screen;
                let zoom = self.view().zoom;
                let v = self.view_mut();
                v.pan = Vec2::new(sp.x - (ev.x - ss.x) / zoom, sp.y - (ev.y - ss.y) / zoom);
            }
            DragState::GradientDrag { cur, start } => {
                *cur = p;
                let (s, c) = (*start, *cur);
                self.overlays = vec![Overlay::Line { from: s, to: c, dashed: false }];
            }
        }
        self.dirty_frame();
    }

    // ------------------------------------------------------------ up

    fn pointer_up(&mut self, _ev: InputEvent, p: Vec2) {
        let Some(drag) = self.drag.take() else { return };
        self.overlays.clear();
        match drag {
            DragState::MoveNodes { origs, orig_strokes, orig_paths, moved, start } => {
                if !moved {
                    return;
                }
                let delta = p - start;
                // revert previews, then commit one txn (spec §3.3)
                for (id, key, v) in &origs {
                    self.doc_mut().preview_param(*id, key, Value::F64(*v));
                }
                for (id, strokes) in &orig_strokes {
                    if let Some(s) = self.doc_mut().strokes_mut(*id) {
                        *s = strokes.clone();
                    }
                }
                for (id, d) in &orig_paths {
                    self.doc_mut().preview_param(*id, "d", Value::Str(d.clone()));
                }
                let blobs = std::mem::take(&mut self.blobs);
                {
                    let doc = self.doc_mut();
                    doc.begin_txn("Move");
                    for (id, key, v) in &origs {
                        let d = if key.starts_with('x') { delta.x } else { delta.y };
                        let _ = doc.apply(
                            OpKind::ParamSet {
                                node_id: *id,
                                path: key.clone(),
                                value: Value::F64(v + d),
                                prev: None,
                            },
                            &blobs,
                        );
                    }
                    for (id, strokes) in &orig_strokes {
                        let mut s2 = strokes.clone();
                        for s in &mut s2 {
                            for pt in &mut s.points {
                                pt.pos = pt.pos + delta;
                            }
                        }
                        let _ = doc.apply(
                            OpKind::StrokesSet { node_id: *id, strokes: s2, prev: Vec::new() },
                            &blobs,
                        );
                    }
                    for (id, d) in &orig_paths {
                        let _ = doc.apply(
                            OpKind::ParamSet {
                                node_id: *id,
                                path: "d".into(),
                                value: Value::Str(offset_path_data(d, delta.x, delta.y)),
                                prev: None,
                            },
                            &blobs,
                        );
                    }
                    doc.commit_txn();
                }
                self.blobs = blobs;
                // reparent across artboards (spec §2.1 nodes draggable between)
                self.reparent_after_move();
            }
            DragState::ResizeNodes { origs, .. } => {
                // previews already hold final values; capture, revert, commit
                let finals: Vec<(NodeId, Rect)> = origs
                    .iter()
                    .map(|(id, _)| {
                        let n = self.doc().node(*id).unwrap();
                        (
                            *id,
                            Rect::new(
                                self.doc().param_f64(n, "x", 0.0),
                                self.doc().param_f64(n, "y", 0.0),
                                self.doc().param_f64(n, "w", 0.0),
                                self.doc().param_f64(n, "h", 0.0),
                            ),
                        )
                    })
                    .collect();
                for (id, r) in &origs {
                    let doc = self.doc_mut();
                    doc.preview_param(*id, "x", Value::F64(r.x));
                    doc.preview_param(*id, "y", Value::F64(r.y));
                    doc.preview_param(*id, "w", Value::F64(r.w));
                    doc.preview_param(*id, "h", Value::F64(r.h));
                }
                let blobs = std::mem::take(&mut self.blobs);
                {
                    let doc = self.doc_mut();
                    doc.begin_txn("Resize");
                    for (id, r) in finals {
                        for (k, v) in [("x", r.x), ("y", r.y), ("w", r.w), ("h", r.h)] {
                            let _ = doc.apply(
                                OpKind::ParamSet {
                                    node_id: id,
                                    path: k.into(),
                                    value: Value::F64(v),
                                    prev: None,
                                },
                                &blobs,
                            );
                        }
                    }
                    doc.commit_txn();
                }
                self.blobs = blobs;
            }
            DragState::Marquee { start, cur } => {
                let r = Rect::from_points(start, cur);
                if r.w > 2.0 && r.h > 2.0 {
                    let mut hits = Vec::new();
                    let roots: Vec<NodeId> = self.doc().children_of(None).to_vec();
                    for root in roots {
                        let node = self.doc().node(root);
                        let is_ab = node.map(|n| n.kind == NodeKind::Artboard).unwrap_or(false);
                        let kids: Vec<NodeId> = if is_ab {
                            self.doc().children_of(Some(root)).to_vec()
                        } else {
                            vec![root]
                        };
                        for id in kids {
                            if let Some(b) = self.engine.node_bounds(self.doc(), id) {
                                if b.intersects(&r) {
                                    hits.push(id);
                                }
                            }
                        }
                    }
                    self.doc_mut().selected_nodes = hits;
                }
            }
            DragState::DrawShape { shape, start, cur, constrain } => {
                let mut c = cur;
                if constrain && shape != "line" && shape != "arrow" {
                    let dx = c.x - start.x;
                    let dy = c.y - start.y;
                    let d = dx.abs().max(dy.abs());
                    c = Vec2::new(start.x + d * dx.signum(), start.y + d * dy.signum());
                }
                let mut r = Rect::from_points(start, c);
                if r.w < 3.0 && r.h < 3.0 {
                    r = Rect::new(start.x, start.y, 100.0, 100.0);
                }
                self.create_shape(shape, r, start, c);
            }
            DragState::Paint { node, before, label, .. } => {
                let mut blobs = std::mem::take(&mut self.blobs);
                {
                    let doc = self.doc_mut();
                    if doc.open_txn_label().is_none() {
                        doc.begin_txn(label);
                    }
                    let _ = doc.commit_paint(node, &before, &mut blobs);
                    doc.commit_txn();
                }
                self.blobs = blobs;
            }
            DragState::PaintStrokes { node, prev, points } => {
                // revert preview, commit as ops
                let stroke = Stroke {
                    color: self.fg,
                    size: self.tool_f64("brush.size", 16.0),
                    hardness: self.tool_f64("brush.hardness", 0.8),
                    opacity: self.tool_f64("brush.opacity", 1.0),
                    erase: false,
                    points,
                };
                let mut all = prev.clone();
                all.push(stroke);
                if let Some(s) = self.doc_mut().strokes_mut(node) {
                    *s = prev.clone();
                }
                let blobs = std::mem::take(&mut self.blobs);
                {
                    let doc = self.doc_mut();
                    if doc.open_txn_label().is_none() {
                        doc.begin_txn("Brush strokes");
                    }
                    let _ = doc.apply(
                        OpKind::StrokesSet { node_id: node, strokes: all, prev: Vec::new() },
                        &blobs,
                    );
                    doc.commit_txn();
                }
                self.blobs = blobs;
            }
            DragState::SelectShape { kind, start, cur, mode } => {
                let r = Rect::from_points(start, cur);
                if r.w < 1.0 || r.h < 1.0 {
                    if mode == CombineMode::Replace {
                        self.doc_mut().pixel_selection = None;
                    }
                } else {
                    let geom = if kind == "ellipse" {
                        SelGeom::Ellipse { rect: r }
                    } else {
                        SelGeom::Rect { rect: r }
                    };
                    let feather = self.tool_f64("sel.feather", 0.0);
                    let doc = self.doc_mut();
                    match doc.pixel_selection.as_mut() {
                        Some(sel) if mode != CombineMode::Replace => sel.combine(geom, mode),
                        _ => {
                            let mut sel = PixelSelection::single(geom);
                            sel.feather = feather;
                            doc.pixel_selection = Some(sel);
                        }
                    }
                }
            }
            DragState::Lasso { points, mode } => {
                if points.len() >= 3 {
                    let geom = SelGeom::Polygon { points };
                    let doc = self.doc_mut();
                    match doc.pixel_selection.as_mut() {
                        Some(sel) if mode != CombineMode::Replace => sel.combine(geom, mode),
                        _ => doc.pixel_selection = Some(PixelSelection::single(geom)),
                    }
                } else if mode == CombineMode::Replace {
                    self.doc_mut().pixel_selection = None;
                }
            }
            DragState::Pan { .. } => {}
            DragState::GradientDrag { start, cur } => {
                if start.distance(cur) > 3.0 {
                    self.create_gradient(start, cur);
                }
            }
        }
        self.dirty_frame();
    }

    fn double_click(&mut self, ev: InputEvent, p: Vec2) {
        match self.tool {
            ToolKind::Pen => self.finish_pen(false),
            ToolKind::Select => {
                // enter group / deep select
                if let Some(id) = self.engine.hit_test(self.doc(), p, true) {
                    self.doc_mut().selected_nodes = vec![id];
                }
                let _ = ev;
            }
            _ => {}
        }
    }

    fn create_shape(&mut self, shape: &str, r: Rect, start: Vec2, end: Vec2) {
        let parent = self.engine.artboard_at(self.doc(), r.center());
        let fg = self.fg;
        let stroke_w = self.tool_f64("shape.stroke-width", 0.0);
        let radius = self.tool_f64("shape.radius", 0.0);
        let sides = self.tool_f64("shape.sides", 6.0);
        let points = self.tool_f64("shape.points", 5.0);
        let blobs = std::mem::take(&mut self.blobs);
        {
            let doc = self.doc_mut();
            doc.begin_txn(&format!("Draw {shape}"));
            let mut params = BTreeMap::new();
            params.insert("shape".into(), Value::Str(shape.into()));
            params.insert("name".into(), Value::Str(capitalize(shape)));
            if shape == "line" || shape == "arrow" {
                params.insert("x".into(), Value::F64(start.x));
                params.insert("y".into(), Value::F64(start.y));
                params.insert("x2".into(), Value::F64(end.x));
                params.insert("y2".into(), Value::F64(end.y));
                params.insert("stroke".into(), Value::Str("solid".into()));
                params.insert("stroke-color".into(), Value::Color(fg));
                params.insert("stroke-width".into(), Value::F64(stroke_w.max(2.0)));
                params.insert("fill".into(), Value::Str("none".into()));
            } else {
                params.insert("x".into(), Value::F64(r.x));
                params.insert("y".into(), Value::F64(r.y));
                params.insert("w".into(), Value::F64(r.w));
                params.insert("h".into(), Value::F64(r.h));
                params.insert("fill".into(), Value::Str("solid".into()));
                params.insert("fill-color".into(), Value::Color(fg));
                if stroke_w > 0.0 {
                    params.insert("stroke".into(), Value::Str("solid".into()));
                    params.insert("stroke-color".into(), Value::Color(ed_core::Color::BLACK));
                    params.insert("stroke-width".into(), Value::F64(stroke_w));
                }
                if shape == "rect" && radius > 0.0 {
                    params.insert("radius".into(), Value::F64(radius));
                }
                if shape == "polygon" {
                    params.insert("sides".into(), Value::F64(sides));
                }
                if shape == "star" {
                    params.insert("sides".into(), Value::F64(points));
                    params.insert("inner-radius".into(), Value::F64(0.5));
                }
            }
            if let Ok(id) = doc.create_node(NodeKind::Shape, parent, params, &blobs) {
                doc.selected_nodes = vec![id];
            }
            doc.commit_txn();
        }
        self.blobs = blobs;
        self.dirty_frame();
    }

    fn create_gradient(&mut self, from: Vec2, to: Vec2) {
        let parent = self.engine.artboard_at(self.doc(), from);
        // region: pixel selection bounds, else the artboard, else 512²
        let region = self
            .doc()
            .pixel_selection
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| s.bounds())
            .or_else(|| parent.and_then(|ab| self.doc().artboard_rect(ab)))
            .unwrap_or(Rect::new(from.x - 256.0, from.y - 256.0, 512.0, 512.0));
        let kind = self.tool_str("gradient.kind", "linear");
        let fg = self.fg;
        let bg = self.bg;
        let blobs = std::mem::take(&mut self.blobs);
        {
            let doc = self.doc_mut();
            doc.begin_txn("Gradient");
            let mut params = BTreeMap::new();
            params.insert("x".into(), Value::F64(region.x));
            params.insert("y".into(), Value::F64(region.y));
            params.insert("w".into(), Value::F64(region.w));
            params.insert("h".into(), Value::F64(region.h));
            params.insert("gradient".into(), Value::Str(kind));
            params.insert("from".into(), Value::Point(from));
            params.insert("to".into(), Value::Point(to));
            params.insert("from-color".into(), Value::Color(fg));
            params.insert("to-color".into(), Value::Color(bg));
            if let Ok(id) = doc.create_node(NodeKind::GradientFill, parent, params, &blobs) {
                doc.selected_nodes = vec![id];
            }
            doc.commit_txn();
        }
        self.blobs = blobs;
        self.dirty_frame();
    }

    fn reparent_after_move(&mut self) {
        let ids = self.doc().selected_nodes.clone();
        let blobs = std::mem::take(&mut self.blobs);
        {
            for id in ids {
                let Some(node) = self.doc().node(id) else { continue };
                // only reparent nodes sitting directly on an artboard/top level
                let parent_is_ab = match node.parent {
                    None => true,
                    Some(p) => self.doc().node(p).map(|n| n.kind == NodeKind::Artboard).unwrap_or(false),
                };
                if !parent_is_ab {
                    continue;
                }
                let Some(b) = self.engine.node_bounds(self.doc(), id) else { continue };
                let target = self.engine.artboard_at(self.doc(), b.center());
                if target != node.parent {
                    let frac = self.doc().frac_for_append(target);
                    let doc = self.doc_mut();
                    doc.begin_txn("Move to artboard");
                    let _ = doc.apply(
                        OpKind::NodeMove {
                            node_id: id,
                            new_parent: target,
                            frac_index: frac,
                            prev_parent: None,
                            prev_frac: String::new(),
                        },
                        &blobs,
                    );
                    doc.commit_txn();
                }
            }
        }
        self.blobs = blobs;
    }

    // ------------------------------------------------------------ keys

    pub fn key(&mut self, key: &str, mods: Modifiers) {
        match key {
            "Escape" => {
                self.cancel_drag();
                self.doc_mut().pixel_selection = None;
                self.doc_mut().selected_nodes.clear();
            }
            "Enter" => {
                if !self.pen.points.is_empty() {
                    self.finish_pen(false);
                }
            }
            "Delete" | "Backspace" => self.delete_selection(),
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                let step = if mods.shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    "ArrowLeft" => (-step, 0.0),
                    "ArrowRight" => (step, 0.0),
                    "ArrowUp" => (0.0, -step),
                    _ => (0.0, step),
                };
                self.nudge(dx, dy);
            }
            _ => {}
        }
        self.dirty_frame();
    }

    fn nudge(&mut self, dx: f64, dy: f64) {
        let ids = self.doc().selected_nodes.clone();
        if ids.is_empty() {
            return;
        }
        let blobs = std::mem::take(&mut self.blobs);
        {
            let doc = self.doc_mut();
            doc.begin_txn("Nudge");
            for id in ids {
                let updates: Vec<(&str, f64)> = {
                    let Some(n) = doc.node(id) else { continue };
                    [("x", dx), ("y", dy), ("x2", dx), ("y2", dy)]
                        .into_iter()
                        .filter_map(|(key, d)| {
                            n.get_param(key).and_then(|v| v.as_f64()).map(|v| (key, v + d))
                        })
                        .collect()
                };
                for (key, v) in updates {
                    let _ = doc.apply(
                        OpKind::ParamSet {
                            node_id: id,
                            path: key.into(),
                            value: Value::F64(v),
                            prev: None,
                        },
                        &blobs,
                    );
                }
            }
            doc.commit_txn();
        }
        self.blobs = blobs;
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Snapshot the tiles a stamp at `pos` (diameter `size`) may touch,
/// before mutation (spec §2.5 tile deltas).
fn capture_before(
    bm: &ed_document::BitmapData,
    pos: Vec2,
    size: f64,
    before: &mut BTreeMap<(u32, u32), Option<Vec<u8>>>,
) {
    let r = size / 2.0 + 2.0;
    let t = ed_document::TILE_SIZE as f64;
    let tx0 = (((pos.x - r) / t).floor().max(0.0)) as u32;
    let ty0 = (((pos.y - r) / t).floor().max(0.0)) as u32;
    let tx1 = (((pos.x + r) / t).ceil().max(0.0) as u32).min(bm.tiles_across());
    let ty1 = (((pos.y + r) / t).ceil().max(0.0) as u32).min(bm.tiles_down());
    for ty in ty0..ty1 {
        for tx in tx0..tx1 {
            before.entry((tx, ty)).or_insert_with(|| bm.tiles.get(&(tx, ty)).cloned());
        }
    }
}

fn capture_before_segment(
    bm: &ed_document::BitmapData,
    from: Vec2,
    to: Vec2,
    size: f64,
    before: &mut BTreeMap<(u32, u32), Option<Vec<u8>>>,
) {
    let steps = (from.distance(to) / (ed_document::TILE_SIZE as f64 / 2.0)).ceil() as usize + 1;
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        capture_before(bm, from.lerp(to, t), size, before);
    }
}

/// Flood fill constrained by an optional selection mask.
fn fill_with_mask(
    bm: &mut ed_document::BitmapData,
    seed: (u32, u32),
    color: [u8; 4],
    tolerance: f64,
    contiguous: bool,
    mask: Option<&SelMask>,
) -> bool {
    match mask {
        None => raster::flood_fill(bm, seed, color, tolerance, contiguous),
        Some(m) => {
            // fill only where mask coverage > 0; simple approach: fill a copy
            // then merge masked pixels
            let mut copy = bm.clone();
            if !raster::flood_fill(&mut copy, seed, color, tolerance, contiguous) {
                return false;
            }
            let mut changed = false;
            for y in 0..bm.height {
                for x in 0..bm.width {
                    let cov = {
                        let ix = x as i64 - m.x0;
                        let iy = y as i64 - m.y0;
                        if ix < 0 || iy < 0 || ix >= m.w as i64 || iy >= m.h as i64 {
                            0u8
                        } else {
                            m.data[(iy as usize) * (m.w as usize) + ix as usize]
                        }
                    };
                    if cov > 127 {
                        let new = copy.get_pixel(x, y);
                        if new != bm.get_pixel(x, y) {
                            bm.set_pixel(x, y, new);
                            changed = true;
                        }
                    }
                }
            }
            changed
        }
    }
}

/// Translate absolute SVG path data (M/L/C/Q, as we emit) by (dx, dy).
pub fn offset_path_data(d: &str, dx: f64, dy: f64) -> String {
    let mut out = String::with_capacity(d.len());
    let mut nums: Vec<f64> = Vec::new();
    let mut chars = d.chars().peekable();
    let mut is_x = true;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            out.push(c);
            out.push(' ');
            chars.next();
            is_x = true;
            nums.clear();
        } else if c.is_ascii_digit() || c == '-' || c == '.' || c == '+' {
            let mut s = String::new();
            while let Some(&c2) = chars.peek() {
                if c2.is_ascii_digit() || c2 == '.' || ((c2 == '-' || c2 == '+') && s.is_empty()) {
                    s.push(c2);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Ok(v) = s.parse::<f64>() {
                let off = if is_x { dx } else { dy };
                out.push_str(&format!("{:.2} ", v + off));
                is_x = !is_x;
            }
        } else {
            chars.next();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_path_data_translates_pairs() {
        let d = "M10 20 L30 40 Q50 60 70 80 Z";
        let out = offset_path_data(d, 5.0, -5.0);
        assert!(out.contains("15.00 15.00"), "{out}");
        assert!(out.contains("35.00 35.00"), "{out}");
        assert!(out.contains("75.00 75.00"), "{out}");
        assert!(out.contains('Z'));
    }
}
