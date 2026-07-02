//! Operation schema (spec §3.2). The op log is append-only; undo appends
//! inverse ops (spec §3.3) — history is never rewritten.

use crate::node::{Modifier, Node, NodeKind};
use ed_core::{BlobHash, NodeId, OpId, TxnId, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Op {
    pub id: OpId,
    /// Causal deps — single local actor today: previous op id (spec §3.1).
    pub parents: Vec<OpId>,
    pub txn: TxnId,
    pub kind: OpKind,
}

/// One tile's before/after patch inside a paint op. Blob hashes address the
/// session blob store; `None` = tile absent (transparent).
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct TilePatch {
    pub tile: (u32, u32),
    pub before: Option<BlobHash>,
    pub after: Option<BlobHash>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum OpKind {
    NodeCreate {
        node: Box<Node>,
    },
    /// Tombstone: node detaches but its data is kept in the graveyard so
    /// undo/restore is exact (spec §3.1 deletion strategy).
    NodeDelete {
        node_id: NodeId,
    },
    NodeRestore {
        node_id: NodeId,
    },
    NodeMove {
        node_id: NodeId,
        new_parent: Option<NodeId>,
        frac_index: String,
        prev_parent: Option<NodeId>,
        prev_frac: String,
    },
    ParamSet {
        node_id: NodeId,
        path: String,
        value: Value,
        /// Stored previous value (spec §3.2) — makes the op invertible.
        prev: Option<Value>,
    },
    ModifierAttach {
        node_id: NodeId,
        modifier: Modifier,
        index: usize,
    },
    ModifierDetach {
        node_id: NodeId,
        modifier_id: u64,
        /// Filled on apply so the inverse can re-attach identically.
        removed: Option<(Modifier, usize)>,
    },
    ModifierReorder {
        node_id: NodeId,
        modifier_id: u64,
        new_index: usize,
        prev_index: usize,
    },
    /// Checkpointed raster edit — tile deltas (spec §2.5, §3.2).
    PaintTilePatch {
        node_id: NodeId,
        patches: Vec<TilePatch>,
    },
    /// Replace a StrokeSet's stroke list slice (append on paint).
    StrokesSet {
        node_id: NodeId,
        strokes: Vec<crate::node::Stroke>,
        prev: Vec<crate::node::Stroke>,
    },
    /// Resize bitmap canvas data wholesale (crop/resize); blob-addressed.
    BitmapReplace {
        node_id: NodeId,
        width: u32,
        height: u32,
        blob: Option<BlobHash>,
        prev_width: u32,
        prev_height: u32,
        prev_blob: Option<BlobHash>,
    },
    VariableSet {
        name: String,
        value: Option<Value>,
        prev: Option<Value>,
    },
    PaletteSet {
        name: String,
        value: Option<ed_core::Color>,
        prev: Option<ed_core::Color>,
    },
}

impl OpKind {
    /// The inverse op that undoes this one. Ops are made invertible at
    /// apply time (prev values filled in by `Document::apply`).
    pub fn inverse(&self) -> OpKind {
        match self.clone() {
            OpKind::NodeCreate { node } => OpKind::NodeDelete { node_id: node.id },
            OpKind::NodeDelete { node_id } => OpKind::NodeRestore { node_id },
            OpKind::NodeRestore { node_id } => OpKind::NodeDelete { node_id },
            OpKind::NodeMove { node_id, new_parent, frac_index, prev_parent, prev_frac } => {
                OpKind::NodeMove {
                    node_id,
                    new_parent: prev_parent,
                    frac_index: prev_frac,
                    prev_parent: new_parent,
                    prev_frac: frac_index,
                }
            }
            OpKind::ParamSet { node_id, path, value, prev } => OpKind::ParamSet {
                node_id,
                path,
                value: prev.unwrap_or(Value::Bool(false)),
                prev: Some(value),
            },
            OpKind::ModifierAttach { node_id, modifier, index } => OpKind::ModifierDetach {
                node_id,
                modifier_id: modifier.id,
                removed: Some((modifier, index)),
            },
            OpKind::ModifierDetach { node_id, removed, .. } => {
                let (modifier, index) = removed.expect("inverse of unapplied detach");
                OpKind::ModifierAttach { node_id, modifier, index }
            }
            OpKind::ModifierReorder { node_id, modifier_id, new_index, prev_index } => {
                OpKind::ModifierReorder {
                    node_id,
                    modifier_id,
                    new_index: prev_index,
                    prev_index: new_index,
                }
            }
            OpKind::PaintTilePatch { node_id, patches } => OpKind::PaintTilePatch {
                node_id,
                patches: patches
                    .into_iter()
                    .map(|p| TilePatch { tile: p.tile, before: p.after, after: p.before })
                    .collect(),
            },
            OpKind::StrokesSet { node_id, strokes, prev } => {
                OpKind::StrokesSet { node_id, strokes: prev, prev: strokes }
            }
            OpKind::BitmapReplace {
                node_id,
                width,
                height,
                blob,
                prev_width,
                prev_height,
                prev_blob,
            } => OpKind::BitmapReplace {
                node_id,
                width: prev_width,
                height: prev_height,
                blob: prev_blob,
                prev_width: width,
                prev_height: height,
                prev_blob: blob,
            },
            OpKind::VariableSet { name, value, prev } => {
                OpKind::VariableSet { name, value: prev, prev: value }
            }
            OpKind::PaletteSet { name, value, prev } => {
                OpKind::PaletteSet { name, value: prev, prev: value }
            }
        }
    }
}

/// A committed transaction: one user gesture (spec §3.3).
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Txn {
    pub id: TxnId,
    pub label: String,
    pub ops: Vec<Op>,
    /// Set when this txn is the undo/redo image of another txn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<TxnId>,
}

/// Node creation payload helper.
pub fn make_node(
    id: NodeId,
    kind: NodeKind,
    parent: Option<NodeId>,
    frac: String,
    params: BTreeMap<String, Value>,
) -> Node {
    let mut node = Node::new(id, kind);
    node.parent = parent;
    node.frac = frac;
    for (k, v) in params {
        node.params.insert(k, v);
    }
    node
}
