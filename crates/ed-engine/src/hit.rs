//! Hit testing in document space, topmost-first (spec §6.4 move tool).

use crate::render::{modifier_matrix, Engine};
use ed_core::{NodeId, Vec2};
use ed_document::{Document, NodeKind};

impl Engine {
    /// Topmost node at a document-space point. Skips locked/invisible.
    /// `deep` enters groups; otherwise returns the outermost group that is
    /// a direct child of an artboard/top-level (Figma-style click).
    pub fn hit_test(&self, doc: &Document, p: Vec2, deep: bool) -> Option<NodeId> {
        // walk top-level in reverse z (topmost first)
        for &root in doc.children_of(None).iter().rev() {
            let node = doc.node(root)?;
            if node.kind == NodeKind::Artboard {
                let rect = doc.artboard_rect(root)?;
                if !rect.contains(p) {
                    continue;
                }
                if let Some(hit) = self.hit_children(doc, root, p) {
                    return Some(if deep { hit.1 } else { hit.0 });
                }
            } else if let Some(hit) = self.hit_node(doc, root, p) {
                return Some(if deep { hit } else { root });
            }
        }
        None
    }

    /// Artboard under a point (for paste targets, new-node parents).
    pub fn artboard_at(&self, doc: &Document, p: Vec2) -> Option<NodeId> {
        for &root in doc.children_of(None).iter().rev() {
            if doc.node(root)?.kind == NodeKind::Artboard && doc.artboard_rect(root)?.contains(p) {
                return Some(root);
            }
        }
        None
    }

    /// Returns (direct child of `parent` on the hit chain, deepest leaf hit).
    fn hit_children(&self, doc: &Document, parent: NodeId, p: Vec2) -> Option<(NodeId, NodeId)> {
        for &c in doc.children_of(Some(parent)).iter().rev() {
            if let Some(leaf) = self.hit_node(doc, c, p) {
                return Some((c, leaf));
            }
        }
        None
    }

    /// Deepest node hit within this subtree, or None.
    fn hit_node(&self, doc: &Document, id: NodeId, p: Vec2) -> Option<NodeId> {
        let node = doc.node(id)?;
        if !node.visible() || node.locked() {
            return None;
        }
        // un-apply this node's transform modifiers
        let m = modifier_matrix(doc, node);
        let local = m.invert().map(|inv| inv.apply(p)).unwrap_or(p);

        match node.kind {
            NodeKind::Group | NodeKind::Layer => {
                for &c in doc.children_of(Some(id)).iter().rev() {
                    if let Some(hit) = self.hit_node(doc, c, local) {
                        return Some(hit);
                    }
                }
                None
            }
            NodeKind::Artboard => None,
            NodeKind::Shape | NodeKind::Path => {
                let path = if node.kind == NodeKind::Shape {
                    crate::shapes::shape_path(doc, node)?
                } else {
                    crate::shapes::parse_path_data(&doc.param_str(node, "d", ""))?
                };
                let b = path.bounds();
                let sw = doc.param_f64(node, "stroke-width", 0.0).max(3.0);
                let pad = sw / 2.0 + 2.0;
                let inside = local.x >= (b.left() as f64 - pad)
                    && local.x <= (b.right() as f64 + pad)
                    && local.y >= (b.top() as f64 - pad)
                    && local.y <= (b.bottom() as f64 + pad);
                if !inside {
                    return None;
                }
                // precise test: filled shapes use containment; lines use bbox
                let kind = doc.param_str(node, "shape", "rect");
                if node.kind == NodeKind::Shape && (kind == "line" || kind == "arrow") {
                    return Some(id);
                }
                let fill = doc.param_str(node, "fill", "solid");
                if fill != "none" {
                    if path_contains(&path, local) {
                        return Some(id);
                    }
                    // near the stroke still counts
                }
                if sw > 0.0 && point_near_path(&path, local, sw / 2.0 + 2.0) {
                    return Some(id);
                }
                if fill == "none" && path_contains(&path, local) {
                    // clicking inside an unfilled shape: miss (paint.net-ish)
                    return None;
                }
                None
            }
            NodeKind::Bitmap => {
                let bm = node.bitmap.as_ref()?;
                let (cx, cy, cw, ch) = crate::render::bitmap_crop(doc, node, bm);
                let x = doc.param_f64(node, "x", 0.0);
                let y = doc.param_f64(node, "y", 0.0);
                let w = doc.param_f64(node, "w", cw as f64).max(1e-6);
                let h = doc.param_f64(node, "h", ch as f64).max(1e-6);
                // normalized position handles crop + non-destructive scale
                let u = (local.x - x) / w;
                let v = (local.y - y) / h;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    return None;
                }
                // hit only where paint exists — a transparent paint layer
                // must not swallow clicks meant for nodes underneath
                let px = (cx + (u * cw as f64) as u32).min(bm.width - 1);
                let py = (cy + (v * ch as f64) as u32).min(bm.height - 1);
                if bm.get_pixel(px, py)[3] < 8 {
                    return None;
                }
                Some(id)
            }
            NodeKind::StrokeSet => {
                let b = crate::render::strokes_bounds(node)?;
                if b.contains(local) {
                    Some(id)
                } else {
                    None
                }
            }
            NodeKind::GradientFill | NodeKind::Text | NodeKind::Reference => {
                let b = self.node_bounds(doc, id)?;
                // node_bounds already includes modifiers; test in parent space
                if b.contains(p) {
                    Some(id)
                } else {
                    None
                }
            }
        }
    }
}

/// Even-odd containment via ray casting over flattened path segments.
fn path_contains(path: &tiny_skia::Path, p: Vec2) -> bool {
    let mut crossings = 0;
    let mut last = (0.0f32, 0.0f32);
    let mut start = last;
    let test = |a: (f32, f32), b: (f32, f32)| -> bool {
        let (px, py) = (p.x as f32, p.y as f32);
        (a.1 > py) != (b.1 > py) && px < (b.0 - a.0) * (py - a.1) / (b.1 - a.1) + a.0
    };
    for seg in path.segments() {
        use tiny_skia::PathSegment::*;
        match seg {
            MoveTo(pt) => {
                last = (pt.x, pt.y);
                start = last;
            }
            LineTo(pt) => {
                if test(last, (pt.x, pt.y)) {
                    crossings += 1;
                }
                last = (pt.x, pt.y);
            }
            QuadTo(c, pt) => {
                // flatten with a few steps
                let mut prev = last;
                for i in 1..=8 {
                    let t = i as f32 / 8.0;
                    let x = (1.0 - t) * (1.0 - t) * last.0 + 2.0 * (1.0 - t) * t * c.x + t * t * pt.x;
                    let y = (1.0 - t) * (1.0 - t) * last.1 + 2.0 * (1.0 - t) * t * c.y + t * t * pt.y;
                    if test(prev, (x, y)) {
                        crossings += 1;
                    }
                    prev = (x, y);
                }
                last = (pt.x, pt.y);
            }
            CubicTo(c1, c2, pt) => {
                let mut prev = last;
                for i in 1..=12 {
                    let t = i as f32 / 12.0;
                    let mt = 1.0 - t;
                    let x = mt * mt * mt * last.0
                        + 3.0 * mt * mt * t * c1.x
                        + 3.0 * mt * t * t * c2.x
                        + t * t * t * pt.x;
                    let y = mt * mt * mt * last.1
                        + 3.0 * mt * mt * t * c1.y
                        + 3.0 * mt * t * t * c2.y
                        + t * t * t * pt.y;
                    if test(prev, (x, y)) {
                        crossings += 1;
                    }
                    prev = (x, y);
                }
                last = (pt.x, pt.y);
            }
            Close => {
                if test(last, start) {
                    crossings += 1;
                }
                last = start;
            }
        }
    }
    crossings % 2 == 1
}

/// Distance check against flattened path segments.
fn point_near_path(path: &tiny_skia::Path, p: Vec2, dist: f64) -> bool {
    let mut last: Option<(f32, f32)> = None;
    let mut start: Option<(f32, f32)> = None;
    let near = |a: (f32, f32), b: (f32, f32)| -> bool {
        seg_distance(p, Vec2::new(a.0 as f64, a.1 as f64), Vec2::new(b.0 as f64, b.1 as f64)) <= dist
    };
    for seg in path.segments() {
        use tiny_skia::PathSegment::*;
        match seg {
            MoveTo(pt) => {
                last = Some((pt.x, pt.y));
                start = last;
            }
            LineTo(pt) => {
                if let Some(l) = last {
                    if near(l, (pt.x, pt.y)) {
                        return true;
                    }
                }
                last = Some((pt.x, pt.y));
            }
            QuadTo(c, pt) => {
                if let Some(l) = last {
                    let mut prev = l;
                    for i in 1..=8 {
                        let t = i as f32 / 8.0;
                        let x = (1.0 - t) * (1.0 - t) * l.0 + 2.0 * (1.0 - t) * t * c.x + t * t * pt.x;
                        let y = (1.0 - t) * (1.0 - t) * l.1 + 2.0 * (1.0 - t) * t * c.y + t * t * pt.y;
                        if near(prev, (x, y)) {
                            return true;
                        }
                        prev = (x, y);
                    }
                }
                last = Some((pt.x, pt.y));
            }
            CubicTo(c1, c2, pt) => {
                if let Some(l) = last {
                    let mut prev = l;
                    for i in 1..=12 {
                        let t = i as f32 / 12.0;
                        let mt = 1.0 - t;
                        let x = mt * mt * mt * l.0
                            + 3.0 * mt * mt * t * c1.x
                            + 3.0 * mt * t * t * c2.x
                            + t * t * t * pt.x;
                        let y = mt * mt * mt * l.1
                            + 3.0 * mt * mt * t * c1.y
                            + 3.0 * mt * t * t * c2.y
                            + t * t * t * pt.y;
                        if near(prev, (x, y)) {
                            return true;
                        }
                        prev = (x, y);
                    }
                }
                last = Some((pt.x, pt.y));
            }
            Close => {
                if let (Some(l), Some(s)) = (last, start) {
                    if near(l, s) {
                        return true;
                    }
                }
                last = start;
            }
        }
    }
    false
}

fn seg_distance(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    if len2 < 1e-12 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2).clamp(0.0, 1.0);
    p.distance(Vec2::new(a.x + ab.x * t, a.y + ab.y * t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed_document::BlobStore;
    use std::collections::BTreeMap;

    fn doc_with_rect() -> (Document, NodeId, Engine) {
        let blobs = BlobStore::default();
        let mut doc = Document::with_artboard(ed_core::ActorId(1), "t", 400.0, 300.0, &blobs);
        let ab = doc.artboards()[0];
        doc.begin_txn("add");
        let mut params = BTreeMap::new();
        params.insert("shape".into(), ed_core::Value::Str("rect".into()));
        params.insert("x".into(), ed_core::Value::F64(50.0));
        params.insert("y".into(), ed_core::Value::F64(50.0));
        params.insert("w".into(), ed_core::Value::F64(100.0));
        params.insert("h".into(), ed_core::Value::F64(80.0));
        let id = doc.create_node(NodeKind::Shape, Some(ab), params, &blobs).unwrap();
        doc.commit_txn();
        (doc, id, Engine::new())
    }

    #[test]
    fn hits_shape_and_misses_empty_space() {
        let (doc, id, engine) = doc_with_rect();
        assert_eq!(engine.hit_test(&doc, Vec2::new(100.0, 90.0), false), Some(id));
        assert_eq!(engine.hit_test(&doc, Vec2::new(300.0, 250.0), false), None);
        assert_eq!(engine.hit_test(&doc, Vec2::new(-50.0, -50.0), false), None);
    }

    #[test]
    fn respects_hidden_and_locked(){
        let (mut doc, id, engine) = doc_with_rect();
        let blobs = BlobStore::default();
        doc.apply_txn(
            "hide",
            ed_document::OpKind::ParamSet {
                node_id: id,
                path: "visible".into(),
                value: ed_core::Value::Bool(false),
                prev: None,
            },
            &blobs,
        )
        .unwrap();
        assert_eq!(engine.hit_test(&doc, Vec2::new(100.0, 90.0), false), None);
    }

    #[test]
    fn transform_modifier_moves_hit_area() {
        let (mut doc, id, engine) = doc_with_rect();
        let blobs = BlobStore::default();
        let mid = doc.mint_modifier_id();
        doc.apply_txn(
            "transform",
            ed_document::OpKind::ModifierAttach {
                node_id: id,
                modifier: ed_document::Modifier {
                    id: mid,
                    kind: "transform".into(),
                    enabled: true,
                    params: BTreeMap::from([
                        ("tx".into(), ed_core::Value::F64(200.0)),
                        ("ty".into(), ed_core::Value::F64(0.0)),
                    ]),
                },
                index: 0,
            },
            &blobs,
        )
        .unwrap();
        assert_eq!(engine.hit_test(&doc, Vec2::new(100.0, 90.0), false), None, "old spot empty");
        assert_eq!(engine.hit_test(&doc, Vec2::new(300.0, 90.0), false), Some(id), "moved spot hits");
    }
}
