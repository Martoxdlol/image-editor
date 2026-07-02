//! Document snapshot serialization (spec §11 `document.json`).
//! Bitmap tile pixels are content-addressed blobs stored beside the
//! snapshot (`blobs/<hash>`); the snapshot references hashes only.

use crate::doc::{BlobStore, Document, PaletteEntry};
use crate::node::Node;
use ed_core::{ActorId, BlobHash, NodeId, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug)]
pub struct TileRef {
    pub x: u32,
    pub y: u32,
    pub blob: BlobHash,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Snapshot {
    pub format: u32,
    pub name: String,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub tiles: BTreeMap<String, Vec<TileRef>>,
    /// Blobs referenced by `Value::Blob` params (masks etc.) — persisted
    /// alongside tile blobs in the container (spec §3.2 content addressing).
    #[serde(default)]
    pub param_blobs: Vec<BlobHash>,
    #[serde(default)]
    pub variables: BTreeMap<String, Value>,
    #[serde(default)]
    pub palette: Vec<PaletteEntry>,
}

/// All `Value::Blob` references reachable from a node's params/modifiers.
pub fn node_blob_refs(node: &Node) -> Vec<BlobHash> {
    node.params
        .values()
        .chain(node.modifiers.iter().flat_map(|m| m.params.values()))
        .filter_map(|v| match v {
            Value::Blob(h) => Some(*h),
            _ => None,
        })
        .collect()
}

impl Document {
    /// Compact tree snapshot; tile pixels go into `blobs`.
    pub fn to_snapshot(&self, blobs: &mut BlobStore) -> Snapshot {
        let mut nodes = Vec::new();
        let mut tiles = BTreeMap::new();
        let mut param_blobs = Vec::new();
        // Depth-first so parents come before children on load.
        let mut stack: Vec<NodeId> = self.children_of(None).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let n = &self.nodes[&id];
            nodes.push(n.clone());
            param_blobs.extend(node_blob_refs(n));
            if let Some(bm) = &n.bitmap {
                let refs: Vec<TileRef> = bm
                    .tiles
                    .iter()
                    .map(|(&(x, y), data)| TileRef { x, y, blob: blobs.put(data.clone()) })
                    .collect();
                if !refs.is_empty() {
                    tiles.insert(id.to_string(), refs);
                }
            }
            for &c in self.children_of(Some(id)).iter().rev() {
                stack.push(c);
            }
        }
        param_blobs.sort();
        param_blobs.dedup();
        Snapshot {
            format: FORMAT_VERSION,
            name: self.name.clone(),
            nodes,
            tiles,
            param_blobs,
            variables: self.variables.clone(),
            palette: self.palette.clone(),
        }
    }

    pub fn from_snapshot(snap: Snapshot, actor: ActorId, blobs: &BlobStore) -> Result<Document, String> {
        let mut doc = Document::new(actor, &snap.name);
        doc.variables = snap.variables;
        doc.palette = snap.palette;
        let mut max_counter = 0u64;
        for mut node in snap.nodes {
            if node.id.actor == actor.0 {
                max_counter = max_counter.max(node.id.counter);
            }
            if let Some(refs) = snap.tiles.get(&node.id.to_string()) {
                if let Some(bm) = node.bitmap.as_mut() {
                    for r in refs {
                        let data = blobs
                            .get(r.blob)
                            .ok_or_else(|| format!("missing tile blob {}", r.blob))?;
                        bm.tiles.insert((r.x, r.y), data.to_vec());
                    }
                }
            }
            doc.insert_loaded_node(node);
        }
        doc.set_counters(max_counter);
        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeKind;
    use crate::ops::OpKind;
    use ed_core::Color;

    #[test]
    fn snapshot_roundtrip() {
        let mut blobs = BlobStore::default();
        let actor = ActorId(1);
        let mut doc = Document::with_artboard(actor, "test", 800.0, 600.0, &blobs);
        let ab = doc.artboards()[0];

        doc.begin_txn("add shape");
        let mut params = BTreeMap::new();
        params.insert("shape".into(), Value::Str("rect".into()));
        params.insert("x".into(), Value::F64(10.0));
        params.insert("fill-color".into(), Value::Color(Color::from_hex("#3366ff").unwrap()));
        let shape = doc.create_node(NodeKind::Shape, Some(ab), params, &blobs).unwrap();
        doc.commit_txn();

        doc.begin_txn("paint");
        let bm_params = BTreeMap::new();
        let bmp = doc.create_node(NodeKind::Bitmap, Some(ab), bm_params, &blobs).unwrap();
        doc.commit_txn();
        {
            let n = doc.nodes.get_mut(&bmp).unwrap();
            let bm = n.bitmap.as_mut().unwrap();
            bm.width = 100;
            bm.height = 100;
            bm.set_pixel(5, 5, [255, 0, 0, 255]);
        }

        doc.begin_txn("globals");
        doc.apply(
            OpKind::VariableSet { name: "gridSize".into(), value: Some(Value::F64(8.0)), prev: None },
            &blobs,
        )
        .unwrap();
        doc.apply(
            OpKind::PaletteSet { name: "accent".into(), value: Some(Color::from_hex("#ff0000").unwrap()), prev: None },
            &blobs,
        )
        .unwrap();
        doc.commit_txn();

        let snap = doc.to_snapshot(&mut blobs);
        let json = serde_json::to_string(&snap).unwrap();
        let snap2: Snapshot = serde_json::from_str(&json).unwrap();
        let doc2 = Document::from_snapshot(snap2, actor, &blobs).unwrap();

        assert_eq!(doc2.artboards().len(), 1);
        let n = doc2.node(shape).unwrap();
        assert_eq!(doc2.param_f64(n, "x", 0.0), 10.0);
        let bm = doc2.node(bmp).unwrap().bitmap.as_ref().unwrap();
        assert_eq!(bm.get_pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(doc2.variables.get("gridSize"), Some(&Value::F64(8.0)));
        assert_eq!(doc2.palette.len(), 1);

        // fresh ids won't collide with loaded ones
        let fresh = doc2.clone_id_probe();
        assert!(fresh.counter > bmp.counter);
    }
}
