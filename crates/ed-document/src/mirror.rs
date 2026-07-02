//! Read-model mirrors (spec §12.1): lightweight JSON projections of the
//! document that React renders from. The UI never touches the tree itself.

use crate::doc::Document;
use crate::node::NodeKind;
use ed_core::{NodeId, Value};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OutlineNode {
    pub id: String,
    pub kind: NodeKind,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    pub blend: String,
    pub modifier_badges: Vec<String>,
    pub children: Vec<OutlineNode>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: u64,
    pub label: String,
    pub undo_of: Option<u64>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModifierMirror {
    pub id: u64,
    pub kind: String,
    pub enabled: bool,
    pub params: serde_json::Value,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PropsMirror {
    pub id: String,
    pub kind: NodeKind,
    pub params: serde_json::Value,
    pub modifiers: Vec<ModifierMirror>,
}

impl Document {
    pub fn outline(&self) -> Vec<OutlineNode> {
        self.children_of(None).iter().map(|&id| self.outline_node(id)).collect()
    }

    fn outline_node(&self, id: NodeId) -> OutlineNode {
        let n = &self.nodes[&id];
        OutlineNode {
            id: id.to_string(),
            kind: n.kind,
            name: n.name().to_string(),
            visible: n.visible(),
            locked: n.locked(),
            opacity: n.opacity(),
            blend: self.param_str(n, "blend", "normal"),
            modifier_badges: n.modifiers.iter().map(|m| m.kind.clone()).collect(),
            children: self
                .children_of(Some(id))
                .iter()
                .map(|&c| self.outline_node(c))
                .collect(),
        }
    }

    pub fn history_mirror(&self) -> Vec<HistoryEntry> {
        self.history
            .iter()
            .map(|t| HistoryEntry { id: t.id.0, label: t.label.clone(), undo_of: t.undo_of.map(|u| u.0) })
            .collect()
    }

    pub fn props_mirror(&self) -> Vec<PropsMirror> {
        self.selected_nodes
            .iter()
            .filter_map(|id| {
                let n = self.node(*id)?;
                Some(PropsMirror {
                    id: id.to_string(),
                    kind: n.kind,
                    params: params_to_json(&n.params),
                    modifiers: n
                        .modifiers
                        .iter()
                        .map(|m| ModifierMirror {
                            id: m.id,
                            kind: m.kind.clone(),
                            enabled: m.enabled,
                            params: params_to_json(&m.params),
                        })
                        .collect(),
                })
            })
            .collect()
    }
}

fn params_to_json(params: &std::collections::BTreeMap<String, Value>) -> serde_json::Value {
    serde_json::to_value(params).unwrap_or(serde_json::Value::Null)
}
