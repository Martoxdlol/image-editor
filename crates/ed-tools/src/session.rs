//! The editor session (spec §12.1): multi-document registry with shared
//! blob store, engine, fonts and internal clipboard. All commands enter
//! here as JSON; all UI state leaves here as JSON mirrors.

use crate::fragment::{self, Fragment, PasteOptions};
use crate::tools::{DragState, ToolKind};
use ed_core::{ActorId, Color, NodeId, Rect, Value, Vec2};
use ed_document::{doc::BlobStore, Document, Modifier, NodeKind, OpKind};
use ed_engine::{Engine, Overlay, View};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

pub struct DocState {
    pub doc: Document,
    pub view: View,
}

pub struct Session {
    pub blobs: BlobStore,
    pub docs: Vec<DocState>,
    pub active: usize,
    pub engine: Engine,
    pub clipboard: Option<Fragment>,
    pub tool: ToolKind,
    pub tool_params: BTreeMap<String, Value>,
    pub fg: Color,
    pub bg: Color,
    pub drag: Option<DragState>,
    pub pen: crate::tools::PenState,
    pub overlays: Vec<Overlay>,
    pub cursor: String,
    pub pointer_doc: Vec2,
    pub viewport: (u32, u32),
    /// Bumped whenever a new frame is needed.
    pub frame_rev: u64,
    last_state_rev: u64,
    doc_counter: u32,
    /// Live param edit in progress (slider drag): (node, path, pre-drag
    /// value). Previews mutate the doc directly; the commit records ONE
    /// txn whose prev is the original — same rule as canvas drags (§3.3).
    param_preview: Option<(NodeId, String, Value)>,
    /// Node created by the last area lift — a repeated drag inside the
    /// selection keeps moving it instead of re-cutting.
    pub floating: Option<NodeId>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        let blobs = BlobStore::default();
        let doc = Document::with_artboard(ActorId(1), "Untitled 1", 4000.0, 3000.0, &blobs);
        Session {
            blobs,
            docs: vec![DocState { doc, view: View::default() }],
            active: 0,
            engine: Engine::new(),
            clipboard: None,
            tool: ToolKind::Select,
            tool_params: default_tool_params(),
            fg: Color::from_hex("#1e88e5").unwrap(),
            bg: Color::WHITE,
            drag: None,
            pen: crate::tools::PenState::default(),
            overlays: Vec::new(),
            cursor: "default".into(),
            pointer_doc: Vec2::ZERO,
            viewport: (800, 600),
            frame_rev: 1,
            last_state_rev: 0,
            doc_counter: 1,
            param_preview: None,
            floating: None,
        }
    }

    /// Commit any in-flight param preview as a single transaction. Called
    /// before anything that reads or rewinds history (undo, tab switch).
    fn flush_param_preview(&mut self) {
        let Some((id, path, orig)) = self.param_preview.take() else { return };
        let current = self.doc().node(id).and_then(|n| n.get_param(&path));
        let Some(current) = current else { return };
        if current == orig {
            return;
        }
        // restore the original silently, then record the real txn so its
        // stored prev is the true pre-drag value
        self.doc_mut().preview_param(id, &path, orig);
        let blobs = std::mem::take(&mut self.blobs);
        let _ = self.doc_mut().apply_txn(
            &format!("Set {path}"),
            OpKind::ParamSet { node_id: id, path, value: current, prev: None },
            &blobs,
        );
        self.blobs = blobs;
        self.dirty_frame();
    }

    pub fn doc(&self) -> &Document {
        &self.docs[self.active].doc
    }

    pub fn doc_mut(&mut self) -> &mut Document {
        self.frame_rev += 1;
        &mut self.docs[self.active].doc
    }

    pub fn view(&self) -> &View {
        &self.docs[self.active].view
    }

    pub fn view_mut(&mut self) -> &mut View {
        self.frame_rev += 1;
        &mut self.docs[self.active].view
    }

    pub fn dirty_frame(&mut self) {
        self.frame_rev += 1;
    }

    // ------------------------------------------------------------ params

    pub fn tool_f64(&self, key: &str, default: f64) -> f64 {
        self.tool_params.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }

    pub fn tool_bool(&self, key: &str, default: bool) -> bool {
        self.tool_params.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub fn tool_str(&self, key: &str, default: &str) -> String {
        self.tool_params
            .get(key)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| default.to_string())
    }

    // ------------------------------------------------------------ commands

    pub fn command(&mut self, cmd_json: &str) -> Result<serde_json::Value, String> {
        let cmd: Command = serde_json::from_str(cmd_json).map_err(|e| format!("bad command: {e}"))?;
        self.dispatch(cmd)
    }

    fn dispatch(&mut self, cmd: Command) -> Result<serde_json::Value, String> {
        use Command::*;
        match cmd {
            Pointer(ev) => {
                self.pointer(ev);
                Ok(json!(null))
            }
            SetTool { tool } => {
                self.cancel_drag();
                self.tool = tool;
                self.overlays.clear();
                self.dirty_frame();
                Ok(json!(null))
            }
            SetToolParam { key, value } => {
                self.tool_params.insert(key, value);
                Ok(json!(null))
            }
            SetFg { color } => {
                self.fg = Color::from_hex(&color).ok_or("bad color")?;
                Ok(json!(null))
            }
            SetBg { color } => {
                self.bg = Color::from_hex(&color).ok_or("bad color")?;
                Ok(json!(null))
            }
            NewDoc { width, height, name, background, bg_color, dpi } => {
                self.doc_counter += 1;
                let name = name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| format!("Untitled {}", self.doc_counter));
                let mut doc = Document::with_artboard(
                    ActorId(1),
                    &name,
                    width.clamp(1.0, ed_document::MAX_ARTBOARD_DIM),
                    height.clamp(1.0, ed_document::MAX_ARTBOARD_DIM),
                    &self.blobs,
                );
                // initial artboard settings are part of the fresh document,
                // not history
                let ab = doc.artboards()[0];
                if let Some(bg) = background {
                    doc.preview_param(ab, "background", Value::Str(bg));
                }
                if let Some(c) = bg_color.and_then(|c| Color::from_hex(&c)) {
                    doc.preview_param(ab, "bg-color", Value::Color(c));
                }
                if let Some(d) = dpi {
                    doc.preview_param(ab, "dpi", Value::F64(d.clamp(1.0, 1200.0)));
                }
                doc.dirty = false;
                self.docs.push(DocState { doc, view: View::default() });
                self.active = self.docs.len() - 1;
                self.fit_view();
                Ok(json!(null))
            }
            SwitchDoc { index } => {
                if index < self.docs.len() {
                    self.cancel_drag();
                    self.flush_param_preview();
                    self.active = index;
                    self.dirty_frame();
                }
                Ok(json!(null))
            }
            CloseDoc { index } => {
                if self.docs.len() > 1 && index < self.docs.len() {
                    self.docs.remove(index);
                    if self.active >= self.docs.len() {
                        self.active = self.docs.len() - 1;
                    }
                    self.dirty_frame();
                }
                Ok(json!(null))
            }
            RenameDoc { name } => {
                self.doc_mut().name = name;
                Ok(json!(null))
            }
            Undo => {
                self.cancel_drag();
                self.flush_param_preview();
                let blobs = std::mem::take(&mut self.blobs);
                self.doc_mut().undo(&blobs);
                self.blobs = blobs;
                self.dirty_frame();
                Ok(json!(null))
            }
            Redo => {
                self.cancel_drag();
                self.flush_param_preview();
                let blobs = std::mem::take(&mut self.blobs);
                self.doc_mut().redo(&blobs);
                self.blobs = blobs;
                self.dirty_frame();
                Ok(json!(null))
            }
            SelectNodes { ids, toggle } => {
                let parsed: Vec<NodeId> = ids.iter().filter_map(|s| parse_node_id(s)).collect();
                let doc = self.doc_mut();
                if toggle {
                    for id in parsed {
                        if let Some(pos) = doc.selected_nodes.iter().position(|&x| x == id) {
                            doc.selected_nodes.remove(pos);
                        } else {
                            doc.selected_nodes.push(id);
                        }
                    }
                } else {
                    doc.selected_nodes = parsed;
                }
                Ok(json!(null))
            }
            DeleteSelection => {
                self.delete_selection();
                Ok(json!(null))
            }
            DuplicateSelection => {
                self.duplicate_selection()?;
                Ok(json!(null))
            }
            GroupSelection => {
                self.group_selection()?;
                Ok(json!(null))
            }
            UngroupSelection => {
                self.ungroup_selection()?;
                Ok(json!(null))
            }
            // Two cut semantics (§10.3): an active pixel selection means
            // the AREA is the subject; otherwise whole objects are.
            Copy => {
                if self.has_pixel_selection() {
                    self.area_copy();
                } else {
                    self.copy_selection();
                }
                Ok(json!(null))
            }
            Cut => {
                if self.has_pixel_selection() {
                    self.area_cut(true, "Cut area");
                } else {
                    self.copy_selection();
                    self.delete_selection();
                }
                Ok(json!(null))
            }
            Paste { in_place } => {
                self.paste_clipboard(in_place)?;
                Ok(json!(null))
            }
            PreviewParam { node, path, value } => {
                let id = parse_node_id(&node).ok_or("bad node id")?;
                let value = json_to_value(&value).ok_or("bad value")?;
                // a preview of a different param commits the previous one
                match &self.param_preview {
                    Some((pn, pp, _)) if *pn == id && *pp == path => {}
                    Some(_) => self.flush_param_preview(),
                    None => {}
                }
                if self.param_preview.is_none() {
                    if let Some(orig) = self.doc().node(id).and_then(|n| n.get_param(&path)) {
                        self.param_preview = Some((id, path.clone(), orig));
                    }
                }
                self.doc_mut().preview_param(id, &path, value);
                Ok(json!(null))
            }
            SetParam { node, path, value } => {
                let id = parse_node_id(&node).ok_or("bad node id")?;
                let value = json_to_value(&value).ok_or("bad value")?;
                // close an in-flight preview: restore the pre-drag original
                // so this txn's recorded prev is correct
                match self.param_preview.take() {
                    Some((pn, pp, orig)) if pn == id && pp == path => {
                        self.doc_mut().preview_param(id, &path, orig);
                    }
                    Some(other) => {
                        self.param_preview = Some(other);
                        self.flush_param_preview();
                    }
                    None => {}
                }
                // no-op commits (blur without change, drag back to start)
                // must not pollute history
                if self.doc().node(id).and_then(|n| n.get_param(&path)).as_ref() == Some(&value) {
                    self.dirty_frame();
                    return Ok(json!(null));
                }
                let label = format!("Set {path}");
                let blobs = std::mem::take(&mut self.blobs);
                let r = self.doc_mut().apply_txn(
                    &label,
                    OpKind::ParamSet { node_id: id, path, value, prev: None },
                    &blobs,
                );
                self.blobs = blobs;
                self.dirty_frame();
                r.map(|_| json!(null))
            }
            AddModifier { node, kind } => {
                let id = parse_node_id(&node).ok_or("bad node id")?;
                let mid = self.doc_mut().mint_modifier_id();
                let modifier = Modifier {
                    id: mid,
                    kind: kind.clone(),
                    enabled: true,
                    params: default_modifier_params(&kind, self.doc(), id),
                };
                let index = self.doc().node(id).map(|n| n.modifiers.len()).unwrap_or(0);
                let blobs = std::mem::take(&mut self.blobs);
                let r = self.doc_mut().apply_txn(
                    &format!("Add {kind}"),
                    OpKind::ModifierAttach { node_id: id, modifier, index },
                    &blobs,
                );
                self.blobs = blobs;
                self.dirty_frame();
                r.map(|_| json!(null))
            }
            RemoveModifier { node, id } => {
                let node = parse_node_id(&node).ok_or("bad node id")?;
                let blobs = std::mem::take(&mut self.blobs);
                let r = self.doc_mut().apply_txn(
                    "Remove modifier",
                    OpKind::ModifierDetach { node_id: node, modifier_id: id, removed: None },
                    &blobs,
                );
                self.blobs = blobs;
                self.dirty_frame();
                r.map(|_| json!(null))
            }
            ReorderModifier { node, id, index } => {
                let node = parse_node_id(&node).ok_or("bad node id")?;
                let blobs = std::mem::take(&mut self.blobs);
                let r = self.doc_mut().apply_txn(
                    "Reorder modifier",
                    OpKind::ModifierReorder {
                        node_id: node,
                        modifier_id: id,
                        new_index: index,
                        prev_index: 0,
                    },
                    &blobs,
                );
                self.blobs = blobs;
                self.dirty_frame();
                r.map(|_| json!(null))
            }
            MoveNode { node, parent, index } => {
                let id = parse_node_id(&node).ok_or("bad node id")?;
                let parent = match parent {
                    Some(p) => Some(parse_node_id(&p).ok_or("bad parent id")?),
                    None => None,
                };
                // no-op guard: dropping into the slot the node already
                // occupies must not touch history
                if self.doc().node(id).map(|n| n.parent) == Some(parent) {
                    let siblings = self.doc().children_of(parent);
                    if let Some(cur) = siblings.iter().position(|&s| s == id) {
                        if index == cur || index == cur + 1 {
                            return Ok(json!(null));
                        }
                    }
                }
                let frac = self.doc().frac_for_insert(parent, index);
                let blobs = std::mem::take(&mut self.blobs);
                let r = self.doc_mut().apply_txn(
                    "Reorder",
                    OpKind::NodeMove {
                        node_id: id,
                        new_parent: parent,
                        frac_index: frac,
                        prev_parent: None,
                        prev_frac: String::new(),
                    },
                    &blobs,
                );
                self.blobs = blobs;
                self.dirty_frame();
                r.map(|_| json!(null))
            }
            SetView { zoom, pan_x, pan_y } => {
                let v = self.view_mut();
                if let Some(z) = zoom {
                    v.zoom = z.clamp(0.01, 64.0);
                }
                if let Some(x) = pan_x {
                    v.pan.x = x;
                }
                if let Some(y) = pan_y {
                    v.pan.y = y;
                }
                Ok(json!(null))
            }
            ZoomBy { factor, cx, cy } => {
                let v = self.view_mut();
                let anchor = v.screen_to_doc(Vec2::new(cx, cy));
                v.zoom = (v.zoom * factor).clamp(0.01, 64.0);
                v.pan = Vec2::new(anchor.x - cx / v.zoom, anchor.y - cy / v.zoom);
                Ok(json!(null))
            }
            FitView => {
                self.fit_view();
                Ok(json!(null))
            }
            SetPixelPreview { on } => {
                self.view_mut().pixel_preview = on;
                Ok(json!(null))
            }
            Resize { width, height } => {
                self.viewport = (width, height);
                self.dirty_frame();
                Ok(json!(null))
            }
            SetVariable { name, value } => {
                let value = value.and_then(|v| json_to_value(&v));
                let blobs = std::mem::take(&mut self.blobs);
                let r = self.doc_mut().apply_txn(
                    "Set variable",
                    OpKind::VariableSet { name, value, prev: None },
                    &blobs,
                );
                self.blobs = blobs;
                self.dirty_frame();
                r.map(|_| json!(null))
            }
            SetPalette { name, color } => {
                let value = color.and_then(|c| Color::from_hex(&c));
                let blobs = std::mem::take(&mut self.blobs);
                let r = self.doc_mut().apply_txn(
                    "Edit palette",
                    OpKind::PaletteSet { name, value, prev: None },
                    &blobs,
                );
                self.blobs = blobs;
                self.dirty_frame();
                r.map(|_| json!(null))
            }
            NewArtboard { width, height } => {
                self.new_artboard(width, height)?;
                Ok(json!(null))
            }
            ClearPixelSelection => {
                self.doc_mut().pixel_selection = None;
                self.floating = None;
                Ok(json!(null))
            }
            InvertPixelSelection => {
                let doc = self.doc_mut();
                if let Some(sel) = doc.pixel_selection.as_mut() {
                    sel.inverted = !sel.inverted;
                }
                Ok(json!(null))
            }
            SelectAll => {
                // select all pixels of the active artboard (spec Ctrl+A)
                let ab = self.doc().artboards().first().copied();
                if let Some(ab) = ab {
                    if let Some(rect) = self.doc().artboard_rect(ab) {
                        self.doc_mut().pixel_selection =
                            Some(ed_document::PixelSelection::single(ed_document::SelGeom::Rect {
                                rect,
                            }));
                    }
                }
                Ok(json!(null))
            }
            Key { key, mods } => {
                self.key(&key, mods);
                Ok(json!(null))
            }
            // context-menu helper: select the node under a screen point
            // without starting a drag (keeps existing multi-selection when
            // the point is already inside it)
            SelectAt { x, y } => {
                let p = self.view().screen_to_doc(Vec2::new(x, y));
                if let Some(id) = self.engine.hit_test(self.doc(), p, false) {
                    if !self.doc().selected_nodes.contains(&id) {
                        self.doc_mut().selected_nodes = vec![id];
                    }
                }
                self.dirty_frame();
                Ok(json!(null))
            }
            // history panel "revert to here" (spec §14 jump-to): walk
            // undo/redo until the target txn is the current state
            HistoryJump { id } => {
                self.cancel_drag();
                self.flush_param_preview();
                let target = ed_core::TxnId(id);
                let blobs = std::mem::take(&mut self.blobs);
                {
                    let doc = self.doc_mut();
                    let mut guard = 0;
                    if doc.undo_stack_contains(target) {
                        while doc.undo_top() != Some(target) && doc.can_undo() && guard < 10_000 {
                            doc.undo(&blobs);
                            guard += 1;
                        }
                    } else if doc.redo_stack_contains(target) {
                        while doc.undo_top() != Some(target) && doc.can_redo() && guard < 10_000 {
                            doc.redo(&blobs);
                            guard += 1;
                        }
                    }
                }
                self.blobs = blobs;
                self.dirty_frame();
                Ok(json!(null))
            }
            RasterizeSelection => {
                self.rasterize_selection()?;
                Ok(json!(null))
            }
            CropToSelection => {
                self.crop_to_selection()?;
                Ok(json!(null))
            }
            ResetCrop => {
                self.reset_crop()?;
                Ok(json!(null))
            }
            ConvertToPath => {
                self.convert_to_path()?;
                Ok(json!(null))
            }
        }
    }

    // ------------------------------------------------------------ edit ops

    pub fn has_pixel_selection(&self) -> bool {
        self.doc().pixel_selection.as_ref().map(|s| !s.is_empty()).unwrap_or(false)
    }

    pub fn delete_selection(&mut self) {
        let ids: Vec<NodeId> = self.doc().selected_nodes.clone();
        if ids.is_empty() {
            return;
        }
        let blobs = std::mem::take(&mut self.blobs);
        {
            let doc = self.doc_mut();
            doc.begin_txn("Delete");
            for id in ids {
                let _ = doc.apply(OpKind::NodeDelete { node_id: id }, &blobs);
            }
            doc.commit_txn();
            doc.selected_nodes.clear();
        }
        self.blobs = blobs;
        self.dirty_frame();
    }

    fn selection_bounds(&self) -> Rect {
        let mut acc = Rect::default();
        for &id in &self.doc().selected_nodes {
            if let Some(b) = self.engine.node_bounds(self.doc(), id) {
                acc = acc.union(&b);
            }
        }
        acc
    }

    pub fn copy_selection(&mut self) {
        let ids = self.doc().selected_nodes.clone();
        if ids.is_empty() {
            return;
        }
        let b = self.selection_bounds();
        self.clipboard =
            Some(fragment::copy_nodes(self.doc(), &ids, (b.x, b.y, b.w, b.h), &self.blobs));
    }

    pub fn paste_clipboard(&mut self, in_place: bool) -> Result<(), String> {
        let Some(frag) = self.clipboard.clone() else { return Ok(()) };
        let (w, h) = self.viewport;
        let center = self.view().screen_to_doc(Vec2::new(w as f64 / 2.0, h as f64 / 2.0));
        // paste target: artboard under content center, else shared space
        let (bx, by, bw, bh) = frag.bounds;
        let content_center = if in_place {
            Vec2::new(bx + bw / 2.0, by + bh / 2.0)
        } else {
            center
        };
        let target = self.engine.artboard_at(self.doc(), content_center);
        // same-document & source visible → offset paste (spec §10.4)
        let src_visible = frag.source_doc == self.doc().name;
        let opts = if in_place {
            PasteOptions { target_parent: target, offset: Vec2::ZERO, center_at: None }
        } else if src_visible {
            PasteOptions { target_parent: target, offset: Vec2::new(10.0, 10.0), center_at: None }
        } else {
            PasteOptions { target_parent: target, offset: Vec2::ZERO, center_at: Some(center) }
        };
        let mut blobs = std::mem::take(&mut self.blobs);
        let roots = fragment::paste(&mut self.docs[self.active].doc, &frag, &opts, &mut blobs);
        self.blobs = blobs;
        let roots = roots?;
        self.doc_mut().selected_nodes = roots;
        self.dirty_frame();
        Ok(())
    }

    pub fn duplicate_selection(&mut self) -> Result<(), String> {
        // copy + paste-in-place(+10,10) without touching the clipboard (§10.3)
        let ids = self.doc().selected_nodes.clone();
        if ids.is_empty() {
            return Ok(());
        }
        let b = self.selection_bounds();
        let frag = fragment::copy_nodes(self.doc(), &ids, (b.x, b.y, b.w, b.h), &self.blobs);
        let target = self
            .doc()
            .node(ids[0])
            .and_then(|n| n.parent);
        let opts = PasteOptions {
            target_parent: target,
            offset: Vec2::new(10.0, 10.0),
            center_at: None,
        };
        let mut blobs = std::mem::take(&mut self.blobs);
        let roots = fragment::paste(&mut self.docs[self.active].doc, &frag, &opts, &mut blobs);
        self.blobs = blobs;
        self.doc_mut().selected_nodes = roots?;
        self.dirty_frame();
        Ok(())
    }

    pub fn group_selection(&mut self) -> Result<(), String> {
        let ids = self.doc().selected_nodes.clone();
        if ids.is_empty() {
            return Ok(());
        }
        let parent = self.doc().node(ids[0]).and_then(|n| n.parent);
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            let doc = self.doc_mut();
            doc.begin_txn("Group");
            let group = doc.create_node(NodeKind::Group, parent, BTreeMap::new(), &blobs)?;
            for id in &ids {
                let frac = doc.frac_for_append(Some(group));
                doc.apply(
                    OpKind::NodeMove {
                        node_id: *id,
                        new_parent: Some(group),
                        frac_index: frac,
                        prev_parent: None,
                        prev_frac: String::new(),
                    },
                    &blobs,
                )?;
            }
            doc.commit_txn();
            doc.selected_nodes = vec![group];
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    pub fn ungroup_selection(&mut self) -> Result<(), String> {
        let ids = self.doc().selected_nodes.clone();
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            let doc = self.doc_mut();
            doc.begin_txn("Ungroup");
            let mut new_sel = Vec::new();
            for id in ids {
                let Some(n) = doc.node(id) else { continue };
                if !matches!(n.kind, NodeKind::Group | NodeKind::Layer) {
                    new_sel.push(id);
                    continue;
                }
                let parent = n.parent;
                let children: Vec<NodeId> = doc.children_of(Some(id)).to_vec();
                for c in children {
                    let frac = doc.frac_for_append(parent);
                    doc.apply(
                        OpKind::NodeMove {
                            node_id: c,
                            new_parent: parent,
                            frac_index: frac,
                            prev_parent: None,
                            prev_frac: String::new(),
                        },
                        &blobs,
                    )?;
                    new_sel.push(c);
                }
                doc.apply(OpKind::NodeDelete { node_id: id }, &blobs)?;
            }
            doc.commit_txn();
            doc.selected_nodes = new_sel;
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    fn new_artboard(&mut self, width: f64, height: f64) -> Result<(), String> {
        // place to the right of existing artboards
        let mut x = 0.0f64;
        for ab in self.doc().artboards() {
            if let Some(r) = self.doc().artboard_rect(ab) {
                x = x.max(r.x + r.w + 40.0);
            }
        }
        let count = self.doc().artboards().len();
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            let doc = self.doc_mut();
            doc.begin_txn("New artboard");
            let mut params = BTreeMap::new();
            params.insert("name".into(), Value::Str(format!("Artboard {}", count + 1)));
            params.insert("x".into(), Value::F64(x));
            params.insert("y".into(), Value::F64(0.0));
            params.insert("w".into(), Value::F64(width.clamp(1.0, ed_document::MAX_ARTBOARD_DIM)));
            params.insert("h".into(), Value::F64(height.clamp(1.0, ed_document::MAX_ARTBOARD_DIM)));
            params.insert("background".into(), Value::Str("color".into()));
            params.insert("bg-color".into(), Value::Color(Color::WHITE));
            doc.create_node(NodeKind::Artboard, None, params, &blobs)?;
            doc.commit_txn();
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    /// Explicit, recorded conversion (spec §2.4): subtree → Bitmap.
    pub fn rasterize_selection(&mut self) -> Result<(), String> {
        let ids = self.doc().selected_nodes.clone();
        if ids.is_empty() {
            return Ok(());
        }
        let bounds = self.selection_bounds();
        if bounds.w < 1.0 || bounds.h < 1.0 {
            return Ok(());
        }
        // render the selected subtrees standalone at 1:1
        let mut pm = tiny_skia::Pixmap::new(bounds.w.ceil() as u32, bounds.h.ceil() as u32)
            .ok_or("selection too large")?;
        let m = ed_core::Mat3::translate(Vec2::new(-bounds.x, -bounds.y));
        {
            let doc = &self.docs[self.active].doc;
            for &id in &ids {
                self.engine.render_node_standalone(doc, &self.blobs, id, &mut pm, &m);
            }
        }
        // demultiply into straight RGBA
        let rgba = demultiply(&pm);
        let parent = self.doc().node(ids[0]).and_then(|n| n.parent);
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            let doc = self.doc_mut();
            doc.begin_txn("Rasterize");
            for id in &ids {
                doc.apply(OpKind::NodeDelete { node_id: *id }, &blobs)?;
            }
            let mut params = BTreeMap::new();
            params.insert("name".into(), Value::Str("Rasterized".into()));
            params.insert("x".into(), Value::F64(bounds.x));
            params.insert("y".into(), Value::F64(bounds.y));
            let id = doc.create_node(NodeKind::Bitmap, parent, params, &blobs)?;
            if let Some(n) = doc.nodes.get_mut(&id) {
                n.bitmap = Some(ed_document::BitmapData::from_rgba(
                    pm.width(),
                    pm.height(),
                    &rgba,
                ));
            }
            doc.commit_txn();
            doc.selected_nodes = vec![id];
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    /// Office-style crop-in-place: shrink the visible window of selected
    /// bitmaps to the active pixel selection. The pixels outside stay in
    /// the document — Reset Crop brings them back exactly.
    pub fn crop_to_selection(&mut self) -> Result<(), String> {
        let Some(sel) = self.doc().pixel_selection.clone() else { return Ok(()) };
        if sel.is_empty() {
            return Ok(());
        }
        let sb = sel.bounds();
        // targets: selected bitmaps, else any bitmap under the selection
        let mut targets: Vec<NodeId> = self
            .doc()
            .selected_nodes
            .iter()
            .copied()
            .filter(|id| self.doc().node(*id).map(|n| n.kind == NodeKind::Bitmap).unwrap_or(false))
            .collect();
        if targets.is_empty() {
            for &root in self.doc().children_of(None) {
                let kids: Vec<NodeId> = if self
                    .doc()
                    .node(root)
                    .map(|n| n.kind == NodeKind::Artboard)
                    .unwrap_or(false)
                {
                    self.doc().children_of(Some(root)).to_vec()
                } else {
                    vec![root]
                };
                for id in kids {
                    let Some(n) = self.doc().node(id) else { continue };
                    if n.kind != NodeKind::Bitmap || !n.visible() || n.locked() {
                        continue;
                    }
                    if self
                        .engine
                        .node_bounds(self.doc(), id)
                        .map(|b| b.intersects(&sb))
                        .unwrap_or(false)
                    {
                        targets.push(id);
                    }
                }
            }
        }
        if targets.is_empty() {
            return Ok(());
        }
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            let mut cropped = Vec::new();
            for id in targets {
                let Some(bounds) = self.engine.node_bounds(self.doc(), id) else { continue };
                // intersect the selection with the visible rect
                let nx = sb.x.max(bounds.x);
                let ny = sb.y.max(bounds.y);
                let nx2 = (sb.x + sb.w).min(bounds.x + bounds.w);
                let ny2 = (sb.y + sb.h).min(bounds.y + bounds.h);
                if nx2 - nx < 1.0 || ny2 - ny < 1.0 {
                    continue;
                }
                let (origin, scale) = crate::tools::bitmap_view(self.doc(), id);
                // new crop window in natural pixels
                let cx = ((nx - origin.x) / scale.x).floor().max(0.0);
                let cy = ((ny - origin.y) / scale.y).floor().max(0.0);
                let cw = ((nx2 - nx) / scale.x).round().max(1.0);
                let ch = ((ny2 - ny) / scale.y).round().max(1.0);
                let doc = self.doc_mut();
                if doc.open_txn_label().is_none() {
                    doc.begin_txn("Crop image");
                }
                for (path, v) in [
                    ("crop-x", cx),
                    ("crop-y", cy),
                    ("crop-w", cw),
                    ("crop-h", ch),
                    ("x", nx),
                    ("y", ny),
                    ("w", nx2 - nx),
                    ("h", ny2 - ny),
                ] {
                    doc.apply(
                        OpKind::ParamSet {
                            node_id: id,
                            path: path.into(),
                            value: Value::F64(v),
                            prev: None,
                        },
                        &blobs,
                    )?;
                }
                cropped.push(id);
            }
            let doc = self.doc_mut();
            doc.commit_txn();
            if !cropped.is_empty() {
                doc.selected_nodes = cropped;
                doc.pixel_selection = None;
            }
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    /// Undo a crop-in-place: the full image returns at its original spot.
    pub fn reset_crop(&mut self) -> Result<(), String> {
        let ids = self.doc().selected_nodes.clone();
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            for id in ids {
                let Some(n) = self.doc().node(id) else { continue };
                if n.kind != NodeKind::Bitmap || n.get_param("crop-w").is_none() {
                    continue;
                }
                let Some(bm) = &n.bitmap else { continue };
                let (nat_w, nat_h) = (bm.width as f64, bm.height as f64);
                let (origin, scale) = crate::tools::bitmap_view(self.doc(), id);
                let doc = self.doc_mut();
                if doc.open_txn_label().is_none() {
                    doc.begin_txn("Reset crop");
                }
                for (path, v) in [
                    ("crop-x", 0.0),
                    ("crop-y", 0.0),
                    ("crop-w", nat_w),
                    ("crop-h", nat_h),
                    ("x", origin.x),
                    ("y", origin.y),
                    ("w", nat_w * scale.x),
                    ("h", nat_h * scale.y),
                ] {
                    doc.apply(
                        OpKind::ParamSet {
                            node_id: id,
                            path: path.into(),
                            value: Value::F64(v),
                            prev: None,
                        },
                        &blobs,
                    )?;
                }
            }
            self.doc_mut().commit_txn();
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    /// Shape→Path / Text→Path conversion (spec §2.4, scoped to shapes).
    pub fn convert_to_path(&mut self) -> Result<(), String> {
        let ids = self.doc().selected_nodes.clone();
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            for id in ids {
                let Some(node) = self.doc().node(id) else { continue };
                if node.kind != NodeKind::Shape {
                    continue;
                }
                let Some(path) = ed_engine::shapes::shape_path(self.doc(), node) else { continue };
                let d = path_to_data(&path);
                let mut params = node.params.clone();
                params.insert("d".into(), Value::Str(d));
                let parent = node.parent;
                let doc = self.doc_mut();
                doc.begin_txn("Convert to path");
                doc.apply(OpKind::NodeDelete { node_id: id }, &blobs)?;
                let new_id = doc.create_node(NodeKind::Path, parent, params, &blobs)?;
                doc.commit_txn();
                doc.selected_nodes = vec![new_id];
            }
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    // ------------------------------------------------------------ view

    pub fn fit_view(&mut self) {
        let mut bounds = Rect::default();
        for ab in self.doc().artboards() {
            if let Some(r) = self.doc().artboard_rect(ab) {
                bounds = bounds.union(&r);
            }
        }
        if bounds.w <= 0.0 {
            bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        }
        let (w, h) = self.viewport;
        let margin = 48.0;
        let zx = (w as f64 - margin * 2.0) / bounds.w;
        let zy = (h as f64 - margin * 2.0) / bounds.h;
        let zoom = zx.min(zy).clamp(0.01, 4.0);
        let v = self.view_mut();
        v.zoom = zoom;
        v.pan = Vec2::new(
            bounds.x - (w as f64 / zoom - bounds.w) / 2.0,
            bounds.y - (h as f64 / zoom - bounds.h) / 2.0,
        );
    }

    // ------------------------------------------------------------ io

    /// Import with placement options (spec §9): `scale` < 0 = fit the
    /// artboard, 1.0 = original pixels, otherwise a custom factor;
    /// `new_doc` opens the image as its own document.
    pub fn import_image(
        &mut self,
        bytes: &[u8],
        name: &str,
        scale: f64,
        new_doc: bool,
    ) -> Result<(), String> {
        let (w, h, rgba) = ed_io::decode_image(bytes)?;
        if new_doc {
            let doc_name = name.rsplit_once('.').map(|(n, _)| n).unwrap_or(name);
            let mut doc = Document::with_artboard(
                ActorId(1),
                doc_name,
                (w as f64).min(ed_document::MAX_ARTBOARD_DIM),
                (h as f64).min(ed_document::MAX_ARTBOARD_DIM),
                &self.blobs,
            );
            let ab = doc.artboards()[0];
            doc.begin_txn("Import image");
            let mut params = BTreeMap::new();
            params.insert("name".into(), Value::Str(name.to_string()));
            params.insert("x".into(), Value::F64(0.0));
            params.insert("y".into(), Value::F64(0.0));
            params.insert("w".into(), Value::F64(w as f64));
            params.insert("h".into(), Value::F64(h as f64));
            let blobs = std::mem::take(&mut self.blobs);
            let id = doc.create_node(NodeKind::Bitmap, Some(ab), params, &blobs)?;
            self.blobs = blobs;
            if let Some(n) = doc.nodes.get_mut(&id) {
                n.bitmap = Some(ed_document::BitmapData::from_rgba(w, h, &rgba));
            }
            doc.commit_txn();
            doc.history.clear();
            doc.dirty = false;
            doc.selected_nodes = vec![id];
            self.docs.push(DocState { doc, view: View::default() });
            self.active = self.docs.len() - 1;
            self.fit_view();
            self.dirty_frame();
            return Ok(());
        }
        let (vw, vh) = self.viewport;
        let center = self.view().screen_to_doc(Vec2::new(vw as f64 / 2.0, vh as f64 / 2.0));
        let parent = self.engine.artboard_at(self.doc(), center);
        // resolve placement scale (non-destructive: w/h params)
        let factor = if scale < 0.0 {
            match parent.and_then(|ab| self.doc().artboard_rect(ab)) {
                Some(r) => (r.w / w as f64).min(r.h / h as f64).min(1.0),
                None => 1.0,
            }
        } else {
            scale.clamp(0.01, 16.0)
        };
        let dw = (w as f64 * factor).round();
        let dh = (h as f64 * factor).round();
        let blobs = std::mem::take(&mut self.blobs);
        let r = (|| {
            let doc = self.doc_mut();
            doc.begin_txn("Import image");
            let mut params = BTreeMap::new();
            params.insert("name".into(), Value::Str(name.to_string()));
            params.insert("x".into(), Value::F64((center.x - dw / 2.0).round()));
            params.insert("y".into(), Value::F64((center.y - dh / 2.0).round()));
            params.insert("w".into(), Value::F64(dw));
            params.insert("h".into(), Value::F64(dh));
            let id = doc.create_node(NodeKind::Bitmap, parent, params, &blobs)?;
            if let Some(n) = doc.nodes.get_mut(&id) {
                n.bitmap = Some(ed_document::BitmapData::from_rgba(w, h, &rgba));
            }
            doc.commit_txn();
            doc.selected_nodes = vec![id];
            Ok(())
        })();
        self.blobs = blobs;
        self.dirty_frame();
        r
    }

    /// Deterministic export (spec §9): artboard by index, scale, format,
    /// with/without the artboard background, JPEG quality.
    pub fn export(
        &mut self,
        artboard_index: usize,
        scale: f64,
        format: &str,
        background: bool,
        quality: u8,
    ) -> Result<Vec<u8>, String> {
        let abs = self.doc().artboards();
        let &ab = abs.get(artboard_index).ok_or("no such artboard")?;
        let doc = &self.docs[self.active].doc;
        let pm = self
            .engine
            .render_artboard(doc, &self.blobs, ab, scale.clamp(0.05, 16.0), background)
            .ok_or("render failed")?;
        let rgba = demultiply(&pm);
        match format {
            "jpeg" => ed_io::encode_jpeg(pm.width(), pm.height(), &rgba, quality.clamp(1, 100)),
            "webp" => ed_io::encode_webp(pm.width(), pm.height(), &rgba),
            _ => ed_io::encode_png(pm.width(), pm.height(), &rgba),
        }
    }

    /// Copy-as-PNG flavor for the system clipboard (spec §10.7): the area
    /// composite when a pixel selection is active, else the node selection
    /// composite, else the active artboard.
    pub fn copy_as_png(&mut self) -> Result<Vec<u8>, String> {
        if self.has_pixel_selection() {
            if let Some((w, h, rgba)) = self.area_composite_public() {
                return ed_io::encode_png(w, h, &rgba);
            }
        }
        let ids = self.doc().selected_nodes.clone();
        if ids.is_empty() {
            return self.export(0, 1.0, "png", true, 90);
        }
        let bounds = self.selection_bounds();
        let mut pm = tiny_skia::Pixmap::new(bounds.w.ceil().max(1.0) as u32, bounds.h.ceil().max(1.0) as u32)
            .ok_or("selection too large")?;
        let m = ed_core::Mat3::translate(Vec2::new(-bounds.x, -bounds.y));
        {
            let doc = &self.docs[self.active].doc;
            for &id in &ids {
                self.engine.render_node_standalone(doc, &self.blobs, id, &mut pm, &m);
            }
        }
        let rgba = demultiply(&pm);
        ed_io::encode_png(pm.width(), pm.height(), &rgba)
    }

    pub fn save_myed(&mut self) -> Result<Vec<u8>, String> {
        let mut blobs = std::mem::take(&mut self.blobs);
        let r = ed_io::save_myed(&self.docs[self.active].doc, &mut blobs);
        self.blobs = blobs;
        if r.is_ok() {
            self.doc_mut().dirty = false;
        }
        r
    }

    pub fn open_myed(&mut self, bytes: &[u8], name: &str) -> Result<(), String> {
        let mut blobs = std::mem::take(&mut self.blobs);
        let r = ed_io::load_myed(bytes, ActorId(1), &mut blobs);
        self.blobs = blobs;
        let mut doc = r?;
        if !name.is_empty() {
            doc.name = name.trim_end_matches(".myed").to_string();
        }
        doc.dirty = false;
        self.docs.push(DocState { doc, view: View::default() });
        self.active = self.docs.len() - 1;
        self.overlays.clear();
        self.fit_view();
        self.dirty_frame();
        Ok(())
    }

    // ------------------------------------------------------------ frame & state

    /// Render the current viewport; returns straight (unpremultiplied)
    /// RGBA8 ready for `putImageData`.
    pub fn render_frame(&mut self, width: u32, height: u32, ants_phase: f32) -> Vec<u8> {
        self.viewport = (width, height);
        let DocState { doc, view } = &mut self.docs[self.active];
        view.ants_phase = ants_phase;
        // outlines are Select-tool feedback; with other tools active they
        // read as a stray border around the object
        let selected = if self.tool == ToolKind::Select {
            doc.selected_nodes.clone()
        } else {
            Vec::new()
        };
        let pm = self.engine.render(doc, &self.blobs, view, width, height, &self.overlays, &selected);
        self.last_state_rev = self.frame_rev;
        demultiply(&pm)
    }

    pub fn needs_frame(&self) -> bool {
        let doc = &self.docs[self.active].doc;
        self.frame_rev != self.last_state_rev || doc.pixel_selection.is_some()
    }

    /// Full UI mirror (spec §12.1 read models).
    pub fn state_json(&self) -> serde_json::Value {
        let doc = self.doc();
        let view = self.view();
        json!({
            "tabs": self.docs.iter().map(|d| json!({
                "name": d.doc.name,
                "dirty": d.doc.dirty,
            })).collect::<Vec<_>>(),
            "active": self.active,
            "tool": self.tool,
            "toolParams": self.tool_params,
            "fg": self.fg.to_hex(),
            "bg": self.bg.to_hex(),
            "view": {
                "zoom": view.zoom,
                "panX": view.pan.x,
                "panY": view.pan.y,
                "pixelPreview": view.pixel_preview,
            },
            "outline": doc.outline(),
            "history": doc.history_mirror(),
            "undoTop": doc.undo_top().map(|t| t.0),
            "canUndo": doc.can_undo(),
            "canRedo": doc.can_redo(),
            "selection": doc.selected_nodes.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            "props": doc.props_mirror(),
            "palette": doc.palette,
            "variables": doc.variables,
            "hasPixelSelection": doc.pixel_selection.as_ref().map(|s| !s.is_empty()).unwrap_or(false),
            "artboards": doc.artboards().iter().enumerate().map(|(i, id)| {
                let r = doc.artboard_rect(*id).unwrap_or_default();
                json!({
                    "index": i,
                    "id": id.to_string(),
                    "name": doc.node(*id).map(|n| n.name().to_string()).unwrap_or_default(),
                    "w": r.w,
                    "h": r.h,
                })
            }).collect::<Vec<_>>(),
            "status": {
                "cursorX": self.pointer_doc.x,
                "cursorY": self.pointer_doc.y,
                "cursor": self.cursor,
                "nodesRendered": self.engine.last_nodes_rendered,
            },
            "clipboardFull": self.clipboard.is_some(),
        })
    }
}

/// Premultiplied pixmap → straight RGBA8 bytes.
pub fn demultiply(pm: &tiny_skia::Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for px in pm.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

/// Serialize a tiny-skia path back to SVG data (absolute commands).
pub fn path_to_data(path: &tiny_skia::Path) -> String {
    use tiny_skia::PathSegment::*;
    let mut d = String::new();
    for seg in path.segments() {
        match seg {
            MoveTo(p) => d.push_str(&format!("M{:.2} {:.2} ", p.x, p.y)),
            LineTo(p) => d.push_str(&format!("L{:.2} {:.2} ", p.x, p.y)),
            QuadTo(c, p) => d.push_str(&format!("Q{:.2} {:.2} {:.2} {:.2} ", c.x, c.y, p.x, p.y)),
            CubicTo(c1, c2, p) => d.push_str(&format!(
                "C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} ",
                c1.x, c1.y, c2.x, c2.y, p.x, p.y
            )),
            Close => d.push('Z'),
        }
    }
    d
}

pub fn parse_node_id(s: &str) -> Option<NodeId> {
    let (a, c) = s.split_once(':')?;
    Some(NodeId::new(a.parse().ok()?, c.parse().ok()?))
}

fn json_to_value(v: &serde_json::Value) -> Option<Value> {
    // typed envelope {t, v} or bare primitives
    if let Ok(val) = serde_json::from_value::<Value>(v.clone()) {
        return Some(val);
    }
    match v {
        serde_json::Value::Number(n) => Some(Value::F64(n.as_f64()?)),
        serde_json::Value::Bool(b) => Some(Value::Bool(*b)),
        serde_json::Value::String(s) => {
            if let Some(hex) = s.strip_prefix('#') {
                if let Some(c) = Color::from_hex(hex) {
                    return Some(Value::Color(c));
                }
            }
            if let Some(expr) = s.strip_prefix('=') {
                if ed_document::expr::parse(expr).is_ok() {
                    return Some(Value::Expr(expr.to_string()));
                }
            }
            Some(Value::Str(s.clone()))
        }
        _ => None,
    }
}

fn default_tool_params() -> BTreeMap<String, Value> {
    let mut p = BTreeMap::new();
    p.insert("brush.size".into(), Value::F64(16.0));
    p.insert("brush.hardness".into(), Value::F64(0.8));
    p.insert("brush.opacity".into(), Value::F64(1.0));
    p.insert("brush.flow".into(), Value::F64(1.0));
    p.insert("brush.as-strokes".into(), Value::Bool(false));
    p.insert("pencil.size".into(), Value::F64(1.0));
    p.insert("eraser.size".into(), Value::F64(24.0));
    p.insert("eraser.hardness".into(), Value::F64(1.0));
    p.insert("fill.tolerance".into(), Value::F64(0.1));
    p.insert("fill.contiguous".into(), Value::Bool(true));
    p.insert("wand.tolerance".into(), Value::F64(0.15));
    p.insert("wand.contiguous".into(), Value::Bool(true));
    p.insert("shape.stroke-width".into(), Value::F64(0.0));
    p.insert("shape.radius".into(), Value::F64(0.0));
    p.insert("shape.sides".into(), Value::F64(6.0));
    p.insert("shape.points".into(), Value::F64(5.0));
    p.insert("pen.stroke-width".into(), Value::F64(2.0));
    p.insert("text.size".into(), Value::F64(24.0));
    p.insert("gradient.kind".into(), Value::Str("linear".into()));
    p.insert("sel.feather".into(), Value::F64(0.0));
    // area cut/move scope (spec §2.3 selections as scope): all | selected | bitmaps
    p.insert("sel.affect".into(), Value::Str("all".into()));
    p
}

fn default_modifier_params(kind: &str, doc: &Document, node: NodeId) -> BTreeMap<String, Value> {
    let mut p = BTreeMap::new();
    match kind {
        "transform" => {
            p.insert("tx".into(), Value::F64(0.0));
            p.insert("ty".into(), Value::F64(0.0));
            p.insert("rotate".into(), Value::F64(0.0));
            p.insert("sx".into(), Value::F64(1.0));
            p.insert("sy".into(), Value::F64(1.0));
            // anchor at current bounds center
            if let Some(n) = doc.node(node) {
                let cx = doc.param_f64(n, "x", 0.0) + doc.param_f64(n, "w", 0.0) / 2.0;
                let cy = doc.param_f64(n, "y", 0.0) + doc.param_f64(n, "h", 0.0) / 2.0;
                p.insert("ax".into(), Value::F64(cx));
                p.insert("ay".into(), Value::F64(cy));
            }
        }
        "filter.gaussian-blur" => {
            p.insert("radius".into(), Value::F64(4.0));
        }
        "filter.pixelate" => {
            p.insert("size".into(), Value::F64(8.0));
        }
        "filter.noise" => {
            p.insert("amount".into(), Value::F64(0.2));
        }
        "adjust.brightness-contrast" => {
            p.insert("brightness".into(), Value::F64(0.0));
            p.insert("contrast".into(), Value::F64(0.0));
        }
        "adjust.hsl" => {
            p.insert("hue".into(), Value::F64(0.0));
            p.insert("saturation".into(), Value::F64(0.0));
            p.insert("lightness".into(), Value::F64(0.0));
        }
        "adjust.levels" => {
            p.insert("in-black".into(), Value::F64(0.0));
            p.insert("in-white".into(), Value::F64(1.0));
            p.insert("gamma".into(), Value::F64(1.0));
            p.insert("out-black".into(), Value::F64(0.0));
            p.insert("out-white".into(), Value::F64(1.0));
        }
        "adjust.posterize" => {
            p.insert("levels".into(), Value::F64(4.0));
        }
        "adjust.threshold" => {
            p.insert("level".into(), Value::F64(0.5));
        }
        "clip" => {
            if let Some(n) = doc.node(node) {
                p.insert("x".into(), Value::F64(doc.param_f64(n, "x", 0.0)));
                p.insert("y".into(), Value::F64(doc.param_f64(n, "y", 0.0)));
                p.insert("w".into(), Value::F64(doc.param_f64(n, "w", 100.0)));
                p.insert("h".into(), Value::F64(doc.param_f64(n, "h", 100.0)));
            }
        }
        _ => {}
    }
    p
}

// ------------------------------------------------------------ command schema

#[derive(Deserialize, Debug)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
enum Command {
    Pointer(crate::input::InputEvent),
    SetTool { tool: ToolKind },
    SetToolParam { key: String, value: Value },
    SetFg { color: String },
    SetBg { color: String },
    NewDoc {
        width: f64,
        height: f64,
        name: Option<String>,
        background: Option<String>,
        bg_color: Option<String>,
        dpi: Option<f64>,
    },
    SwitchDoc { index: usize },
    CloseDoc { index: usize },
    RenameDoc { name: String },
    Undo,
    Redo,
    SelectNodes { ids: Vec<String>, #[serde(default)] toggle: bool },
    DeleteSelection,
    DuplicateSelection,
    GroupSelection,
    UngroupSelection,
    Copy,
    Cut,
    Paste { #[serde(default)] in_place: bool },
    SetParam { node: String, path: String, value: serde_json::Value },
    PreviewParam { node: String, path: String, value: serde_json::Value },
    AddModifier { node: String, kind: String },
    RemoveModifier { node: String, id: u64 },
    ReorderModifier { node: String, id: u64, index: usize },
    MoveNode { node: String, parent: Option<String>, index: usize },
    SetView { zoom: Option<f64>, pan_x: Option<f64>, pan_y: Option<f64> },
    ZoomBy { factor: f64, cx: f64, cy: f64 },
    FitView,
    SetPixelPreview { on: bool },
    Resize { width: u32, height: u32 },
    SetVariable { name: String, value: Option<serde_json::Value> },
    SetPalette { name: String, color: Option<String> },
    NewArtboard { width: f64, height: f64 },
    ClearPixelSelection,
    InvertPixelSelection,
    SelectAll,
    Key { key: String, #[serde(default)] mods: crate::input::Modifiers },
    RasterizeSelection,
    ConvertToPath,
    SelectAt { x: f64, y: f64 },
    HistoryJump { id: u64 },
    CropToSelection,
    ResetCrop,
}
