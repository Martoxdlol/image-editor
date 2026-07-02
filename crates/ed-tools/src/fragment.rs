//! Clipboard fragments (spec §10): a self-contained subtree snapshot with
//! its dependency closure. The internal clipboard holds it zero-copy in
//! the session; the same structure serializes to the system-clipboard
//! flavor (`application/x-myed-fragment+zip` payload, JSON body here).

use ed_core::{NodeId, Value, Vec2};
use ed_document::{doc::BlobStore, Document, Node, PaletteEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Fragment {
    /// Snapshot of the copied subtrees; parents precede children.
    /// Roots keep their original parent id for paste-in-place context.
    pub nodes: Vec<Node>,
    pub roots: Vec<NodeId>,
    /// Dependency closure (spec §10.2/§10.5).
    pub palette: Vec<PaletteEntry>,
    pub variables: BTreeMap<String, Value>,
    /// Bitmap tile pixels keyed by "nodeid/x,y" (content travels with the
    /// fragment so cross-document paste is self-contained).
    pub tiles: BTreeMap<String, Vec<u8>>,
    /// Param-referenced blob payloads (sel-mask data etc.), content-keyed.
    #[serde(default)]
    pub blobs: BTreeMap<String, Vec<u8>>,
    /// Bounds of the copied selection in source doc coords.
    pub bounds: (f64, f64, f64, f64),
    pub source_doc: String,
}

/// Collect the dependency closure of a set of nodes: which palette entries
/// and variables their params (and expressions) reference (spec §10.5).
fn collect_deps(doc: &Document, nodes: &[&Node]) -> (Vec<PaletteEntry>, BTreeMap<String, Value>) {
    let mut palette = Vec::new();
    let mut variables = BTreeMap::new();
    let want_palette = |name: &str, palette: &mut Vec<PaletteEntry>| {
        if let Some(e) = doc.palette.iter().find(|e| e.name == name) {
            if !palette.iter().any(|p: &PaletteEntry| p.name == e.name) {
                palette.push(e.clone());
            }
        }
    };
    for node in nodes {
        let all_params = node
            .params
            .iter()
            .chain(node.modifiers.iter().flat_map(|m| m.params.iter()));
        for (_, v) in all_params {
            match v {
                Value::Ref(ed_core::value::RefValue::Palette { entry }) => {
                    want_palette(entry, &mut palette)
                }
                Value::Ref(ed_core::value::RefValue::Variable { name }) => {
                    if let Some(val) = doc.variables.get(name) {
                        variables.insert(name.clone(), val.clone());
                    }
                }
                Value::Expr(src) => {
                    if let Ok(ast) = ed_document::expr::parse(src) {
                        for dep in ast.dependencies() {
                            if let Some(rest) = dep.strip_prefix("palette.") {
                                want_palette(rest, &mut palette);
                            } else if let Some(val) = doc.variables.get(&dep) {
                                variables.insert(dep.clone(), val.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (palette, variables)
}

/// Copy a node selection into a fragment (spec §10.3, node-selection row).
pub fn copy_nodes(
    doc: &Document,
    ids: &[NodeId],
    bounds: (f64, f64, f64, f64),
    blob_store: &BlobStore,
) -> Fragment {
    let mut nodes = Vec::new();
    let mut tiles = BTreeMap::new();
    let mut blobs = BTreeMap::new();
    // only top-most selected roots (skip ids inside another selected id)
    let roots: Vec<NodeId> = ids
        .iter()
        .copied()
        .filter(|&id| !ids.iter().any(|&other| other != id && doc.is_ancestor(other, id)))
        .collect();
    for &root in &roots {
        let mut order = Vec::new();
        doc.walk(root, &mut order);
        for id in order {
            if let Some(n) = doc.node(id) {
                nodes.push(n.clone());
                if let Some(bm) = &n.bitmap {
                    for (&(tx, ty), data) in &bm.tiles {
                        tiles.insert(format!("{id}/{tx},{ty}"), data.clone());
                    }
                }
                for hash in ed_document::serialize::node_blob_refs(n) {
                    if let Some(data) = blob_store.get(hash) {
                        blobs.insert(hash.to_string(), data.to_vec());
                    }
                }
            }
        }
    }
    let refs: Vec<&Node> = nodes.iter().collect();
    let (palette, variables) = collect_deps(doc, &refs);
    // strip heavy tile maps from the node snapshots (travel separately)
    let nodes = nodes
        .into_iter()
        .map(|mut n| {
            if let Some(bm) = n.bitmap.as_mut() {
                bm.tiles.clear();
            }
            n
        })
        .collect();
    Fragment {
        nodes,
        roots,
        palette,
        variables,
        tiles,
        blobs,
        bounds,
        source_doc: doc.name.clone(),
    }
}

pub struct PasteOptions {
    pub target_parent: Option<NodeId>,
    /// Offset applied to root positions (paste = +10,+10; in place = 0).
    pub offset: Vec2,
    /// Where to center the pasted content, if the source isn't visible.
    pub center_at: Option<Vec2>,
}

/// Paste a fragment as one transaction of ordinary ops (spec §10.4):
/// fresh ids, fractional indexes at the insertion point, dependency
/// resolution into target globals (§10.5). Returns the new root ids.
pub fn paste(
    doc: &mut Document,
    frag: &Fragment,
    opts: &PasteOptions,
    blobs: &mut BlobStore,
) -> Result<Vec<NodeId>, String> {
    doc.begin_txn("Paste");

    // param blobs (masks) land in the target's content-addressed store —
    // identical content dedupes to the same hash (spec §10.2)
    for data in frag.blobs.values() {
        blobs.put(data.clone());
    }

    // -------- dependency resolution (§10.5)
    for e in &frag.palette {
        match doc.palette.iter().find(|p| p.name == e.name) {
            Some(existing) if existing.color == e.color => {} // identical → reuse
            Some(_) => {
                // same name, different content → suffixed import
                let mut name = format!("{} (2)", e.name);
                let mut i = 2;
                while doc.palette.iter().any(|p| p.name == name) {
                    i += 1;
                    name = format!("{} ({i})", e.name);
                }
                doc.apply(
                    ed_document::OpKind::PaletteSet { name, value: Some(e.color), prev: None },
                    blobs,
                )?;
            }
            None => {
                doc.apply(
                    ed_document::OpKind::PaletteSet {
                        name: e.name.clone(),
                        value: Some(e.color),
                        prev: None,
                    },
                    blobs,
                )?;
            }
        }
    }
    for (name, value) in &frag.variables {
        if !doc.variables.contains_key(name) {
            doc.apply(
                ed_document::OpKind::VariableSet {
                    name: name.clone(),
                    value: Some(value.clone()),
                    prev: None,
                },
                blobs,
            )?;
        }
    }

    // -------- id remapping: every pasted node gets a fresh id (§10.4)
    let mut id_map: HashMap<NodeId, NodeId> = HashMap::new();
    for n in &frag.nodes {
        id_map.insert(n.id, doc.mint_node_id());
    }

    // paste offset: either explicit center or source position + offset
    let (bx, by, bw, bh) = frag.bounds;
    let delta = match opts.center_at {
        Some(c) => Vec2::new(c.x - (bx + bw / 2.0), c.y - (by + bh / 2.0)) + opts.offset,
        None => opts.offset,
    };

    let mut new_roots = Vec::new();
    for n in &frag.nodes {
        let new_id = id_map[&n.id];
        let is_root = frag.roots.contains(&n.id);
        let mut node = n.clone();
        node.id = new_id;
        node.parent = if is_root {
            opts.target_parent
        } else {
            n.parent.map(|p| id_map.get(&p).copied().unwrap_or(p))
        };
        node.frac = doc.frac_for_append(node.parent);
        // remap node refs in params (mask/component references)
        for (_, v) in node.params.iter_mut() {
            if let Value::Ref(ed_core::value::RefValue::Node { id }) = v {
                if let Some(new) = id_map.get(id) {
                    *id = *new;
                }
            }
        }
        for m in node.modifiers.iter_mut() {
            for (_, v) in m.params.iter_mut() {
                if let Value::Ref(ed_core::value::RefValue::Node { id }) = v {
                    if let Some(new) = id_map.get(id) {
                        *id = *new;
                    }
                }
            }
        }
        // offset root geometry
        if is_root && delta != Vec2::ZERO {
            for key in ["x", "y", "x2", "y2"] {
                if let Some(Value::F64(v)) = node.params.get(key).cloned() {
                    let d = if key.starts_with('x') { delta.x } else { delta.y };
                    node.params.insert(key.into(), Value::F64(v + d));
                }
            }
            for s in node.strokes.iter_mut() {
                for p in s.points.iter_mut() {
                    p.pos = p.pos + delta;
                }
            }
        }
        doc.apply(ed_document::OpKind::NodeCreate { node: Box::new(node) }, blobs)?;

        // restore bitmap tiles via tile-patch ops (ordinary ops rule §10.4)
        if n.bitmap.is_some() {
            let mut patches = Vec::new();
            let prefix = format!("{}/", n.id);
            for (key, data) in frag.tiles.range(prefix.clone()..) {
                if !key.starts_with(&prefix) {
                    break;
                }
                let coords = &key[prefix.len()..];
                if let Some((x, y)) = coords.split_once(',') {
                    if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                        let hash = blobs.put(data.clone());
                        patches.push(ed_document::TilePatch {
                            tile: (x, y),
                            before: None,
                            after: Some(hash),
                        });
                    }
                }
            }
            if !patches.is_empty() {
                doc.apply(
                    ed_document::OpKind::PaintTilePatch { node_id: new_id, patches },
                    blobs,
                )?;
            }
        }
        if is_root {
            new_roots.push(new_id);
        }
    }
    doc.commit_txn();
    Ok(new_roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed_core::{ActorId, Color};
    use ed_document::NodeKind;

    #[test]
    fn copy_paste_roundtrip_with_deps() {
        let mut blobs = BlobStore::default();
        let mut doc = Document::with_artboard(ActorId(1), "a", 400.0, 300.0, &blobs);
        let ab = doc.artboards()[0];
        doc.begin_txn("build");
        doc.apply(
            ed_document::OpKind::PaletteSet {
                name: "accent".into(),
                value: Some(Color::from_hex("#ff0000").unwrap()),
                prev: None,
            },
            &blobs,
        )
        .unwrap();
        let mut params = BTreeMap::new();
        params.insert("x".into(), Value::F64(10.0));
        params.insert("y".into(), Value::F64(20.0));
        params.insert("w".into(), Value::F64(30.0));
        params.insert("h".into(), Value::F64(30.0));
        params.insert(
            "fill-color".into(),
            Value::Ref(ed_core::value::RefValue::Palette { entry: "accent".into() }),
        );
        let shape = doc.create_node(NodeKind::Shape, Some(ab), params, &blobs).unwrap();
        doc.commit_txn();

        let frag = copy_nodes(&doc, &[shape], (10.0, 20.0, 30.0, 30.0), &blobs);
        assert_eq!(frag.nodes.len(), 1);
        assert_eq!(frag.palette.len(), 1, "palette dep captured");

        // paste into a DIFFERENT document: dep should be imported
        let mut doc2 = Document::with_artboard(ActorId(1), "b", 400.0, 300.0, &blobs);
        let ab2 = doc2.artboards()[0];
        let roots = paste(
            &mut doc2,
            &frag,
            &PasteOptions {
                target_parent: Some(ab2),
                offset: Vec2::new(10.0, 10.0),
                center_at: None,
            },
            &mut blobs,
        )
        .unwrap();
        assert_eq!(roots.len(), 1);
        assert!(doc2.palette.iter().any(|p| p.name == "accent"), "palette imported");
        let n = doc2.node(roots[0]).unwrap();
        assert_eq!(doc2.param_f64(n, "x", 0.0), 20.0, "offset applied");
        // resolved fill still follows the palette ref
        assert_eq!(
            doc2.param_color(n, "fill-color", Color::BLACK).to_hex(),
            "#ff0000"
        );
        // undo removes everything the paste created (one txn)
        doc2.undo(&blobs);
        assert!(doc2.node(roots[0]).is_none());

        // same-document paste mints fresh ids (spec §10.4)
        let roots_same = paste(
            &mut doc,
            &frag,
            &PasteOptions { target_parent: Some(ab), offset: Vec2::new(10.0, 10.0), center_at: None },
            &mut blobs,
        )
        .unwrap();
        assert_ne!(roots_same[0], shape);
        assert!(doc.node(shape).is_some());
        assert!(doc.node(roots_same[0]).is_some());
    }
}
