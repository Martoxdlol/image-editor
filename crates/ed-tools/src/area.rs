//! Area cut/copy/move (spec §10.3 pixel-selection rows, §2.3 selections
//! as scope). Two cut semantics live in the editor:
//!
//! - **Object cut**: no pixel selection active → whole nodes (fragment +
//!   delete). Lives in `session.rs`.
//! - **Area cut** (this module): a pixel selection is active → the region
//!   is copied as a merged bitmap fragment and every affected object gets
//!   a hole: Bitmaps destructively via tile deltas (spec §2.5), everything
//!   else non-destructively via an inverted `sel-mask` modifier that shows
//!   up — and can be toggled/removed per object — in the Properties panel.

use crate::fragment::Fragment;
use crate::session::{demultiply, Session};
use ed_core::{NodeId, Rect, Value, Vec2};
use ed_document::{BitmapData, Modifier, NodeKind, OpKind, PixelSelection};
use std::collections::BTreeMap;

/// Which objects an area operation touches (tool option `sel.affect`).
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum AreaScope {
    All,
    SelectedOnly,
    BitmapsOnly,
}

impl AreaScope {
    pub fn from_param(s: &str) -> AreaScope {
        match s {
            "selected" => AreaScope::SelectedOnly,
            "bitmaps" => AreaScope::BitmapsOnly,
            _ => AreaScope::All,
        }
    }
}

const MAX_AREA_DIM: f64 = 8192.0;

impl Session {
    fn area_scope(&self) -> AreaScope {
        AreaScope::from_param(&self.tool_str("sel.affect", "all"))
    }

    /// Objects an area op affects: direct children of artboards (and
    /// shared-space nodes) intersecting the selection — visible, unlocked,
    /// filtered by scope.
    fn affected_nodes(&self, bounds: &Rect, scope: AreaScope) -> Vec<NodeId> {
        let doc = self.doc();
        let mut out = Vec::new();
        for &root in doc.children_of(None) {
            let Some(rn) = doc.node(root) else { continue };
            let candidates: Vec<NodeId> = if rn.kind == NodeKind::Artboard {
                doc.children_of(Some(root)).to_vec()
            } else {
                vec![root]
            };
            for id in candidates {
                let Some(n) = doc.node(id) else { continue };
                if !n.visible() || n.locked() {
                    continue;
                }
                match scope {
                    AreaScope::SelectedOnly if !doc.selected_nodes.contains(&id) => continue,
                    AreaScope::BitmapsOnly if n.kind != NodeKind::Bitmap => continue,
                    _ => {}
                }
                if let Some(b) = self.engine.node_bounds(doc, id) {
                    if b.intersects(bounds) {
                        out.push(id);
                    }
                }
            }
        }
        out
    }

    fn selection_region(&self) -> Option<(PixelSelection, Rect)> {
        let sel = self.doc().pixel_selection.clone()?;
        if sel.is_empty() {
            return None;
        }
        let b = sel.bounds();
        if b.w < 1.0 || b.h < 1.0 || b.w > MAX_AREA_DIM || b.h > MAX_AREA_DIM {
            return None;
        }
        Some((sel, Rect::new(b.x.floor(), b.y.floor(), b.w.ceil(), b.h.ceil())))
    }

    /// Merged composite of `nodes` inside the region, multiplied by the
    /// selection coverage. Straight RGBA at 1:1 doc resolution.
    fn region_composite(
        &mut self,
        sel: &PixelSelection,
        bounds: &Rect,
        nodes: &[NodeId],
    ) -> Option<(u32, u32, Vec<u8>)> {
        let w = bounds.w as u32;
        let h = bounds.h as u32;
        let mut pm = tiny_skia::Pixmap::new(w, h)?;
        let m = ed_core::Mat3::translate(Vec2::new(-bounds.x, -bounds.y));
        {
            let doc = &self.docs[self.active].doc;
            for &id in nodes {
                self.engine.render_node_standalone(doc, &self.blobs, id, &mut pm, &m);
            }
        }
        let coverage = sel.rasterize(bounds.x as i64, bounds.y as i64, w, h);
        let mut rgba = demultiply(&pm);
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            px[3] = ((px[3] as u32 * coverage[i] as u32) / 255) as u8;
        }
        Some((w, h, rgba))
    }

    /// Region composite for the system-clipboard PNG flavor.
    pub fn area_composite_public(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let (sel, bounds) = self.selection_region()?;
        let nodes = self.affected_nodes(&bounds, self.area_scope());
        self.region_composite(&sel, &bounds, &nodes)
    }

    /// Copy Merged for the active pixel selection (spec §10.3): the region
    /// composite becomes a Bitmap fragment on the internal clipboard.
    pub fn area_copy(&mut self) -> bool {
        let Some((sel, bounds)) = self.selection_region() else { return false };
        let nodes = self.affected_nodes(&bounds, self.area_scope());
        let Some((w, h, rgba)) = self.region_composite(&sel, &bounds, &nodes) else {
            return false;
        };
        let mut node = ed_document::Node::new(NodeId::new(0, 1), NodeKind::Bitmap);
        node.params.insert("name".into(), Value::Str("Cut region".into()));
        node.params.insert("x".into(), Value::F64(bounds.x));
        node.params.insert("y".into(), Value::F64(bounds.y));
        let bm = BitmapData::from_rgba(w, h, &rgba);
        let mut tiles = BTreeMap::new();
        for (&(tx, ty), data) in &bm.tiles {
            tiles.insert(format!("{}/{tx},{ty}", node.id), data.clone());
        }
        node.bitmap = Some(BitmapData::new(w, h));
        self.clipboard = Some(Fragment {
            roots: vec![node.id],
            nodes: vec![node],
            palette: Vec::new(),
            variables: BTreeMap::new(),
            tiles,
            blobs: BTreeMap::new(),
            bounds: (bounds.x, bounds.y, bounds.w, bounds.h),
            source_doc: self.doc().name.clone(),
        });
        true
    }

    /// Cut the selected area out of every affected object (one txn):
    /// bitmaps lose pixels via tile deltas; everything else gains an
    /// inverted sel-mask modifier (a non-destructive hole).
    pub fn area_cut(&mut self, to_clipboard: bool, label: &str) -> bool {
        let Some((sel, bounds)) = self.selection_region() else { return false };
        if self.affected_nodes(&bounds, self.area_scope()).is_empty() {
            return false;
        }
        if to_clipboard {
            self.area_copy();
        }
        self.doc_mut().begin_txn(label);
        self.area_cut_into_open_txn(&sel, &bounds);
        self.doc_mut().commit_txn();
        self.dirty_frame();
        true
    }

    /// Paint-style area move: lift the selected region into a floating
    /// Bitmap node (sources get the hole), returning the new node id. The
    /// caller starts a move drag on it. One "Lift selection" txn.
    pub fn lift_area(&mut self) -> Option<NodeId> {
        let (sel, bounds) = self.selection_region()?;
        let nodes = self.affected_nodes(&bounds, self.area_scope());
        if nodes.is_empty() {
            return None;
        }
        let (w, h, rgba) = self.region_composite(&sel, &bounds, &nodes)?;
        if rgba.iter().skip(3).step_by(4).all(|&a| a == 0) {
            return None; // nothing but empty pixels under the selection
        }
        let parent = self.engine.artboard_at(self.doc(), bounds.center());

        // cut holes + create the floating node in ONE txn
        self.doc_mut().begin_txn("Lift selection");
        self.area_cut_into_open_txn(&sel, &bounds);
        let blobs = std::mem::take(&mut self.blobs);
        let float_id = {
            let doc = &mut self.docs[self.active].doc;
            let mut params = BTreeMap::new();
            params.insert("name".into(), Value::Str("Floating selection".into()));
            params.insert("x".into(), Value::F64(bounds.x));
            params.insert("y".into(), Value::F64(bounds.y));
            let id = doc.create_node(NodeKind::Bitmap, parent, params, &blobs).ok();
            if let Some(id) = id {
                if let Some(n) = doc.nodes.get_mut(&id) {
                    n.bitmap = Some(BitmapData::from_rgba(w, h, &rgba));
                }
                doc.selected_nodes = vec![id];
            }
            doc.commit_txn();
            id
        };
        self.blobs = blobs;
        // the selection border stays and follows the floating object
        self.floating = float_id;
        self.dirty_frame();
        float_id
    }

    /// Hole-cutting shared by `area_cut` and `lift_area`; assumes an open
    /// txn and does NOT commit it.
    fn area_cut_into_open_txn(&mut self, sel: &PixelSelection, bounds: &Rect) {
        let nodes = self.affected_nodes(bounds, self.area_scope());
        let mask_bytes =
            sel.rasterize(bounds.x as i64, bounds.y as i64, bounds.w as u32, bounds.h as u32);
        let mask_hash = self.blobs.put(mask_bytes);
        let mut blobs = std::mem::take(&mut self.blobs);
        {
            let doc = &mut self.docs[self.active].doc;
            for id in nodes {
                let Some(n) = doc.node(id) else { continue };
                if n.kind == NodeKind::Bitmap {
                    clear_bitmap_region(doc, id, sel, bounds, &mut blobs);
                } else {
                    let mid = doc.mint_modifier_id();
                    let mut params = BTreeMap::new();
                    params.insert("mask".into(), Value::Blob(mask_hash));
                    params.insert("x".into(), Value::F64(bounds.x));
                    params.insert("y".into(), Value::F64(bounds.y));
                    params.insert("w".into(), Value::F64(bounds.w));
                    params.insert("h".into(), Value::F64(bounds.h));
                    params.insert("invert".into(), Value::Bool(true));
                    let index = doc.node(id).map(|n| n.modifiers.len()).unwrap_or(0);
                    let _ = doc.apply(
                        OpKind::ModifierAttach {
                            node_id: id,
                            modifier: Modifier {
                                id: mid,
                                kind: "sel-mask".into(),
                                enabled: true,
                                params,
                            },
                            index,
                        },
                        &blobs,
                    );
                }
            }
        }
        self.blobs = blobs;
    }
}

/// Erase a selection region from a bitmap as recorded tile deltas
/// (spec §10.3 "Cut clears region"). Alpha scales by 1 − coverage.
fn clear_bitmap_region(
    doc: &mut ed_document::Document,
    id: NodeId,
    sel: &PixelSelection,
    bounds: &Rect,
    blobs: &mut ed_document::BlobStore,
) {
    let (offset, scale) = crate::tools::bitmap_view(doc, id);
    let (sx, sy) = (scale.x, scale.y);
    let Some(n) = doc.node(id) else { return };
    let Some(bm) = n.bitmap.as_ref() else { return };
    let bw = bm.width;
    let bh = bm.height;
    // intersect region in bitmap-local pixels
    let x0 = (((bounds.x - offset.x) / sx).floor().max(0.0)) as u32;
    let y0 = (((bounds.y - offset.y) / sy).floor().max(0.0)) as u32;
    let x1 = ((((bounds.x + bounds.w) - offset.x) / sx).ceil().min(bw as f64)) as u32;
    let y1 = ((((bounds.y + bounds.h) - offset.y) / sy).ceil().min(bh as f64)) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    // capture before-tiles for the touched region
    let t = ed_document::TILE_SIZE;
    let mut before: BTreeMap<(u32, u32), Option<Vec<u8>>> = BTreeMap::new();
    for ty in (y0 / t)..=((y1 - 1) / t) {
        for tx in (x0 / t)..=((x1 - 1) / t) {
            before.insert((tx, ty), bm.tiles.get(&(tx, ty)).cloned());
        }
    }
    let cov = sel.rasterize_scaled(
        offset.x + x0 as f64 * sx,
        offset.y + y0 as f64 * sy,
        sx,
        sy,
        x1 - x0,
        y1 - y0,
    );
    if let Some(bm) = doc.bitmap_mut(id) {
        for y in y0..y1 {
            for x in x0..x1 {
                let c = cov[((y - y0) * (x1 - x0) + (x - x0)) as usize] as u32;
                if c == 0 {
                    continue;
                }
                let mut px = bm.get_pixel(x, y);
                if px[3] == 0 {
                    continue;
                }
                px[3] = ((px[3] as u32 * (255 - c)) / 255) as u8;
                bm.set_pixel(x, y, px);
            }
        }
    }
    let _ = doc.commit_paint(id, &before, blobs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed_document::SelGeom;

    /// Session with one artboard, a shape, and a painted bitmap.
    fn setup() -> (Session, NodeId, NodeId) {
        let mut s = Session::new();
        let ab = s.doc().artboards()[0];
        let blobs = std::mem::take(&mut s.blobs);
        let (shape, bmp) = {
            let doc = s.doc_mut();
            doc.begin_txn("build");
            let mut params = BTreeMap::new();
            params.insert("shape".into(), Value::Str("rect".into()));
            params.insert("x".into(), Value::F64(50.0));
            params.insert("y".into(), Value::F64(50.0));
            params.insert("w".into(), Value::F64(200.0));
            params.insert("h".into(), Value::F64(200.0));
            let shape = doc.create_node(NodeKind::Shape, Some(ab), params, &blobs).unwrap();
            let mut bparams = BTreeMap::new();
            bparams.insert("x".into(), Value::F64(0.0));
            bparams.insert("y".into(), Value::F64(0.0));
            let bmp = doc.create_node(NodeKind::Bitmap, Some(ab), bparams, &blobs).unwrap();
            doc.commit_txn();
            (shape, bmp)
        };
        s.blobs = blobs;
        {
            let bm = s.doc_mut().bitmap_mut(bmp).unwrap();
            bm.width = 400;
            bm.height = 400;
            for y in 0..400 {
                for x in 0..400 {
                    bm.set_pixel(x, y, [10, 200, 30, 255]);
                }
            }
        }
        (s, shape, bmp)
    }

    #[test]
    fn area_cut_clears_bitmap_and_masks_shape() {
        let (mut s, shape, bmp) = setup();
        s.doc_mut().pixel_selection = Some(PixelSelection::single(SelGeom::Rect {
            rect: Rect::new(100.0, 100.0, 80.0, 80.0),
        }));

        assert!(s.area_cut(true, "Cut area"));

        // bitmap: hole cleared, outside intact (tile deltas)
        let bm = s.doc().node(bmp).unwrap().bitmap.as_ref().unwrap();
        assert_eq!(bm.get_pixel(140, 140)[3], 0, "inside the cut is transparent");
        assert_eq!(bm.get_pixel(20, 20)[3], 255, "outside untouched");

        // shape: non-destructive sel-mask modifier attached
        let n = s.doc().node(shape).unwrap();
        assert_eq!(n.modifiers.len(), 1);
        assert_eq!(n.modifiers[0].kind, "sel-mask");
        assert!(matches!(n.modifiers[0].params.get("mask"), Some(Value::Blob(_))));

        // clipboard: merged region fragment ready to paste
        assert!(s.clipboard.is_some());
        let frag = s.clipboard.as_ref().unwrap();
        assert_eq!(frag.nodes.len(), 1);
        assert!(!frag.tiles.is_empty(), "region pixels travel with the fragment");

        // single undo restores both objects
        let blobs = std::mem::take(&mut s.blobs);
        s.doc_mut().undo(&blobs);
        s.blobs = blobs;
        let bm = s.doc().node(bmp).unwrap().bitmap.as_ref().unwrap();
        assert_eq!(bm.get_pixel(140, 140)[3], 255, "undo restores bitmap pixels");
        assert!(s.doc().node(shape).unwrap().modifiers.is_empty(), "undo detaches the mask");
    }

    #[test]
    fn scope_bitmaps_only_spares_shapes() {
        let (mut s, shape, bmp) = setup();
        s.tool_params.insert("sel.affect".into(), Value::Str("bitmaps".into()));
        s.doc_mut().pixel_selection = Some(PixelSelection::single(SelGeom::Rect {
            rect: Rect::new(100.0, 100.0, 50.0, 50.0),
        }));
        assert!(s.area_cut(false, "Delete area"));
        assert!(s.doc().node(shape).unwrap().modifiers.is_empty(), "shape untouched");
        let bm = s.doc().node(bmp).unwrap().bitmap.as_ref().unwrap();
        assert_eq!(bm.get_pixel(120, 120)[3], 0, "bitmap still cut");
    }

    #[test]
    fn sel_mask_survives_save_load() {
        let (mut s, shape, _bmp) = setup();
        s.doc_mut().pixel_selection = Some(PixelSelection::single(SelGeom::Rect {
            rect: Rect::new(100.0, 100.0, 40.0, 40.0),
        }));
        assert!(s.area_cut(false, "Cut area"));
        let bytes = s.save_myed().unwrap();
        s.open_myed(&bytes, "reload.myed").unwrap();
        let n = s.doc().node(shape).expect("shape survives roundtrip");
        assert_eq!(n.modifiers[0].kind, "sel-mask");
        let Some(Value::Blob(h)) = n.modifiers[0].params.get("mask") else {
            panic!("mask param missing")
        };
        assert!(s.blobs.contains(*h), "mask blob persisted inside .myed");
    }

    #[test]
    fn lift_area_creates_floating_bitmap() {
        let (mut s, _shape, bmp) = setup();
        s.doc_mut().pixel_selection = Some(PixelSelection::single(SelGeom::Rect {
            rect: Rect::new(100.0, 100.0, 60.0, 60.0),
        }));
        let float_id = s.lift_area().expect("lift produced a node");
        assert_ne!(float_id, bmp);
        let f = s.doc().node(float_id).unwrap();
        assert_eq!(f.kind, NodeKind::Bitmap);
        let fbm = f.bitmap.as_ref().unwrap();
        assert_eq!(fbm.get_pixel(30, 30), [10, 200, 30, 255], "lifted pixels present");
        // source has the hole
        let bm = s.doc().node(bmp).unwrap().bitmap.as_ref().unwrap();
        assert_eq!(bm.get_pixel(130, 130)[3], 0);
        // the selection border stays (it travels with the floating node)
        assert!(s.has_pixel_selection());
        assert_eq!(s.floating, Some(float_id));
        assert_eq!(s.doc().selected_nodes, vec![float_id]);
    }
}
