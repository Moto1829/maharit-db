use std::hash::{Hash, Hasher};

/// プロパティの値を表す列挙型
#[derive(Debug, Clone)]
pub enum PropertyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

impl PartialEq for PropertyValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PropertyValue::Null, PropertyValue::Null) => true,
            (PropertyValue::Bool(a), PropertyValue::Bool(b)) => a == b,
            (PropertyValue::Int(a), PropertyValue::Int(b)) => a == b,
            (PropertyValue::Float(a), PropertyValue::Float(b)) => a.to_bits() == b.to_bits(),
            (PropertyValue::String(a), PropertyValue::String(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for PropertyValue {}

impl Hash for PropertyValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            PropertyValue::Null => {}
            PropertyValue::Bool(v) => v.hash(state),
            PropertyValue::Int(v) => v.hash(state),
            PropertyValue::Float(v) => v.to_bits().hash(state),
            PropertyValue::String(v) => v.hash(state),
        }
    }
}

impl From<bool> for PropertyValue {
    fn from(v: bool) -> Self {
        PropertyValue::Bool(v)
    }
}

impl From<i64> for PropertyValue {
    fn from(v: i64) -> Self {
        PropertyValue::Int(v)
    }
}

impl From<i32> for PropertyValue {
    fn from(v: i32) -> Self {
        PropertyValue::Int(v as i64)
    }
}

impl From<f64> for PropertyValue {
    fn from(v: f64) -> Self {
        PropertyValue::Float(v)
    }
}

impl From<String> for PropertyValue {
    fn from(v: String) -> Self {
        PropertyValue::String(v)
    }
}

impl From<&str> for PropertyValue {
    fn from(v: &str) -> Self {
        PropertyValue::String(v.to_string())
    }
}

impl std::fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropertyValue::Null => write!(f, "null"),
            PropertyValue::Bool(v) => write!(f, "{}", v),
            PropertyValue::Int(v) => write!(f, "{}", v),
            PropertyValue::Float(v) => write!(f, "{}", v),
            PropertyValue::String(v) => write!(f, "\"{}\"", v),
        }
    }
}
