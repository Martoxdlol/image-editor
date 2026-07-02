//! The Document: single source of truth (spec §1). Holds the movable tree,
//! the append-only op log grouped into transactions, per-actor undo/redo,
//! globals (variables, palette), and transient selection state.

use crate::expr::{self, ExprValue};
use crate::node::{BitmapData, Node, NodeKind};
use crate::ops::{Op, OpKind, TilePatch, Txn};
use crate::{frac_index, selection::PixelSelection};
use ed_core::{ActorId, BlobHash, Color, NodeId, OpId, TxnId, Value, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Content-addressed binary store (spec §3.2). Shared across the session
/// so cross-document paste dedupes automatically (spec §12.1).
#[derive(Default, Debug)]
pub struct BlobStore {
    blobs: HashMap<BlobHash, Vec<u8>>,
}

impl BlobStore {
    pub fn put(&mut self, data: Vec<u8>) -> BlobHash {
        let hash = BlobHash::of(&data);
        self.blobs.entry(hash).or_insert(data);
        hash
    }

    pub fn get(&self, hash: BlobHash) -> Option<&[u8]> {
        self.blobs.get(&hash).map(|v| v.as_slice())
    }

    pub fn contains(&self, hash: BlobHash) -> bool {
        self.blobs.contains_key(&hash)
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct PaletteEntry {
    pub name: String,
    pub color: Color,
}

pub const MAX_ARTBOARD_DIM: f64 = 16384.0; // spec §2.1 / §4.4

#[derive(Debug)]
pub struct Document {
    pub actor: ActorId,
    lamport: u64,
    node_counter: u64,
    modifier_counter: u64,
    txn_counter: u64,
    last_op: Option<OpId>,

    pub nodes: HashMap<NodeId, Node>,
    /// Tombstoned nodes (spec §3.1): detached but fully retained.
    graveyard: HashMap<NodeId, Node>,
    /// Sibling lists sorted by fractional index. `None` key = top level.
    children: HashMap<Option<NodeId>, Vec<NodeId>>,

    pub variables: BTreeMap<String, Value>,
    pub palette: Vec<PaletteEntry>,

    /// Committed transactions — the visible history (spec §3.3).
    pub history: Vec<Txn>,
    undo_stack: Vec<TxnId>,
    redo_stack: Vec<TxnId>,
    open_txn: Option<(TxnId, String, Vec<Op>)>,

    /// Transient UI-facing state, mirrored to React but owned here (§1).
    pub selected_nodes: Vec<NodeId>,
    pub pixel_selection: Option<PixelSelection>,

    pub name: String,
    pub dirty: bool,
    /// Bumped on every visible change; the render loop watches it.
    pub revision: u64,
}

impl Document {
    pub fn new(actor: ActorId, name: &str) -> Self {
        let mut doc = Document {
            actor,
            lamport: 0,
            node_counter: 0,
            modifier_counter: 0,
            txn_counter: 0,
            last_op: None,
            nodes: HashMap::new(),
            graveyard: HashMap::new(),
            children: HashMap::new(),
            variables: BTreeMap::new(),
            palette: Vec::new(),
            history: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            open_txn: None,
            selected_nodes: Vec::new(),
            pixel_selection: None,
            name: name.to_string(),
            dirty: false,
            revision: 0,
        };
        doc.children.insert(None, Vec::new());
        doc
    }

    // ------------------------------------------------------------ ids

    pub fn mint_node_id(&mut self) -> NodeId {
        self.node_counter += 1;
        NodeId::new(self.actor.0, self.node_counter)
    }

    pub fn mint_modifier_id(&mut self) -> u64 {
        self.modifier_counter += 1;
        self.modifier_counter
    }

    fn next_op_id(&mut self) -> OpId {
        self.lamport += 1;
        OpId { actor: self.actor, lamport: self.lamport }
    }

    // ------------------------------------------------------------ tree reads

    pub fn children_of(&self, parent: Option<NodeId>) -> &[NodeId] {
        self.children.get(&parent).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn artboards(&self) -> Vec<NodeId> {
        self.children_of(None)
            .iter()
            .copied()
            .filter(|id| self.nodes.get(id).map(|n| n.kind == NodeKind::Artboard).unwrap_or(false))
            .collect()
    }

    /// Depth-first walk of a subtree, parents before children.
    pub fn walk(&self, root: NodeId, out: &mut Vec<NodeId>) {
        out.push(root);
        for &c in self.children_of(Some(root)) {
            self.walk(c, out);
        }
    }

    pub fn is_ancestor(&self, maybe_ancestor: NodeId, node: NodeId) -> bool {
        let mut cur = self.nodes.get(&node).and_then(|n| n.parent);
        while let Some(p) = cur {
            if p == maybe_ancestor {
                return true;
            }
            cur = self.nodes.get(&p).and_then(|n| n.parent);
        }
        false
    }

    /// Fractional index for appending at the end of a sibling list.
    pub fn frac_for_append(&self, parent: Option<NodeId>) -> String {
        let siblings = self.children_of(parent);
        let last = siblings.last().and_then(|id| self.nodes.get(id)).map(|n| n.frac.as_str());
        frac_index::between(last, None)
    }

    /// Fractional index for inserting at `index` within a sibling list.
    pub fn frac_for_insert(&self, parent: Option<NodeId>, index: usize) -> String {
        let siblings = self.children_of(parent);
        let frac_at = |i: usize| siblings.get(i).and_then(|id| self.nodes.get(id)).map(|n| n.frac.as_str());
        let lo = if index == 0 { None } else { frac_at(index - 1) };
        let hi = frac_at(index);
        frac_index::between(lo, hi)
    }

    // ------------------------------------------------------------ txns

    pub fn begin_txn(&mut self, label: &str) {
        if self.open_txn.is_some() {
            self.commit_txn();
        }
        self.txn_counter += 1;
        self.open_txn = Some((TxnId(self.txn_counter), label.to_string(), Vec::new()));
    }

    pub fn commit_txn(&mut self) {
        if let Some((id, label, ops)) = self.open_txn.take() {
            if ops.is_empty() {
                return;
            }
            self.history.push(Txn { id, label, ops, undo_of: None });
            self.undo_stack.push(id);
            self.redo_stack.clear();
            self.dirty = true;
        }
    }

    pub fn open_txn_label(&self) -> Option<&str> {
        self.open_txn.as_ref().map(|(_, label, _)| label.as_str())
    }

    pub fn abort_txn(&mut self, blobs: &BlobStore) {
        if let Some((_, _, ops)) = self.open_txn.take() {
            for op in ops.iter().rev() {
                let _ = self.apply_raw(&op.kind.inverse(), blobs);
            }
            self.revision += 1;
        }
    }

    /// Record and apply an op inside the open txn. Returns the applied op
    /// (with prev-values filled in) or an error string.
    pub fn apply(&mut self, kind: OpKind, blobs: &BlobStore) -> Result<(), String> {
        if self.open_txn.is_none() {
            self.begin_txn("Edit");
        }
        let applied = self.apply_raw(&kind, blobs)?;
        let id = self.next_op_id();
        let parents = self.last_op.iter().copied().collect();
        self.last_op = Some(id);
        let txn = self.open_txn.as_ref().unwrap().0;
        let op = Op { id, parents, txn, kind: applied };
        self.open_txn.as_mut().unwrap().2.push(op);
        self.revision += 1;
        Ok(())
    }

    /// One-shot convenience: begin, apply, commit.
    pub fn apply_txn(&mut self, label: &str, kind: OpKind, blobs: &BlobStore) -> Result<(), String> {
        self.begin_txn(label);
        let r = self.apply(kind, blobs);
        self.commit_txn();
        r
    }

    // ------------------------------------------------------------ undo/redo

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Per-actor undo: appends the inverse ops as a NEW txn (spec §3.3).
    pub fn undo(&mut self, blobs: &BlobStore) -> Option<String> {
        self.commit_txn();
        let target = self.undo_stack.pop()?;
        let txn = self.history.iter().find(|t| t.id == target)?.clone();
        let label = format!("Undo {}", txn.label);
        self.txn_counter += 1;
        let undo_id = TxnId(self.txn_counter);
        let mut ops = Vec::new();
        for op in txn.ops.iter().rev() {
            let inv = op.kind.inverse();
            if self.apply_raw(&inv, blobs).is_ok() {
                let id = self.next_op_id();
                let parents = self.last_op.iter().copied().collect();
                self.last_op = Some(id);
                ops.push(Op { id, parents, txn: undo_id, kind: inv });
            }
        }
        self.history.push(Txn { id: undo_id, label: label.clone(), ops, undo_of: Some(target) });
        self.redo_stack.push(target);
        self.revision += 1;
        self.dirty = true;
        Some(label)
    }

    pub fn redo(&mut self, blobs: &BlobStore) -> Option<String> {
        let target = self.redo_stack.pop()?;
        let txn = self.history.iter().find(|t| t.id == target)?.clone();
        let label = format!("Redo {}", txn.label);
        self.txn_counter += 1;
        let redo_id = TxnId(self.txn_counter);
        let mut ops = Vec::new();
        for op in txn.ops.iter() {
            if self.apply_raw(&op.kind, blobs).is_ok() {
                let id = self.next_op_id();
                let parents = self.last_op.iter().copied().collect();
                self.last_op = Some(id);
                ops.push(Op { id, parents, txn: redo_id, kind: op.kind.clone() });
            }
        }
        self.history.push(Txn { id: redo_id, label: label.clone(), ops, undo_of: Some(target) });
        self.undo_stack.push(target);
        self.revision += 1;
        self.dirty = true;
        Some(label)
    }

    // ------------------------------------------------------------ op application

    fn attach_to_children(&mut self, node_id: NodeId) {
        let (parent, frac) = {
            let n = &self.nodes[&node_id];
            (n.parent, n.frac.clone())
        };
        let pos = {
            let list = self.children.get(&parent).map(|v| v.as_slice()).unwrap_or(&[]);
            list.partition_point(|id| self.nodes[id].frac.as_str() < frac.as_str())
        };
        self.children.entry(parent).or_default().insert(pos, node_id);
    }

    fn detach_from_children(&mut self, node_id: NodeId) {
        let parent = self.nodes.get(&node_id).and_then(|n| n.parent);
        // Node may be top-level (parent None) — that list also needs cleanup.
        if let Some(list) = self.children.get_mut(&parent) {
            list.retain(|&id| id != node_id);
        }
    }

    /// Apply an op to the tree, returning a fully-invertible copy of it
    /// (prev values filled in). Does not touch history.
    fn apply_raw(&mut self, kind: &OpKind, blobs: &BlobStore) -> Result<OpKind, String> {
        match kind {
            OpKind::NodeCreate { node } => {
                if self.nodes.contains_key(&node.id) {
                    return Err(format!("node {} already exists", node.id));
                }
                self.nodes.insert(node.id, (**node).clone());
                self.children.entry(Some(node.id)).or_default();
                self.attach_to_children(node.id);
                Ok(kind.clone())
            }
            OpKind::NodeDelete { node_id } => {
                let node =
                    self.nodes.get(node_id).cloned().ok_or_else(|| format!("no node {node_id}"))?;
                self.detach_from_children(*node_id);
                self.nodes.remove(node_id);
                // Tombstone the whole subtree so hit-tests can't find dead
                // nodes; children lists stay intact for exact restore.
                let direct: Vec<NodeId> =
                    self.children.get(&Some(*node_id)).cloned().unwrap_or_default();
                let mut subtree = Vec::new();
                for c in direct {
                    self.walk_including_dead(c, &mut subtree);
                }
                for id in subtree {
                    if let Some(n) = self.nodes.remove(&id) {
                        self.graveyard.insert(id, n);
                    }
                }
                self.graveyard.insert(*node_id, node);
                self.selected_nodes.retain(|id| id != node_id);
                Ok(kind.clone())
            }
            OpKind::NodeRestore { node_id } => {
                let node = self
                    .graveyard
                    .remove(node_id)
                    .ok_or_else(|| format!("no tombstone for {node_id}"))?;
                self.nodes.insert(*node_id, node);
                // restore descendants
                let mut stack = vec![*node_id];
                while let Some(id) = stack.pop() {
                    let kids: Vec<NodeId> =
                        self.children.get(&Some(id)).cloned().unwrap_or_default();
                    for c in kids {
                        if let Some(n) = self.graveyard.remove(&c) {
                            self.nodes.insert(c, n);
                        }
                        stack.push(c);
                    }
                }
                self.attach_to_children(*node_id);
                Ok(kind.clone())
            }
            OpKind::NodeMove { node_id, new_parent, frac_index, .. } => {
                if let Some(p) = new_parent {
                    if *p == *node_id || self.is_ancestor(*node_id, *p) {
                        return Err("cannot move a node into its own subtree".into());
                    }
                    if !self.nodes.contains_key(p) {
                        return Err(format!("no parent {p}"));
                    }
                }
                let (prev_parent, prev_frac) = {
                    let n = self.nodes.get(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                    (n.parent, n.frac.clone())
                };
                self.detach_from_children(*node_id);
                {
                    let n = self.nodes.get_mut(node_id).unwrap();
                    n.parent = *new_parent;
                    n.frac = frac_index.clone();
                }
                self.attach_to_children(*node_id);
                Ok(OpKind::NodeMove {
                    node_id: *node_id,
                    new_parent: *new_parent,
                    frac_index: frac_index.clone(),
                    prev_parent,
                    prev_frac,
                })
            }
            OpKind::ParamSet { node_id, path, value, .. } => {
                let n = self.nodes.get_mut(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                let prev = n.set_param(path, value.clone());
                Ok(OpKind::ParamSet {
                    node_id: *node_id,
                    path: path.clone(),
                    value: value.clone(),
                    prev,
                })
            }
            OpKind::ModifierAttach { node_id, modifier, index } => {
                let n = self.nodes.get_mut(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                let index = (*index).min(n.modifiers.len());
                n.modifiers.insert(index, modifier.clone());
                Ok(OpKind::ModifierAttach { node_id: *node_id, modifier: modifier.clone(), index })
            }
            OpKind::ModifierDetach { node_id, modifier_id, .. } => {
                let n = self.nodes.get_mut(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                let index = n
                    .modifiers
                    .iter()
                    .position(|m| m.id == *modifier_id)
                    .ok_or_else(|| format!("no modifier {modifier_id}"))?;
                let removed = n.modifiers.remove(index);
                Ok(OpKind::ModifierDetach {
                    node_id: *node_id,
                    modifier_id: *modifier_id,
                    removed: Some((removed, index)),
                })
            }
            OpKind::ModifierReorder { node_id, modifier_id, new_index, .. } => {
                let n = self.nodes.get_mut(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                let prev_index = n
                    .modifiers
                    .iter()
                    .position(|m| m.id == *modifier_id)
                    .ok_or_else(|| format!("no modifier {modifier_id}"))?;
                let m = n.modifiers.remove(prev_index);
                let new_index = (*new_index).min(n.modifiers.len());
                n.modifiers.insert(new_index, m);
                Ok(OpKind::ModifierReorder {
                    node_id: *node_id,
                    modifier_id: *modifier_id,
                    new_index,
                    prev_index,
                })
            }
            OpKind::PaintTilePatch { node_id, patches } => {
                let n = self.nodes.get_mut(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                let bm = n.bitmap.as_mut().ok_or("node has no bitmap")?;
                for p in patches {
                    match p.after {
                        None => {
                            bm.tiles.remove(&p.tile);
                        }
                        Some(hash) => {
                            let data =
                                blobs.get(hash).ok_or_else(|| format!("missing blob {hash}"))?;
                            bm.tiles.insert(p.tile, data.to_vec());
                        }
                    }
                }
                bm.rev += 1;
                Ok(kind.clone())
            }
            OpKind::StrokesSet { node_id, strokes, .. } => {
                let n = self.nodes.get_mut(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                let prev = std::mem::replace(&mut n.strokes, strokes.clone());
                Ok(OpKind::StrokesSet { node_id: *node_id, strokes: strokes.clone(), prev })
            }
            OpKind::BitmapReplace { node_id, width, height, blob, .. } => {
                let n = self.nodes.get_mut(node_id).ok_or_else(|| format!("no node {node_id}"))?;
                let bm = n.bitmap.as_ref().ok_or("node has no bitmap")?;
                let (prev_width, prev_height) = (bm.width, bm.height);
                let prev_blob = if bm.tiles.is_empty() { None } else { Some(BlobHash::of(&bm.to_rgba())) };
                let mut new_bm = match blob {
                    None => BitmapData::new(*width, *height),
                    Some(hash) => {
                        let data = blobs.get(*hash).ok_or_else(|| format!("missing blob {hash}"))?;
                        BitmapData::from_rgba(*width, *height, data)
                    }
                };
                new_bm.rev = bm.rev + 1;
                n.bitmap = Some(new_bm);
                Ok(OpKind::BitmapReplace {
                    node_id: *node_id,
                    width: *width,
                    height: *height,
                    blob: *blob,
                    prev_width,
                    prev_height,
                    prev_blob,
                })
            }
            OpKind::VariableSet { name, value, .. } => {
                let prev = match value {
                    Some(v) => self.variables.insert(name.clone(), v.clone()),
                    None => self.variables.remove(name),
                };
                Ok(OpKind::VariableSet { name: name.clone(), value: value.clone(), prev })
            }
            OpKind::PaletteSet { name, value, .. } => {
                let prev = self.palette.iter().find(|e| e.name == *name).map(|e| e.color);
                match value {
                    Some(c) => {
                        if let Some(e) = self.palette.iter_mut().find(|e| e.name == *name) {
                            e.color = *c;
                        } else {
                            self.palette.push(PaletteEntry { name: name.clone(), color: *c });
                        }
                    }
                    None => self.palette.retain(|e| e.name != *name),
                }
                Ok(OpKind::PaletteSet { name: name.clone(), value: *value, prev })
            }
        }
    }

    fn walk_including_dead(&self, root: NodeId, out: &mut Vec<NodeId>) {
        out.push(root);
        for &c in self.children.get(&Some(root)).map(|v| v.as_slice()).unwrap_or(&[]) {
            self.walk_including_dead(c, out);
        }
    }

    // ------------------------------------------------------------ params & expressions

    /// Resolve a param to a concrete value: follows palette/variable refs
    /// and evaluates expressions against globals (spec §8, §6.7).
    pub fn resolve(&self, v: &Value) -> Value {
        match v {
            Value::Ref(r) => match r {
                ed_core::value::RefValue::Palette { entry } => self
                    .palette
                    .iter()
                    .find(|e| e.name == *entry)
                    .map(|e| Value::Color(e.color))
                    .unwrap_or(Value::Color(Color::BLACK)),
                ed_core::value::RefValue::Variable { name } => self
                    .variables
                    .get(name)
                    .map(|v| self.resolve(v))
                    .unwrap_or(Value::F64(0.0)),
                ed_core::value::RefValue::Node { .. } => v.clone(),
            },
            Value::Expr(src) => match expr::parse(src).and_then(|ast| ast.eval(&|path| self.expr_ref(path))) {
                Ok(ExprValue::Number(n)) => Value::F64(n),
                Ok(ExprValue::Point(p)) => Value::Point(p),
                Ok(ExprValue::Color(c)) => Value::Color(c),
                Err(_) => Value::F64(0.0),
            },
            other => other.clone(),
        }
    }

    fn expr_ref(&self, path: &[String]) -> Option<ExprValue> {
        if path.first().map(|s| s.as_str()) == Some("palette") && path.len() == 2 {
            return self
                .palette
                .iter()
                .find(|e| e.name == path[1])
                .map(|e| ExprValue::Color(e.color));
        }
        let v = self.variables.get(&path.join("."))?;
        match self.resolve(v) {
            Value::F64(n) => Some(ExprValue::Number(n)),
            Value::Point(p) => Some(ExprValue::Point(p)),
            Value::Color(c) => Some(ExprValue::Color(c)),
            _ => None,
        }
    }

    pub fn param_f64(&self, node: &Node, path: &str, default: f64) -> f64 {
        node.get_param(path)
            .map(|v| self.resolve(&v))
            .and_then(|v| v.as_f64())
            .unwrap_or(default)
    }

    pub fn param_color(&self, node: &Node, path: &str, default: Color) -> Color {
        node.get_param(path)
            .map(|v| self.resolve(&v))
            .and_then(|v| v.as_color())
            .unwrap_or(default)
    }

    pub fn param_str(&self, node: &Node, path: &str, default: &str) -> String {
        node.get_param(path)
            .and_then(|v| match v {
                Value::Str(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| default.to_string())
    }

    pub fn param_bool(&self, node: &Node, path: &str, default: bool) -> bool {
        node.get_param(path)
            .map(|v| self.resolve(&v))
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    }

    // ------------------------------------------------------------ helpers for tools

    /// Create a node in one call (used by tools & paste). Caller owns txn.
    pub fn create_node(
        &mut self,
        kind: NodeKind,
        parent: Option<NodeId>,
        params: BTreeMap<String, Value>,
        blobs: &BlobStore,
    ) -> Result<NodeId, String> {
        let id = self.mint_node_id();
        let frac = self.frac_for_append(parent);
        let node = crate::ops::make_node(id, kind, parent, frac, params);
        self.apply(OpKind::NodeCreate { node: Box::new(node) }, blobs)?;
        Ok(id)
    }

    /// Record a bitmap paint as tile patches, given before-tiles captured at
    /// stroke start and the current (already painted) tile contents.
    pub fn commit_paint(
        &mut self,
        node_id: NodeId,
        before_tiles: &BTreeMap<(u32, u32), Option<Vec<u8>>>,
        blobs: &mut BlobStore,
    ) -> Result<(), String> {
        let node = self.nodes.get(&node_id).ok_or("no node")?;
        let bm = node.bitmap.as_ref().ok_or("no bitmap")?;
        let mut patches = Vec::new();
        for (&tile, before) in before_tiles {
            let after = bm.tiles.get(&tile);
            let before_hash = before.as_ref().map(|d| BlobHash::of(d));
            let after_hash = after.map(|d| BlobHash::of(d));
            if before_hash == after_hash {
                continue;
            }
            if let Some(d) = before {
                blobs.put(d.clone());
            }
            if let Some(d) = after {
                blobs.put(d.clone());
            }
            patches.push(TilePatch { tile, before: before_hash, after: after_hash });
        }
        if patches.is_empty() {
            return Ok(());
        }
        self.apply(OpKind::PaintTilePatch { node_id, patches }, blobs)
    }

    /// Artboard bounds in pasteboard coordinates.
    pub fn artboard_rect(&self, id: NodeId) -> Option<ed_core::Rect> {
        let n = self.nodes.get(&id)?;
        if n.kind != NodeKind::Artboard {
            return None;
        }
        Some(ed_core::Rect::new(
            self.param_f64(n, "x", 0.0),
            self.param_f64(n, "y", 0.0),
            self.param_f64(n, "w", 0.0),
            self.param_f64(n, "h", 0.0),
        ))
    }

    /// Standard new document: one artboard.
    pub fn with_artboard(actor: ActorId, name: &str, w: f64, h: f64, blobs: &BlobStore) -> Self {
        let mut doc = Document::new(actor, name);
        doc.begin_txn("New document");
        let mut params = BTreeMap::new();
        params.insert("name".into(), Value::Str("Artboard 1".into()));
        params.insert("x".into(), Value::F64(0.0));
        params.insert("y".into(), Value::F64(0.0));
        params.insert("w".into(), Value::F64(w.min(MAX_ARTBOARD_DIM)));
        params.insert("h".into(), Value::F64(h.min(MAX_ARTBOARD_DIM)));
        params.insert("dpi".into(), Value::F64(72.0));
        params.insert("background".into(), Value::Str("color".into()));
        params.insert("bg-color".into(), Value::Color(Color::WHITE));
        let _ = doc.create_node(NodeKind::Artboard, None, params, blobs);
        doc.commit_txn();
        // A fresh document isn't "dirty" and its creation isn't undoable.
        doc.history.clear();
        doc.undo_stack.clear();
        doc.dirty = false;
        doc
    }

    /// Node's own geometric position params, used by move tool (x/y or points).
    pub fn node_position(&self, id: NodeId) -> Vec2 {
        self.nodes
            .get(&id)
            .map(|n| Vec2::new(self.param_f64(n, "x", 0.0), self.param_f64(n, "y", 0.0)))
            .unwrap_or(Vec2::ZERO)
    }

    /// Transient param write during a drag preview — bypasses the op log.
    /// The owning tool must restore initial values before committing the
    /// real txn on pointer-up (spec §3.3: one drag = one txn).
    pub fn preview_param(&mut self, id: NodeId, path: &str, value: Value) {
        if let Some(n) = self.nodes.get_mut(&id) {
            n.set_param(path, value);
            self.revision += 1;
        }
    }

    /// Direct bitmap access for live painting (preview = real mutation;
    /// undo comes from before-tile capture + `commit_paint`).
    pub fn bitmap_mut(&mut self, id: NodeId) -> Option<&mut BitmapData> {
        self.revision += 1;
        self.nodes.get_mut(&id)?.bitmap.as_mut()
    }

    pub fn strokes_mut(&mut self, id: NodeId) -> Option<&mut Vec<crate::node::Stroke>> {
        self.revision += 1;
        Some(&mut self.nodes.get_mut(&id)?.strokes)
    }

    // ------------------------------------------------------------ loading

    /// Insert a node during snapshot load, bypassing the op log.
    pub fn insert_loaded_node(&mut self, node: Node) {
        let id = node.id;
        self.nodes.insert(id, node);
        self.children.entry(Some(id)).or_default();
        self.attach_to_children(id);
    }

    /// After load: continue minting ids above anything in the file.
    pub fn set_counters(&mut self, max_node_counter: u64) {
        self.node_counter = self.node_counter.max(max_node_counter);
        let max_mod = self
            .nodes
            .values()
            .flat_map(|n| n.modifiers.iter().map(|m| m.id))
            .max()
            .unwrap_or(0);
        self.modifier_counter = self.modifier_counter.max(max_mod);
    }

    /// What the next minted id would look like (test/diagnostic helper).
    pub fn clone_id_probe(&self) -> NodeId {
        NodeId::new(self.actor.0, self.node_counter + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Document, BlobStore) {
        let blobs = BlobStore::default();
        let doc = Document::with_artboard(ActorId(1), "t", 800.0, 600.0, &blobs);
        (doc, blobs)
    }

    #[test]
    fn undo_appends_never_rewrites() {
        let (mut doc, blobs) = setup();
        let ab = doc.artboards()[0];
        doc.begin_txn("add");
        let id = doc.create_node(NodeKind::Shape, Some(ab), BTreeMap::new(), &blobs).unwrap();
        doc.commit_txn();
        assert_eq!(doc.history.len(), 1);
        assert!(doc.node(id).is_some());

        doc.undo(&blobs).unwrap();
        assert!(doc.node(id).is_none(), "undo removes the node");
        assert_eq!(doc.history.len(), 2, "undo appended a txn, didn't pop");
        assert!(doc.history[1].undo_of.is_some());

        doc.redo(&blobs).unwrap();
        assert!(doc.node(id).is_some(), "redo restores the node");
        assert_eq!(doc.history.len(), 3);
    }

    #[test]
    fn param_undo_restores_previous_value() {
        let (mut doc, blobs) = setup();
        let ab = doc.artboards()[0];
        doc.begin_txn("add");
        let id = doc.create_node(NodeKind::Shape, Some(ab), BTreeMap::new(), &blobs).unwrap();
        doc.commit_txn();

        doc.apply_txn(
            "set x",
            OpKind::ParamSet { node_id: id, path: "x".into(), value: Value::F64(10.0), prev: None },
            &blobs,
        )
        .unwrap();
        doc.apply_txn(
            "set x",
            OpKind::ParamSet { node_id: id, path: "x".into(), value: Value::F64(99.0), prev: None },
            &blobs,
        )
        .unwrap();
        assert_eq!(doc.param_f64(doc.node(id).unwrap(), "x", 0.0), 99.0);
        doc.undo(&blobs);
        assert_eq!(doc.param_f64(doc.node(id).unwrap(), "x", 0.0), 10.0);
        doc.undo(&blobs);
        doc.undo(&blobs); // undo create
        assert!(doc.node(id).is_none());
        doc.redo(&blobs);
        assert!(doc.node(id).is_some());
    }

    #[test]
    fn delete_tombstones_subtree_and_restores() {
        let (mut doc, blobs) = setup();
        let ab = doc.artboards()[0];
        doc.begin_txn("build");
        let group = doc.create_node(NodeKind::Group, Some(ab), BTreeMap::new(), &blobs).unwrap();
        let child = doc.create_node(NodeKind::Shape, Some(group), BTreeMap::new(), &blobs).unwrap();
        doc.commit_txn();

        doc.apply_txn("delete", OpKind::NodeDelete { node_id: group }, &blobs).unwrap();
        assert!(doc.node(group).is_none());
        assert!(doc.node(child).is_none(), "descendants tombstoned too");

        doc.undo(&blobs);
        assert!(doc.node(group).is_some());
        assert!(doc.node(child).is_some(), "descendants restored");
        assert_eq!(doc.children_of(Some(group)), &[child]);
    }

    #[test]
    fn sibling_order_via_frac_index() {
        let (mut doc, blobs) = setup();
        let ab = doc.artboards()[0];
        doc.begin_txn("build");
        let a = doc.create_node(NodeKind::Shape, Some(ab), BTreeMap::new(), &blobs).unwrap();
        let b = doc.create_node(NodeKind::Shape, Some(ab), BTreeMap::new(), &blobs).unwrap();
        let c = doc.create_node(NodeKind::Shape, Some(ab), BTreeMap::new(), &blobs).unwrap();
        doc.commit_txn();
        assert_eq!(doc.children_of(Some(ab)), &[a, b, c]);

        // move c between a and b
        let frac = doc.frac_for_insert(Some(ab), 1);
        doc.apply_txn(
            "reorder",
            OpKind::NodeMove {
                node_id: c,
                new_parent: Some(ab),
                frac_index: frac,
                prev_parent: None,
                prev_frac: String::new(),
            },
            &blobs,
        )
        .unwrap();
        assert_eq!(doc.children_of(Some(ab)), &[a, c, b]);
        doc.undo(&blobs);
        assert_eq!(doc.children_of(Some(ab)), &[a, b, c]);
    }

    #[test]
    fn move_into_own_subtree_rejected() {
        let (mut doc, blobs) = setup();
        let ab = doc.artboards()[0];
        doc.begin_txn("build");
        let g1 = doc.create_node(NodeKind::Group, Some(ab), BTreeMap::new(), &blobs).unwrap();
        let g2 = doc.create_node(NodeKind::Group, Some(g1), BTreeMap::new(), &blobs).unwrap();
        doc.commit_txn();
        doc.begin_txn("bad move");
        let r = doc.apply(
            OpKind::NodeMove {
                node_id: g1,
                new_parent: Some(g2),
                frac_index: "V".into(),
                prev_parent: None,
                prev_frac: String::new(),
            },
            &blobs,
        );
        assert!(r.is_err());
    }

    #[test]
    fn paint_tile_patch_undo() {
        let (mut doc, mut blobs) = setup();
        let ab = doc.artboards()[0];
        doc.begin_txn("add bitmap");
        let bmp = doc.create_node(NodeKind::Bitmap, Some(ab), BTreeMap::new(), &blobs).unwrap();
        doc.commit_txn();
        {
            let bm = doc.nodes.get_mut(&bmp).unwrap().bitmap.as_mut().unwrap();
            bm.width = 64;
            bm.height = 64;
        }

        // stroke: capture before, mutate, commit patches
        let before: BTreeMap<(u32, u32), Option<Vec<u8>>> = BTreeMap::from([((0, 0), None)]);
        {
            let bm = doc.nodes.get_mut(&bmp).unwrap().bitmap.as_mut().unwrap();
            bm.set_pixel(3, 3, [255, 0, 0, 255]);
        }
        doc.begin_txn("paint");
        doc.commit_paint(bmp, &before, &mut blobs).unwrap();
        doc.commit_txn();

        let px = |doc: &Document| doc.node(bmp).unwrap().bitmap.as_ref().unwrap().get_pixel(3, 3);
        assert_eq!(px(&doc), [255, 0, 0, 255]);
        doc.undo(&blobs);
        assert_eq!(px(&doc), [0, 0, 0, 0], "undo clears the painted tile");
        doc.redo(&blobs);
        assert_eq!(px(&doc), [255, 0, 0, 255], "redo replays tile patch from blob");
    }

    #[test]
    fn expression_params_resolve_against_variables() {
        let (mut doc, blobs) = setup();
        let ab = doc.artboards()[0];
        doc.begin_txn("setup");
        doc.apply(
            OpKind::VariableSet { name: "gridSize".into(), value: Some(Value::F64(16.0)), prev: None },
            &blobs,
        )
        .unwrap();
        let id = doc.create_node(NodeKind::Shape, Some(ab), BTreeMap::new(), &blobs).unwrap();
        doc.apply(
            OpKind::ParamSet {
                node_id: id,
                path: "x".into(),
                value: Value::Expr("$gridSize * 2 + 1".into()),
                prev: None,
            },
            &blobs,
        )
        .unwrap();
        doc.commit_txn();
        assert_eq!(doc.param_f64(doc.node(id).unwrap(), "x", 0.0), 33.0);

        doc.apply_txn(
            "change var",
            OpKind::VariableSet { name: "gridSize".into(), value: Some(Value::F64(8.0)), prev: None },
            &blobs,
        )
        .unwrap();
        assert_eq!(doc.param_f64(doc.node(id).unwrap(), "x", 0.0), 17.0);
    }
}
