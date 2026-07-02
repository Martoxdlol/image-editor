use crate::{BlobHash, Color, Mat3, NodeId, Vec2};
use serde::{Deserialize, Serialize};

/// Param values (spec §3.4). Every tool/modifier param is one of these;
/// property panels auto-render from the typed schema.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(tag = "t", content = "v", rename_all = "kebab-case")]
pub enum Value {
    F64(f64),
    Bool(bool),
    Str(String),
    Color(Color),
    Point(Vec2),
    Matrix(Mat3),
    Ref(RefValue),
    /// Expression source (spec §8) — parsed/validated at input time,
    /// stored as source, evaluated against document globals.
    Expr(String),
    Blob(BlobHash),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RefValue {
    Palette { entry: String },
    Variable { name: String },
    Node { id: NodeId },
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::F64(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_color(&self) -> Option<Color> {
        match self {
            Value::Color(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_point(&self) -> Option<Vec2> {
        match self {
            Value::Point(v) => Some(*v),
            _ => None,
        }
    }
}
