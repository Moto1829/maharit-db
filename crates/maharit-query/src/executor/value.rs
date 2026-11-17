//! クエリ結果の値 `Value`、行 `Row`、表 `ResultSet` と、
//! `PropertyValue` / `Literal` との相互変換。

use super::*;

/// 実行結果の行
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub columns: Vec<Value>,
}

/// 実行結果のテーブル
#[derive(Debug, Clone, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

impl ResultSet {
    pub fn empty() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }

    pub fn new(columns: Vec<String>, rows: Vec<Row>) -> Self {
        Self { columns, rows }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

/// 実行時の値
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Node(NodeId),
    /// リレーションシップの値（`relationships(p)` の要素、`collect(r)` など）。
    /// 出力（JSON）は従来どおり ID の整数。
    Edge(u64),
    NodeData {
        id: NodeId,
        /// ノードが保持するラベルのリスト（複数ラベル対応）
        labels: Vec<String>,
        properties: Arc<HashMap<String, PropertyValue>>,
    },
    /// リスト値（可変長パスのエッジリストなど）
    List(Vec<Value>),
    /// パス値（ノードとエッジの交互シーケンス）
    Path {
        nodes: Vec<NodeId>,
        edges: Vec<u64>,
    },
    /// 日付 (1970-01-01 からの日数)
    Date(i32),
    /// 日時 (Unix エポックからのミリ秒)
    DateTime(i64),
    /// 期間
    Duration {
        months: i32,
        days: i32,
        millis: i64,
    },
    /// マップ値: {key: value, ...}
    Map(std::collections::HashMap<String, Value>),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Node(id) => write!(f, "Node({})", id),
            Value::Edge(id) => write!(f, "{}", id),
            Value::NodeData { id, labels, .. } => {
                write!(f, "({}", id)?;
                for lbl in labels {
                    write!(f, ":{}", lbl)?;
                }
                write!(f, ")")
            }
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Path { nodes, edges } => {
                write!(f, "Path(nodes: {:?}, edges: {:?})", nodes, edges)
            }
            Value::Date(days) => {
                let (y, m, d) = maharit_core::temporal::days_to_ymd(*days);
                write!(f, "{:04}-{:02}-{:02}", y, m, d)
            }
            Value::DateTime(ms) => {
                let (y, mo, d, h, mi, s, frac) = maharit_core::temporal::millis_to_datetime(*ms);
                write!(
                    f,
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                    y, mo, d, h, mi, s, frac
                )
            }
            Value::Duration {
                months,
                days,
                millis,
            } => {
                write!(
                    f,
                    "{}",
                    maharit_core::temporal::duration_to_string(*months, *days, *millis)
                )
            }
            Value::Map(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

impl Value {
    /// Cypher 値を `serde_json::Value` に変換する。
    ///
    /// プリミティブ (Null/Bool/Int/Float/String) は対応する JSON 型を維持。
    /// List/Map は再帰的に変換、Date/DateTime/Duration/Node/NodeData/Path 等は
    /// 既存の Display 文字列表現で JSON 文字列として返す（型を区別したい場合は
    /// 専用のオブジェクト表現に変更可能だが現状はシンプル化を優先）。
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Int(n) => serde_json::Value::Number((*n).into()),
            // 従来どおり ID の整数
            Value::Edge(id) => serde_json::Value::Number((*id).into()),
            Value::Float(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::String(s) => serde_json::Value::String(s.clone()),
            Value::List(items) => {
                serde_json::Value::Array(items.iter().map(|v| v.to_json()).collect())
            }
            Value::Map(map) => {
                let obj: serde_json::Map<String, serde_json::Value> =
                    map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
                serde_json::Value::Object(obj)
            }
            // 非プリミティブ型は Display 文字列で表現（後方互換、UI で読み取り可能）
            Value::Node(_)
            | Value::NodeData { .. }
            | Value::Path { .. }
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::Duration { .. } => serde_json::Value::String(self.to_string_unquoted()),
        }
    }

    /// `serde_json::Value` を Cypher 値に変換する（クエリパラメータの受け取り用）。
    ///
    /// 整数に収まる数値は `Int`、それ以外の数値は `Float`。配列は `List`、
    /// オブジェクトは `Map` に再帰的に変換する。
    pub fn from_json(json: &serde_json::Value) -> Value {
        match json {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Value::Int(i),
                None => n.as_f64().map(Value::Float).unwrap_or(Value::Null),
            },
            serde_json::Value::String(s) => Value::String(s.clone()),
            serde_json::Value::Array(items) => {
                Value::List(items.iter().map(Value::from_json).collect())
            }
            serde_json::Value::Object(obj) => Value::Map(
                obj.iter()
                    .map(|(k, v)| (k.clone(), Value::from_json(v)))
                    .collect(),
            ),
        }
    }

    /// Display と違って文字列値の外側ダブルクォートを付けない文字列化。
    /// JSON 化用途以外でも、ログ・診断メッセージで素のままを欲しい場面で使う。
    pub fn to_string_unquoted(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            _ => self.to_string(),
        }
    }
}

impl From<&PropertyValue> for Value {
    fn from(pv: &PropertyValue) -> Self {
        match pv {
            PropertyValue::Null => Value::Null,
            PropertyValue::Bool(b) => Value::Bool(*b),
            PropertyValue::Int(n) => Value::Int(*n),
            PropertyValue::Float(n) => Value::Float(*n),
            PropertyValue::String(s) => Value::String(s.clone()),
            PropertyValue::Date(d) => Value::Date(*d),
            PropertyValue::DateTime(ms) => Value::DateTime(*ms),
            PropertyValue::Duration {
                months,
                days,
                millis,
            } => Value::Duration {
                months: *months,
                days: *days,
                millis: *millis,
            },
            PropertyValue::List(items) => Value::List(items.iter().map(Value::from).collect()),
        }
    }
}

impl From<Literal> for Value {
    fn from(lit: Literal) -> Self {
        match lit {
            Literal::Null => Value::Null,
            Literal::Bool(b) => Value::Bool(b),
            Literal::Int(n) => Value::Int(n),
            Literal::Float(n) => Value::Float(n),
            Literal::String(s) => Value::String(s),
        }
    }
}

impl From<Literal> for PropertyValue {
    fn from(lit: Literal) -> Self {
        match lit {
            Literal::Null => PropertyValue::Null,
            Literal::Bool(b) => PropertyValue::Bool(b),
            Literal::Int(n) => PropertyValue::Int(n),
            Literal::Float(n) => PropertyValue::Float(n),
            Literal::String(s) => PropertyValue::String(s),
        }
    }
}
