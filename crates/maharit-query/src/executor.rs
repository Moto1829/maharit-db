use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use maharit_core::{
    Constraint, ConstraintError, ConstraintManager, ConstraintType, ConcurrentGraph, Edge,
    FulltextError, FulltextManager, Graph, GraphBackend, IndexDefinition, NodeId, PropertyIndex,
    PropertyType, PropertyValue, traversal,
};
use rayon::prelude::*;
use regex::Regex;
use thiserror::Error;

use crate::ast::*;
use crate::cache::AstCache;

/// Minimum number of candidate nodes required before switching to parallel filtering.
///
/// Below this threshold the overhead of spawning rayon tasks exceeds the benefit.
const PARALLEL_MATCH_THRESHOLD: usize = 500;

/// Compute Levenshtein edit distance between two strings (used for fuzzy CONTAINS).
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m < n {
        return levenshtein_distance(b, a);
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// 実行エラー
#[derive(Debug, Clone, Error, PartialEq)]
pub enum ExecuteError {
    #[error("undefined variable: {0}")]
    UndefinedVariable(String),

    #[error("type error: {0}")]
    TypeError(String),

    #[error("graph error: {0}")]
    GraphError(#[from] maharit_core::GraphError),

    #[error("constraint error: {0}")]
    ConstraintError(#[from] ConstraintError),

    #[error("fulltext error: {0}")]
    FulltextError(#[from] FulltextError),

    #[error("parse error: {0}")]
    ParseError(#[from] crate::parser::ParseError),
}

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
                write!(f, "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, mo, d, h, mi, s, frac)
            }
            Value::Duration { months, days, millis } => {
                write!(f, "{}", maharit_core::temporal::duration_to_string(*months, *days, *millis))
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
            Value::Float(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::String(s) => serde_json::Value::String(s.clone()),
            Value::List(items) => {
                serde_json::Value::Array(items.iter().map(|v| v.to_json()).collect())
            }
            Value::Map(map) => {
                let obj: serde_json::Map<String, serde_json::Value> = map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect();
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
            PropertyValue::Duration { months, days, millis } => Value::Duration {
                months: *months,
                days: *days,
                millis: *millis,
            },
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

/// バインディング値
#[derive(Debug, Clone, PartialEq)]
pub enum BindingValue {
    /// 単一ノード
    Node(NodeId),
    /// 単一エッジ
    Edge(u64),
    /// パス（可変長パス用）
    Path { nodes: Vec<NodeId>, edges: Vec<u64> },
    /// スカラー値（WITH句で渡される中間値）
    Scalar(Value),
}

impl BindingValue {
    /// ノードIDを取得（Nodeの場合のみ）
    pub fn as_node(&self) -> Option<NodeId> {
        match self {
            BindingValue::Node(id) => Some(*id),
            _ => None,
        }
    }

    /// エッジIDを取得（Edgeの場合のみ）
    pub fn as_edge(&self) -> Option<u64> {
        match self {
            BindingValue::Edge(id) => Some(*id),
            _ => None,
        }
    }
}

/// 変数バインディング
type Bindings = HashMap<String, BindingValue>;

/// クエリエグゼキュータ
pub struct Executor<'a> {
    /// Raw pointer to the graph backend.  Always non-null and valid for lifetime `'a`.
    ///
    /// Stored as `*mut dyn GraphBackend` so the same field can hold either a
    /// `Graph` or a `ConcurrentGraph`:
    ///
    /// * **Writable (`Graph`)**: derived from `&'a mut Graph`.  `graph_mut()` may
    ///   produce `&mut dyn GraphBackend` because the caller holds an exclusive lock.
    ///
    /// * **Read-only**: derived from `&'a Graph` (or `&'a ConcurrentGraph`).
    ///   Only `graph_ref()` is ever called; the `*mut` cast does not by itself
    ///   create UB.
    ///
    /// * **ConcurrentGraph**: derived from `&'a ConcurrentGraph` with a const→mut
    ///   pointer cast.  Write methods on `ConcurrentGraph` use DashMap interior
    ///   mutability, so calling `graph_mut()` is safe even though the original
    ///   reference was shared.
    graph: *mut dyn GraphBackend,
    readonly: bool,
    _marker: std::marker::PhantomData<&'a ()>,
    constraints: ConstraintManager,
    fulltext: FulltextManager,
    property_index: PropertyIndex,
    params: HashMap<String, Value>,
}

// SAFETY: The raw `*mut Graph` pointer inside `Executor` is derived from a
// valid `&mut Graph` or `&Graph` and is only ever accessed through the
// `graph_ref()` / `graph_mut()` helpers, which enforce the correct access
// mode.  The underlying `Graph` type is `Sync` (no interior mutability), so
// sharing `&Executor<'_>` across threads — as rayon does for parallel
// pattern matching — is safe.
unsafe impl<'a> Sync for Executor<'a> {}

impl<'a> Executor<'a> {
    pub fn new(graph: &'a mut Graph) -> Self {
        let g: &mut dyn GraphBackend = graph;
        Self {
            graph: g as *mut dyn GraphBackend,
            readonly: false,
            _marker: std::marker::PhantomData,
            constraints: ConstraintManager::new(),
            fulltext: FulltextManager::new(),
            property_index: PropertyIndex::new(),
            params: HashMap::new(),
        }
    }

    /// Create an `Executor` for read-only query evaluation.
    ///
    /// Allows the caller to pass a shared `&Graph` reference (e.g. obtained
    /// from a `RwLockReadGuard`) without requiring an exclusive write lock.
    /// Multiple read-only executors can therefore run concurrently against the
    /// same graph.
    ///
    /// # Safety
    ///
    /// The caller must guarantee **both** of the following:
    ///
    /// 1. The query is confirmed to be read-only by [`maharit_query::is_read_only`].
    /// 2. The graph will not be mutated (by any thread) while this executor is
    ///    alive — typically enforced by holding a `RwLockReadGuard`.
    ///
    /// Only `graph_ref()` is called in read-only mode; the `graph_mut()`
    /// accessor is never reachable through read-only statement handlers.
    pub unsafe fn new_readonly(graph: &'a Graph) -> Self {
        // Pointer cast from *const to *mut is legal and does not by itself
        // create undefined behaviour.  UB would only arise if we subsequently
        // derived `&mut dyn GraphBackend` from this pointer while another reference
        // exists, which we never do in readonly mode.
        let g: &dyn GraphBackend = graph;
        Self {
            graph: (g as *const dyn GraphBackend) as *mut dyn GraphBackend,
            readonly: true,
            _marker: std::marker::PhantomData,
            constraints: ConstraintManager::new(),
            fulltext: FulltextManager::new(),
            property_index: PropertyIndex::new(),
            params: HashMap::new(),
        }
    }

    /// Create an `Executor` for use with a [`ConcurrentGraph`].
    ///
    /// Unlike [`new`] (which requires `&mut Graph`), this constructor accepts
    /// a shared reference because `ConcurrentGraph` uses DashMap interior
    /// mutability for all write operations.
    ///
    /// # Safety
    ///
    /// The caller must ensure that no other thread concurrently reads a
    /// structural snapshot (e.g. for rollback) while this executor is running
    /// a write query.  Concurrent reads from other executors are safe because
    /// DashMap provides fine-grained shard locking.
    pub unsafe fn new_concurrent(graph: &'a ConcurrentGraph) -> Self {
        let g: &dyn GraphBackend = graph;
        Self {
            graph: (g as *const dyn GraphBackend) as *mut dyn GraphBackend,
            readonly: false,
            _marker: std::marker::PhantomData,
            constraints: ConstraintManager::new(),
            fulltext: FulltextManager::new(),
            property_index: PropertyIndex::new(),
            params: HashMap::new(),
        }
    }

    /// Create an `Executor` for use with a [`ConcurrentGraph`], pre-populated
    /// with existing constraint and fulltext managers for state persistence.
    ///
    /// # Safety
    ///
    /// Same safety requirements as [`new_concurrent`].
    pub unsafe fn new_concurrent_with_managers(
        graph: &'a ConcurrentGraph,
        constraints: ConstraintManager,
        fulltext: FulltextManager,
    ) -> Self {
        let g: &dyn GraphBackend = graph;
        Self {
            graph: (g as *const dyn GraphBackend) as *mut dyn GraphBackend,
            readonly: false,
            _marker: std::marker::PhantomData,
            constraints,
            fulltext,
            property_index: PropertyIndex::new(),
            params: HashMap::new(),
        }
    }

    /// Consume the executor and return the constraint and fulltext managers.
    ///
    /// Use this after `execute()` to persist updated manager state.
    pub fn into_managers(self) -> (ConstraintManager, FulltextManager) {
        (self.constraints, self.fulltext)
    }

    /// Return a shared reference to the graph backend.
    ///
    /// Safe to call in both writable and read-only mode.
    #[inline]
    fn graph_ref(&self) -> &dyn GraphBackend {
        // SAFETY: `self.graph` is always a valid, non-null, properly aligned
        // fat pointer derived from a valid `&mut dyn GraphBackend` or `&dyn GraphBackend`.
        // Creating `&T` from a raw pointer is safe as long as the pointee is valid,
        // which it is for the entire lifetime `'a`.
        unsafe { &*self.graph }
    }

    /// Return an exclusive reference to the graph backend.
    ///
    /// Must only be called when `self.readonly == false`.  For `ConcurrentGraph`
    /// this is always safe because writes use DashMap interior mutability.
    #[inline]
    fn graph_mut(&mut self) -> &mut dyn GraphBackend {
        debug_assert!(!self.readonly, "write operation called on read-only executor");
        // SAFETY: In non-readonly mode the pointer was derived from either
        // `&mut Graph` (exclusive lock held by caller) or `&ConcurrentGraph`
        // (interior mutability via DashMap).  In both cases producing `&mut`
        // is safe for the duration of this call.
        unsafe { &mut *self.graph }
    }

    /// パラメータ付きクエリを実行
    pub fn execute_with_params(
        &mut self,
        stmt: Statement,
        params: HashMap<String, Value>,
    ) -> Result<ResultSet, ExecuteError> {
        self.params = params;
        let result = self.execute(stmt);
        self.params = HashMap::new();
        result
    }

    /// AstCache を利用してクエリ文字列をパース・キャッシュし、パラメータ付きで実行する。
    ///
    /// 同じクエリ文字列を繰り返し実行する場合、2回目以降はパースをスキップして
    /// キャッシュ済みの AST を再利用する。パラメータが異なっても同じ実行計画を使用できる。
    ///
    /// # Arguments
    ///
    /// * `query` - 実行するクエリ文字列
    /// * `params` - クエリパラメータ（`$param` 形式の変数に対応）
    /// * `cache` - AST キャッシュ（複数回の呼び出し間で共有する）
    ///
    /// # Errors
    ///
    /// * [`ExecuteError::ParseError`] - クエリのパースに失敗した場合
    /// * その他の [`ExecuteError`] - クエリの実行に失敗した場合
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut cache = AstCache::new(100);
    /// let mut params = HashMap::new();
    /// params.insert("name".to_string(), Value::String("Alice".to_string()));
    ///
    /// let result = executor.execute_cached(
    ///     "MATCH (n:Person) WHERE n.name = $name RETURN n",
    ///     params,
    ///     &mut cache,
    /// )?;
    /// ```
    pub fn execute_cached(
        &mut self,
        query: &str,
        params: HashMap<String, Value>,
        cache: &mut AstCache,
    ) -> Result<ResultSet, ExecuteError> {
        let stmt = cache.get_or_parse(query)?;
        self.execute_with_params(stmt, params)
    }

    /// 制約マネージャーへの参照を取得
    pub fn constraint_manager(&self) -> &ConstraintManager {
        &self.constraints
    }

    /// 制約マネージャーへの可変参照を取得
    pub fn constraint_manager_mut(&mut self) -> &mut ConstraintManager {
        &mut self.constraints
    }

    /// 全文検索マネージャーへの参照を取得
    pub fn fulltext_manager(&self) -> &FulltextManager {
        &self.fulltext
    }

    /// 全文検索マネージャーへの可変参照を取得
    pub fn fulltext_manager_mut(&mut self) -> &mut FulltextManager {
        &mut self.fulltext
    }

    /// プロパティインデックスへの参照を取得
    pub fn property_index(&self) -> &PropertyIndex {
        &self.property_index
    }

    /// プロパティインデックスへの可変参照を取得
    pub fn property_index_mut(&mut self) -> &mut PropertyIndex {
        &mut self.property_index
    }

    /// 文を実行
    pub fn execute(&mut self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
        match stmt {
            Statement::Create(create) => self.execute_create(create),
            Statement::Match(m) => self.execute_match(m),
            Statement::Delete(d) => self.execute_delete(d),
            Statement::Union(u) => self.execute_union(u),
            Statement::MatchCreate(mc) => self.execute_match_create(mc),
            Statement::MatchSet(ms) => self.execute_match_set(ms),
            Statement::Merge(merge) => self.execute_merge(merge),
            Statement::MatchRemove(mr) => self.execute_match_remove(mr),
            Statement::Unwind(uw) => self.execute_unwind(uw),
            Statement::Foreach(f) => self.execute_foreach_stmt_ref(&f, &Bindings::new()),
            Statement::MatchForeach(mf) => self.execute_match_foreach(mf),
            Statement::CreateConstraint(cc) => self.execute_create_constraint(cc),
            Statement::DropConstraint(dc) => self.execute_drop_constraint(dc),
            Statement::ShowConstraints => self.execute_show_constraints(),
            Statement::CreateIndex(ci) => self.execute_create_index(ci),
            Statement::DropIndex(di) => self.execute_drop_index(di),
            Statement::ShowIndexes => self.execute_show_indexes(),
            Statement::CreateFulltextIndex(cfi) => self.execute_create_fulltext_index(cfi),
            Statement::DropFulltextIndex(dfi) => self.execute_drop_fulltext_index(dfi),
            Statement::CreateUser(cu) => Ok(ResultSet::new(
                vec!["result".to_string()],
                vec![Row {
                    columns: vec![Value::String(format!(
                        "User '{}' created with role '{}'",
                        cu.username, cu.role
                    ))],
                }],
            )),
            Statement::DropUser(du) => Ok(ResultSet::new(
                vec!["result".to_string()],
                vec![Row {
                    columns: vec![Value::String(format!("User '{}' dropped", du.username))],
                }],
            )),
            Statement::AlterUser(au) => Ok(ResultSet::new(
                vec!["result".to_string()],
                vec![Row {
                    columns: vec![Value::String(format!("User '{}' altered", au.username))],
                }],
            )),
            Statement::ShowUsers => Ok(ResultSet::new(
                vec!["result".to_string()],
                vec![Row {
                    columns: vec![Value::String(
                        "SHOW USERS requires server context".to_string(),
                    )],
                }],
            )),
            Statement::Explain(inner) => self.execute_explain(*inner),
            Statement::Profile(inner) => self.execute_profile(*inner),
            Statement::ProcedureCall(pc) => self.execute_procedure_call(pc),
            Statement::Return(rc) => self.build_result_set(&rc, &[Bindings::new()]),
        }
    }

    // ========== UNION ==========

    fn execute_union(&mut self, union_stmt: UnionStatement) -> Result<ResultSet, ExecuteError> {
        let mut results: Vec<ResultSet> = Vec::new();

        for query in union_stmt.queries {
            let result = self.execute_match(query)?;
            results.push(result);
        }

        if results.is_empty() {
            return Ok(ResultSet::empty());
        }

        // Validate all result sets have the same number of columns
        let col_count = results[0].columns.len();
        for (i, rs) in results.iter().enumerate().skip(1) {
            if rs.columns.len() != col_count {
                return Err(ExecuteError::TypeError(format!(
                    "UNION: all queries must have the same number of columns (query 1 has {}, query {} has {})",
                    col_count,
                    i + 1,
                    rs.columns.len()
                )));
            }
        }

        // Use column names from the first query
        let columns = results[0].columns.clone();

        // Collect all rows
        let mut all_rows: Vec<Row> = Vec::new();
        for rs in results {
            all_rows.extend(rs.rows);
        }

        // Deduplicate for UNION (not UNION ALL)
        if union_stmt.union_type == UnionType::Union {
            let mut seen = HashSet::new();
            let mut unique_rows = Vec::new();
            for row in all_rows {
                let key: Vec<String> = row.columns.iter().map(|v| format!("{}", v)).collect();
                let key_str = key.join("\x00");
                if seen.insert(key_str) {
                    unique_rows.push(row);
                }
            }
            all_rows = unique_rows;
        }

        Ok(ResultSet::new(columns, all_rows))
    }

    // ========== CREATE ==========

    fn execute_create(&mut self, create: CreateClause) -> Result<ResultSet, ExecuteError> {
        let mut bindings = Bindings::new();
        let mut created_nodes = 0;
        let mut created_edges = 0;

        for pattern in create.patterns {
            match pattern {
                Pattern::Node(node_pattern) => {
                    self.create_node(&node_pattern, &mut bindings)?;
                    created_nodes += 1;
                }
                Pattern::Path(path_pattern) => {
                    // Create start node
                    let start_id = self.create_node(&path_pattern.start, &mut bindings)?;
                    created_nodes += 1;

                    let mut current_id = start_id;

                    for segment in path_pattern.segments {
                        // Create end node
                        let end_id = self.create_node(&segment.node, &mut bindings)?;
                        created_nodes += 1;

                        // Create edge
                        let (from, to) = match segment.edge.direction {
                            EdgeDirection::Outgoing => (current_id, end_id),
                            EdgeDirection::Incoming => (end_id, current_id),
                            EdgeDirection::Both => (current_id, end_id),
                        };

                        let edge_label = segment.edge.edge_type.unwrap_or_default();

                        // Validate endpoint label constraints before creating edge
                        self.constraints
                            .validate_edge_create(self.graph_ref(),&edge_label, from, to)?;

                        let edge_id = self.graph_mut().create_edge(from, to, edge_label)?;

                        // Evaluate and set edge properties
                        let edge_props: Vec<(String, PropertyValue)> = segment
                            .edge
                            .properties
                            .iter()
                            .map(|(k, expr)| {
                                let val = self.evaluate_expression(expr, &bindings)?;
                                let prop_val = self.value_to_property(&val)?;
                                Ok((k.clone(), prop_val))
                            })
                            .collect::<Result<_, ExecuteError>>()?;

                        for (key, prop_val) in edge_props {
                            self.graph_mut().set_edge_property(edge_id, &key, prop_val);
                        }

                        created_edges += 1;
                        current_id = end_id;
                    }
                }
            }
        }

        // Return summary
        let columns = vec!["created_nodes".to_string(), "created_edges".to_string()];
        let rows = vec![Row {
            columns: vec![Value::Int(created_nodes), Value::Int(created_edges)],
        }];

        Ok(ResultSet::new(columns, rows))
    }

    fn create_node(
        &mut self,
        pattern: &NodePattern,
        bindings: &mut Bindings,
    ) -> Result<NodeId, ExecuteError> {
        let labels = pattern.labels.clone();
        // Use the first label for constraint validation (primary label)
        let primary_label = labels.first().cloned().unwrap_or_default();

        // Evaluate property expressions
        let evaluated_props: Vec<(String, PropertyValue)> = pattern
            .properties
            .iter()
            .map(|(k, expr)| {
                let val = self.evaluate_expression(expr, bindings as &Bindings)?;
                let prop_val = self.value_to_property(&val)?;
                Ok((k.clone(), prop_val))
            })
            .collect::<Result<_, ExecuteError>>()?;

        let props: HashMap<String, PropertyValue> = evaluated_props.iter().cloned().collect();

        // Validate constraints before creating (using primary label)
        self.constraints
            .validate_node_create(self.graph_ref(),&primary_label, &props, None)?;

        let node_id = self.graph_mut().create_node_with_labels(labels.clone());

        // Set properties
        for (key, prop_val) in &evaluated_props {
            self.graph_mut().set_node_property(node_id, key, prop_val.clone());
        }

        // Index in fulltext indexes (using primary label for now)
        self.fulltext.index_node(node_id, &primary_label, &props);

        // Index properties for property indexes
        for (key, prop_val) in &evaluated_props {
            if self.property_index.has_index(&primary_label, key) {
                self.property_index.index_property(node_id, key, prop_val);
            }
        }

        // Bind variable
        if let Some(var) = &pattern.variable {
            bindings.insert(var.clone(), BindingValue::Node(node_id));
        }

        Ok(node_id)
    }

    // ========== DELETE ==========

    fn execute_delete(&mut self, d: DeleteStatement) -> Result<ResultSet, ExecuteError> {
        // Find all matching bindings
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for pattern in &d.patterns {
            all_bindings = self.match_pattern(pattern, all_bindings)?;
        }

        // Apply WHERE filter
        if let Some(where_expr) = &d.where_clause {
            all_bindings.retain(|bindings| {
                self.evaluate_expression(where_expr, bindings)
                    .map(|v| matches!(v, Value::Bool(true)))
                    .unwrap_or(false)
            });
        }

        // Apply SET clause
        if let Some(set_clause) = &d.set_clause.clone() {
            self.apply_set_clause(set_clause, &all_bindings)?;
        }

        // Collect all IDs to delete (to avoid modifying while iterating)
        let mut nodes_to_delete = Vec::new();
        let mut edges_to_delete = Vec::new();

        for bindings in &all_bindings {
            for var in &d.delete_clause.variables {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(id) => {
                            if !nodes_to_delete.contains(id) {
                                nodes_to_delete.push(*id);
                            }
                        }
                        BindingValue::Edge(id) => {
                            if !edges_to_delete.contains(id) {
                                edges_to_delete.push(*id);
                            }
                        }
                        BindingValue::Path { .. } | BindingValue::Scalar(_) => {
                            // Paths and scalars cannot be deleted directly
                        }
                    }
                }
            }
        }

        // Delete edges first
        let mut deleted_edges = 0;
        for edge_id in edges_to_delete {
            if self.graph_mut().delete_edge(edge_id).is_some() {
                deleted_edges += 1;
            }
        }

        // Delete nodes (with DETACH if specified)
        let mut deleted_nodes = 0;
        for node_id in nodes_to_delete {
            if d.delete_clause.detach {
                // delete_node already handles related edges
                if self.graph_mut().delete_node(node_id).is_some() {
                    deleted_nodes += 1;
                }
            } else {
                // Check if node has edges
                let has_edges = self.graph_ref().has_incident_edges(node_id);

                if has_edges {
                    // In a real Cypher implementation, this would be an error
                    // For simplicity, we just skip or we could return an error
                    continue;
                }

                if self.graph_mut().delete_node(node_id).is_some() {
                    deleted_nodes += 1;
                }
            }
        }

        // Return summary
        let columns = vec!["deleted_nodes".to_string(), "deleted_edges".to_string()];
        let rows = vec![Row {
            columns: vec![Value::Int(deleted_nodes), Value::Int(deleted_edges)],
        }];

        Ok(ResultSet::new(columns, rows))
    }

    // ========== MATCH + CREATE ==========

    fn execute_match_create(
        &mut self,
        mc: MatchCreateStatement,
    ) -> Result<ResultSet, ExecuteError> {
        // Execute MATCH segments to get bindings
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &mc.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;
        }

        // Apply WHERE filter (from last segment)
        if let Some(where_expr) = &mc.where_clause {
            all_bindings.retain(|b| {
                self.evaluate_expression(where_expr, b)
                    .map(|v| matches!(v, Value::Bool(true)))
                    .unwrap_or(false)
            });
        }

        // Execute CREATE for each binding set
        let mut created_nodes = 0i64;
        let mut created_edges = 0i64;

        for bindings in &all_bindings {
            let (cn, ce) = self.execute_create_with_bindings(&mc.create_clause, bindings)?;
            created_nodes += cn;
            created_edges += ce;
        }

        let columns = vec!["created_nodes".to_string(), "created_edges".to_string()];
        let rows = vec![Row {
            columns: vec![Value::Int(created_nodes), Value::Int(created_edges)],
        }];
        Ok(ResultSet::new(columns, rows))
    }

    fn execute_create_with_bindings(
        &mut self,
        create: &CreateClause,
        existing_bindings: &Bindings,
    ) -> Result<(i64, i64), ExecuteError> {
        let mut bindings = existing_bindings.clone();
        let mut created_nodes = 0i64;
        let mut created_edges = 0i64;

        for pattern in &create.patterns {
            match pattern {
                Pattern::Node(node_pattern) => {
                    // If variable is already bound, skip creation
                    if let Some(var) = &node_pattern.variable
                        && bindings.contains_key(var)
                    {
                        continue;
                    }
                    self.create_node(node_pattern, &mut bindings)?;
                    created_nodes += 1;
                }
                Pattern::Path(path_pattern) => {
                    // Check if start node is already bound
                    let start_id = if let Some(var) = &path_pattern.start.variable {
                        if let Some(bound) = bindings.get(var) {
                            bound.as_node().ok_or_else(|| {
                                ExecuteError::TypeError("expected node binding".to_string())
                            })?
                        } else {
                            let id = self.create_node(&path_pattern.start, &mut bindings)?;
                            created_nodes += 1;
                            id
                        }
                    } else {
                        let id = self.create_node(&path_pattern.start, &mut bindings)?;
                        created_nodes += 1;
                        id
                    };

                    let mut current_id = start_id;

                    for segment in &path_pattern.segments {
                        // Check if end node is already bound
                        let end_id = if let Some(var) = &segment.node.variable {
                            if let Some(bound) = bindings.get(var) {
                                bound.as_node().ok_or_else(|| {
                                    ExecuteError::TypeError("expected node binding".to_string())
                                })?
                            } else {
                                let id = self.create_node(&segment.node, &mut bindings)?;
                                created_nodes += 1;
                                id
                            }
                        } else {
                            let id = self.create_node(&segment.node, &mut bindings)?;
                            created_nodes += 1;
                            id
                        };

                        // Create edge
                        let (from, to) = match segment.edge.direction {
                            EdgeDirection::Outgoing => (current_id, end_id),
                            EdgeDirection::Incoming => (end_id, current_id),
                            EdgeDirection::Both => (current_id, end_id),
                        };

                        let edge_label = segment.edge.edge_type.clone().unwrap_or_default();

                        // Validate endpoint label constraints before creating edge
                        self.constraints
                            .validate_edge_create(self.graph_ref(),&edge_label, from, to)?;

                        let edge_id = self.graph_mut().create_edge(from, to, edge_label)?;

                        // Evaluate and set edge properties
                        let edge_props: Vec<(String, PropertyValue)> = segment
                            .edge
                            .properties
                            .iter()
                            .map(|(k, expr)| {
                                let val = self.evaluate_expression(expr, &bindings)?;
                                let prop_val = self.value_to_property(&val)?;
                                Ok((k.clone(), prop_val))
                            })
                            .collect::<Result<_, ExecuteError>>()?;

                        for (key, prop_val) in edge_props {
                            self.graph_mut().set_edge_property(edge_id, &key, prop_val);
                        }

                        created_edges += 1;
                        current_id = end_id;
                    }
                }
            }
        }

        Ok((created_nodes, created_edges))
    }

    // ========== MATCH + SET ==========

    fn execute_match_set(&mut self, ms: MatchSetStatement) -> Result<ResultSet, ExecuteError> {
        // Execute MATCH segments
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &ms.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;
        }

        // Apply WHERE filter
        if let Some(where_expr) = &ms.where_clause {
            all_bindings.retain(|b| {
                self.evaluate_expression(where_expr, b)
                    .map(|v| matches!(v, Value::Bool(true)))
                    .unwrap_or(false)
            });
        }

        // Apply SET clause
        self.apply_set_clause(&ms.set_clause, &all_bindings)?;

        // Build result set
        if let Some(return_clause) = &ms.return_clause {
            self.build_result_set(return_clause, &all_bindings)
        } else {
            let columns = vec!["properties_set".to_string()];
            let rows = vec![Row {
                columns: vec![Value::Int(
                    ms.set_clause.items.len() as i64 * all_bindings.len() as i64,
                )],
            }];
            Ok(ResultSet::new(columns, rows))
        }
    }

    fn apply_set_clause(
        &mut self,
        set_clause: &SetClause,
        all_bindings: &[Bindings],
    ) -> Result<(), ExecuteError> {
        for bindings in all_bindings {
            for item in &set_clause.items {
                match item {
                    SetItem::Property(variable, property, value_expr) => {
                        let binding_value = bindings
                            .get(variable)
                            .ok_or_else(|| ExecuteError::UndefinedVariable(variable.clone()))?;

                        let value = self.evaluate_expression(value_expr, bindings)?;
                        let prop_value = self.value_to_property(&value)?;

                        match binding_value {
                            BindingValue::Node(node_id) => {
                                // Validate constraint before setting
                                if let Some(node) = self.graph_ref().get_node(*node_id) {
                                    self.constraints.validate_property_set(
                                        self.graph_ref(),
                                        &node,
                                        property,
                                        &prop_value,
                                    )?;
                                }
                                self.graph_mut().set_node_property(*node_id, property, prop_value);
                            }
                            BindingValue::Edge(edge_id) => {
                                self.graph_mut().set_edge_property(*edge_id, property, prop_value);
                            }
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "SET requires node or edge binding".to_string(),
                                ));
                            }
                        }
                    }
                    SetItem::MergeProperties(variable, props_map) => {
                        let binding_value = bindings
                            .get(variable)
                            .ok_or_else(|| ExecuteError::UndefinedVariable(variable.clone()))?;

                        // Evaluate all property expressions first
                        let evaluated: Vec<(String, PropertyValue)> = props_map
                            .iter()
                            .map(|(k, expr)| {
                                let val = self.evaluate_expression(expr, bindings)?;
                                let prop_val = self.value_to_property(&val)?;
                                Ok((k.clone(), prop_val))
                            })
                            .collect::<Result<_, ExecuteError>>()?;

                        match binding_value {
                            BindingValue::Node(node_id) => {
                                // Validate constraints for each new property
                                for (key, prop_val) in &evaluated {
                                    if let Some(node) = self.graph_ref().get_node(*node_id) {
                                        self.constraints.validate_property_set(
                                            self.graph_ref(), &node, key, prop_val,
                                        )?;
                                    }
                                }
                                for (key, prop_val) in evaluated {
                                    self.graph_mut().set_node_property(*node_id, &key, prop_val);
                                }
                            }
                            BindingValue::Edge(edge_id) => {
                                for (key, prop_val) in evaluated {
                                    self.graph_mut().set_edge_property(*edge_id, &key, prop_val);
                                }
                            }
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "SET += requires node or edge binding".to_string(),
                                ));
                            }
                        }
                    }
                    SetItem::AddLabel(variable, new_label) => {
                        let binding_value = bindings
                            .get(variable)
                            .ok_or_else(|| ExecuteError::UndefinedVariable(variable.clone()))?;

                        match binding_value {
                            BindingValue::Node(node_id) => {
                                self.graph_mut().add_node_label(*node_id, new_label.clone());
                            }
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "SET n:Label requires a node binding".to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // ========== MERGE ==========

    fn execute_merge(&mut self, merge: MergeStatement) -> Result<ResultSet, ExecuteError> {
        // If there are MATCH clauses, execute them first
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        if !merge.match_clauses.is_empty() {
            for clause in &merge.match_clauses {
                all_bindings = self.execute_match_clause(clause, all_bindings)?;
            }

            if let Some(where_expr) = &merge.where_clause {
                all_bindings.retain(|b| {
                    self.evaluate_expression(where_expr, b)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                });
            }
        }

        let mut result_bindings = Vec::new();

        for bindings in &all_bindings {
            // Try to MATCH the merge patterns
            let mut match_result = vec![bindings.clone()];
            for pattern in &merge.patterns {
                match_result = self.match_pattern(pattern, match_result)?;
            }

            if match_result.is_empty() {
                // Pattern not found -> CREATE
                let mut new_bindings = bindings.clone();
                for pattern in &merge.patterns {
                    self.create_pattern(pattern, &mut new_bindings)?;
                }
                // Apply ON CREATE SET
                if let Some(on_create_set) = &merge.on_create_set {
                    self.apply_set_clause(on_create_set, &[new_bindings.clone()])?;
                }
                result_bindings.push(new_bindings);
            } else {
                // Pattern found -> use existing
                // Apply ON MATCH SET
                if let Some(on_match_set) = &merge.on_match_set {
                    self.apply_set_clause(on_match_set, &match_result)?;
                }
                result_bindings.extend(match_result);
            }
        }

        if let Some(return_clause) = &merge.return_clause {
            self.build_result_set(return_clause, &result_bindings)
        } else {
            Ok(ResultSet::new(
                vec!["merge_result".to_string()],
                vec![Row {
                    columns: vec![Value::String("ok".to_string())],
                }],
            ))
        }
    }

    fn create_pattern(
        &mut self,
        pattern: &Pattern,
        bindings: &mut Bindings,
    ) -> Result<(), ExecuteError> {
        match pattern {
            Pattern::Node(node_pattern) => {
                if let Some(var) = &node_pattern.variable
                    && bindings.contains_key(var)
                {
                    return Ok(());
                }
                self.create_node(node_pattern, bindings)?;
                Ok(())
            }
            Pattern::Path(path_pattern) => {
                let start_id = if let Some(var) = &path_pattern.start.variable {
                    if let Some(bound) = bindings.get(var) {
                        bound
                            .as_node()
                            .ok_or_else(|| ExecuteError::TypeError("expected node".to_string()))?
                    } else {
                        self.create_node(&path_pattern.start, bindings)?
                    }
                } else {
                    self.create_node(&path_pattern.start, bindings)?
                };

                let mut current_id = start_id;

                for segment in &path_pattern.segments {
                    let end_id = if let Some(var) = &segment.node.variable {
                        if let Some(bound) = bindings.get(var) {
                            bound.as_node().ok_or_else(|| {
                                ExecuteError::TypeError("expected node".to_string())
                            })?
                        } else {
                            self.create_node(&segment.node, bindings)?
                        }
                    } else {
                        self.create_node(&segment.node, bindings)?
                    };

                    let (from, to) = match segment.edge.direction {
                        EdgeDirection::Outgoing => (current_id, end_id),
                        EdgeDirection::Incoming => (end_id, current_id),
                        EdgeDirection::Both => (current_id, end_id),
                    };

                    let edge_label = segment.edge.edge_type.clone().unwrap_or_default();

                    // Validate endpoint label constraints before creating edge
                    self.constraints
                        .validate_edge_create(self.graph_ref(),&edge_label, from, to)?;

                    let edge_id = self.graph_mut().create_edge(from, to, edge_label)?;

                    // Evaluate and set edge properties
                    let edge_props: Vec<(String, PropertyValue)> = segment
                        .edge
                        .properties
                        .iter()
                        .map(|(k, expr)| {
                            let val = self.evaluate_expression(expr, bindings as &Bindings)?;
                            let prop_val = self.value_to_property(&val)?;
                            Ok((k.clone(), prop_val))
                        })
                        .collect::<Result<_, ExecuteError>>()?;

                    for (key, prop_val) in edge_props {
                        self.graph_mut().set_edge_property(edge_id, &key, prop_val);
                    }

                    current_id = end_id;
                }
                Ok(())
            }
        }
    }

    // ========== MATCH + REMOVE ==========

    fn execute_match_remove(
        &mut self,
        mr: MatchRemoveStatement,
    ) -> Result<ResultSet, ExecuteError> {
        // Execute MATCH segments
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &mr.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;
        }

        // Apply WHERE filter
        if let Some(where_expr) = &mr.where_clause {
            all_bindings.retain(|b| {
                self.evaluate_expression(where_expr, b)
                    .map(|v| matches!(v, Value::Bool(true)))
                    .unwrap_or(false)
            });
        }

        // Apply REMOVE clause
        for bindings in &all_bindings {
            for item in &mr.remove_clause.items {
                match item {
                    RemoveItem::Property(var, prop) => {
                        let binding_value = bindings
                            .get(var)
                            .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;

                        match binding_value {
                            BindingValue::Node(node_id) => {
                                // Validate constraint before removing
                                if let Some(node) = self.graph_ref().get_node(*node_id) {
                                    self.constraints.validate_property_remove(&node, prop)?;
                                }
                                self.graph_mut().remove_node_property(*node_id, prop);
                            }
                            BindingValue::Edge(edge_id) => {
                                self.graph_mut().remove_edge_property(*edge_id, prop);
                            }
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "REMOVE requires node or edge binding".to_string(),
                                ));
                            }
                        }
                    }
                    RemoveItem::Label(var, label) => {
                        let _binding_value = bindings
                            .get(var)
                            .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;
                        if let Some(node_id) = bindings.get(var).and_then(|v| v.as_node()) {
                            self.graph_mut().remove_node_label(node_id, label);
                        }
                    }
                }
            }
        }

        // Build result set
        if let Some(return_clause) = &mr.return_clause {
            self.build_result_set(return_clause, &all_bindings)
        } else {
            Ok(ResultSet::new(
                vec!["properties_removed".to_string()],
                vec![Row {
                    columns: vec![Value::Int(
                        mr.remove_clause.items.len() as i64 * all_bindings.len() as i64,
                    )],
                }],
            ))
        }
    }

    // ========== UNWIND ==========

    fn execute_unwind(&mut self, uw: UnwindStatement) -> Result<ResultSet, ExecuteError> {
        let empty_bindings = Bindings::new();
        let list_value = self.evaluate_expression(&uw.expression, &empty_bindings)?;

        let items = match list_value {
            Value::List(items) => items,
            _ => {
                return Err(ExecuteError::TypeError(
                    "UNWIND requires a list expression".to_string(),
                ));
            }
        };

        // Expand list into bindings
        let mut all_bindings: Vec<Bindings> = Vec::new();
        for item in &items {
            let mut bindings = Bindings::new();
            bindings.insert(uw.variable.clone(), BindingValue::Scalar(item.clone()));
            all_bindings.push(bindings);
        }

        // Execute CREATE if present
        if let Some(create_clause) = &uw.create_clause {
            let mut created_nodes = 0i64;
            let mut created_edges = 0i64;

            for bindings in &all_bindings {
                let (cn, ce) = self.execute_create_with_bindings(create_clause, bindings)?;
                created_nodes += cn;
                created_edges += ce;
            }

            // Apply SET if present (after CREATE)
            if let Some(set_clause) = &uw.set_clause {
                self.apply_set_clause(set_clause, &all_bindings)?;
            }

            if let Some(return_clause) = &uw.return_clause {
                return self.build_result_set(return_clause, &all_bindings);
            }

            return Ok(ResultSet::new(
                vec!["created_nodes".to_string(), "created_edges".to_string()],
                vec![Row {
                    columns: vec![Value::Int(created_nodes), Value::Int(created_edges)],
                }],
            ));
        }

        // Build result set
        if let Some(return_clause) = &uw.return_clause {
            self.build_result_set(return_clause, &all_bindings)
        } else {
            Ok(ResultSet::new(
                vec!["unwound_rows".to_string()],
                vec![Row {
                    columns: vec![Value::Int(all_bindings.len() as i64)],
                }],
            ))
        }
    }

    // ========== FOREACH ==========

    fn execute_foreach_stmt_ref(
        &mut self,
        stmt: &ForeachStatement,
        outer_bindings: &Bindings,
    ) -> Result<ResultSet, ExecuteError> {
        let list_val = self.evaluate_expression(&stmt.list, outer_bindings)?;

        let items = match list_val {
            Value::List(items) => items,
            _ => {
                return Err(ExecuteError::TypeError(
                    "FOREACH requires a list expression".to_string(),
                ));
            }
        };

        for item in &items {
            let mut bindings = outer_bindings.clone();
            bindings.insert(stmt.variable.clone(), BindingValue::Scalar(item.clone()));

            for clause in &stmt.clauses {
                self.execute_foreach_clause(clause, &bindings)?;
            }
        }

        Ok(ResultSet::new(
            vec!["foreach_result".to_string()],
            vec![Row {
                columns: vec![Value::String("ok".to_string())],
            }],
        ))
    }

    fn execute_foreach_clause(
        &mut self,
        clause: &ForeachClause,
        bindings: &Bindings,
    ) -> Result<(), ExecuteError> {
        match clause {
            ForeachClause::Create(create) => {
                self.execute_create_with_bindings(create, bindings)?;
                Ok(())
            }
            ForeachClause::Set(set) => {
                self.apply_set_clause(set, std::slice::from_ref(bindings))?;
                Ok(())
            }
            ForeachClause::Remove(remove) => {
                self.apply_remove_clause(remove, bindings)?;
                Ok(())
            }
            ForeachClause::Delete(delete) => {
                self.apply_delete_clause(delete, bindings)?;
                Ok(())
            }
            ForeachClause::Merge(patterns) => {
                let mut b = bindings.clone();
                let mut match_result = vec![b.clone()];
                for pattern in patterns {
                    match_result = self.match_pattern(pattern, match_result)?;
                }
                if match_result.is_empty() {
                    for pattern in patterns {
                        self.create_pattern(pattern, &mut b)?;
                    }
                }
                Ok(())
            }
            ForeachClause::Foreach(inner) => {
                self.execute_foreach_stmt_ref(inner, bindings)?;
                Ok(())
            }
        }
    }

    fn apply_remove_clause(
        &mut self,
        remove: &RemoveClause,
        bindings: &Bindings,
    ) -> Result<(), ExecuteError> {
        for item in &remove.items {
            match item {
                RemoveItem::Property(var, prop) => {
                    let binding_value = bindings
                        .get(var)
                        .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;

                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                self.constraints.validate_property_remove(&node, prop)?;
                            }
                            self.graph_mut().remove_node_property(*node_id, prop);
                        }
                        BindingValue::Edge(edge_id) => {
                            self.graph_mut().remove_edge_property(*edge_id, prop);
                        }
                        _ => {
                            return Err(ExecuteError::TypeError(
                                "REMOVE requires node or edge binding".to_string(),
                            ));
                        }
                    }
                }
                RemoveItem::Label(var, label) => {
                    if let Some(node_id) = bindings.get(var).and_then(|v| v.as_node()) {
                        self.graph_mut().remove_node_label(node_id, label);
                    } else {
                        return Err(ExecuteError::UndefinedVariable(var.clone()));
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_delete_clause(
        &mut self,
        delete: &DeleteClause,
        bindings: &Bindings,
    ) -> Result<(), ExecuteError> {
        let mut nodes_to_delete = Vec::new();
        let mut edges_to_delete = Vec::new();

        for var in &delete.variables {
            if let Some(binding_value) = bindings.get(var) {
                match binding_value {
                    BindingValue::Node(id) => {
                        if !nodes_to_delete.contains(id) {
                            nodes_to_delete.push(*id);
                        }
                    }
                    BindingValue::Edge(id) => {
                        if !edges_to_delete.contains(id) {
                            edges_to_delete.push(*id);
                        }
                    }
                    BindingValue::Path { .. } | BindingValue::Scalar(_) => {}
                }
            }
        }

        for edge_id in edges_to_delete {
            self.graph_mut().delete_edge(edge_id);
        }

        for node_id in nodes_to_delete {
            if delete.detach {
                self.graph_mut().delete_node(node_id);
            } else {
                let has_edges = self.graph_ref().has_incident_edges(node_id);
                if !has_edges {
                    self.graph_mut().delete_node(node_id);
                }
            }
        }

        Ok(())
    }

    fn execute_match_foreach(
        &mut self,
        mf: MatchForeachStatement,
    ) -> Result<ResultSet, ExecuteError> {
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &mf.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;
        }

        for bindings in &all_bindings {
            self.execute_foreach_stmt_ref(&mf.foreach_clause, bindings)?;
        }

        Ok(ResultSet::new(
            vec!["foreach_result".to_string()],
            vec![Row {
                columns: vec![Value::String("ok".to_string())],
            }],
        ))
    }

    // ========== CONSTRAINT DDL ==========

    fn execute_create_constraint(
        &mut self,
        cc: CreateConstraintStatement,
    ) -> Result<ResultSet, ExecuteError> {
        let constraint_type = match cc.constraint_type {
            ConstraintTypeAst::Unique => ConstraintType::Unique,
            ConstraintTypeAst::NotNull => ConstraintType::NotNull,
            ConstraintTypeAst::TypeCheck(t) => {
                let prop_type = match t {
                    PropertyTypeAst::Integer => PropertyType::Int,
                    PropertyTypeAst::Float => PropertyType::Float,
                    PropertyTypeAst::String => PropertyType::String,
                    PropertyTypeAst::Boolean => PropertyType::Bool,
                };
                ConstraintType::TypeCheck(prop_type)
            }
            ConstraintTypeAst::RequiredLabel(required_label) => {
                ConstraintType::RequiredLabel(required_label)
            }
            ConstraintTypeAst::EndpointLabel {
                source_label,
                target_label,
            } => ConstraintType::EndpointLabel {
                source_label,
                target_label,
            },
        };

        let constraint = Constraint {
            name: cc.name.clone(),
            label: cc.label,
            properties: cc.properties,
            constraint_type,
        };

        self.constraints.create_constraint(constraint)?;

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!("Constraint '{}' created", cc.name))],
            }],
        ))
    }

    fn execute_drop_constraint(
        &mut self,
        dc: DropConstraintStatement,
    ) -> Result<ResultSet, ExecuteError> {
        self.constraints.drop_constraint(&dc.name)?;

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!("Constraint '{}' dropped", dc.name))],
            }],
        ))
    }

    fn execute_show_constraints(&self) -> Result<ResultSet, ExecuteError> {
        let constraints = self.constraints.list_constraints();

        let mut rows = Vec::new();
        for c in constraints {
            rows.push(Row {
                columns: vec![
                    Value::String(c.name.clone()),
                    Value::String(c.label.clone()),
                    Value::String(c.properties.join(", ")),
                    Value::String(c.constraint_type.to_string()),
                ],
            });
        }

        Ok(ResultSet::new(
            vec![
                "name".to_string(),
                "label".to_string(),
                "properties".to_string(),
                "type".to_string(),
            ],
            rows,
        ))
    }

    // ========== FULLTEXT INDEX ==========

    fn execute_create_fulltext_index(
        &mut self,
        cfi: CreateFulltextIndexStatement,
    ) -> Result<ResultSet, ExecuteError> {
        self.fulltext
            .create_index(&cfi.name, &cfi.label, cfi.properties)?;

        // Index existing nodes that match the label
        let node_ids: Vec<NodeId> = self
            .graph_ref()
            .all_nodes()
            .into_iter()
            .filter(|n| n.has_label(&cfi.label))
            .map(|n| n.id)
            .collect();

        for node_id in node_ids {
            if let Some(node) = self.graph_ref().get_node(node_id) {
                let props = Arc::clone(&node.properties);
                self.fulltext.index_node(node_id, &cfi.label, &props);
            }
        }

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!(
                    "Fulltext index '{}' created",
                    cfi.name
                ))],
            }],
        ))
    }

    fn execute_drop_fulltext_index(
        &mut self,
        dfi: DropFulltextIndexStatement,
    ) -> Result<ResultSet, ExecuteError> {
        self.fulltext.drop_index(&dfi.name)?;

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!(
                    "Fulltext index '{}' dropped",
                    dfi.name
                ))],
            }],
        ))
    }

    // ========== PROPERTY INDEX ==========

    fn execute_create_index(&mut self, ci: CreateIndexStatement) -> Result<ResultSet, ExecuteError> {
        let def = IndexDefinition::new(ci.label.clone(), ci.property.clone());
        self.property_index.create_index(def);

        // Index existing nodes that match the label
        let node_ids: Vec<NodeId> = self
            .graph_ref()
            .all_nodes()
            .into_iter()
            .filter(|n| n.has_label(&ci.label))
            .map(|n| n.id)
            .collect();

        for node_id in node_ids {
            if let Some(node) = self.graph_ref().get_node(node_id)
                && let Some(val) = node.get_property(&ci.property)
            {
                self.property_index.index_property(node_id, &ci.property, val);
            }
        }

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!(
                    "Index created on :{}({})",
                    ci.label, ci.property
                ))],
            }],
        ))
    }

    fn execute_drop_index(&mut self, di: DropIndexStatement) -> Result<ResultSet, ExecuteError> {
        self.property_index.drop_index(&di.label, &di.property);

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!(
                    "Index dropped on :{}({})",
                    di.label, di.property
                ))],
            }],
        ))
    }

    fn execute_show_indexes(&self) -> Result<ResultSet, ExecuteError> {
        let indexes = self.property_index.list_indexes();

        let mut rows = Vec::new();
        for def in indexes {
            rows.push(Row {
                columns: vec![
                    Value::String(def.label.clone()),
                    Value::String(def.property.clone()),
                ],
            });
        }

        Ok(ResultSet::new(
            vec!["label".to_string(), "property".to_string()],
            rows,
        ))
    }

    // ========== PROCEDURE CALLS ==========

    /// Execute a top-level procedure call: CALL proc.name(args) YIELD col1, col2 [RETURN ...]
    ///
    /// Currently supports:
    ///   - `db.index.fulltext.search(indexName, query)` — returns (node, score) rows
    fn execute_procedure_call(
        &mut self,
        pc: ProcedureCallStatement,
    ) -> Result<ResultSet, ExecuteError> {
        match pc.procedure.as_str() {
            "db.index.fulltext.search" => {
                self.execute_fulltext_search_procedure(pc)
            }
            other => Err(ExecuteError::TypeError(format!(
                "unknown procedure: {}",
                other
            ))),
        }
    }

    /// Execute `CALL db.index.fulltext.search(indexName, query) YIELD node, score`.
    ///
    /// Returns a result set with columns derived from the YIELD clause.
    /// Standard column names: `node` (NodeId as Node value) and `score` (Float).
    fn execute_fulltext_search_procedure(
        &mut self,
        pc: ProcedureCallStatement,
    ) -> Result<ResultSet, ExecuteError> {
        if pc.arguments.len() != 2 {
            return Err(ExecuteError::TypeError(
                "db.index.fulltext.search requires exactly 2 arguments: (indexName, query)"
                    .to_string(),
            ));
        }

        // Evaluate arguments — they must resolve to strings
        let empty_bindings = Bindings::new();
        let index_name_val = self.evaluate_expression(&pc.arguments[0], &empty_bindings)?;
        let query_val = self.evaluate_expression(&pc.arguments[1], &empty_bindings)?;

        let index_name = match &index_name_val {
            Value::String(s) => s.clone(),
            _ => {
                return Err(ExecuteError::TypeError(
                    "db.index.fulltext.search: first argument must be a string (index name)"
                        .to_string(),
                ))
            }
        };

        let query = match &query_val {
            Value::String(s) => s.clone(),
            _ => {
                return Err(ExecuteError::TypeError(
                    "db.index.fulltext.search: second argument must be a string (query)"
                        .to_string(),
                ))
            }
        };

        // Perform the search
        let search_results = self.fulltext.search(&index_name, &query)?;

        // Determine column names from YIELD clause (default: node, score)
        let yield_cols = if pc.yield_columns.is_empty() {
            vec!["node".to_string(), "score".to_string()]
        } else {
            pc.yield_columns.clone()
        };

        // Build bindings for each result row
        let mut all_bindings: Vec<Bindings> = Vec::new();
        for result in &search_results {
            let mut bindings = Bindings::new();
            // Always bind "node" and "score" so RETURN items can reference them
            bindings.insert(
                "node".to_string(),
                BindingValue::Node(result.node_id),
            );
            bindings.insert(
                "score".to_string(),
                BindingValue::Scalar(Value::Float(result.score)),
            );
            all_bindings.push(bindings);
        }

        // If RETURN clause present, use it for projection/ordering
        if let Some(return_clause) = pc.return_clause {
            return self.build_result_set(&return_clause, &all_bindings);
        }

        // Otherwise, project YIELD columns directly
        let mut rows = Vec::new();
        for bindings in &all_bindings {
            let mut row_cols = Vec::new();
            for col_name in &yield_cols {
                let val = match bindings.get(col_name) {
                    Some(BindingValue::Node(id)) => Value::Node(*id),
                    Some(BindingValue::Scalar(v)) => v.clone(),
                    Some(BindingValue::Edge(id)) => Value::Int(*id as i64),
                    Some(BindingValue::Path { nodes, edges }) => Value::Path {
                        nodes: nodes.clone(),
                        edges: edges.clone(),
                    },
                    None => Value::Null,
                };
                row_cols.push(val);
            }
            rows.push(Row { columns: row_cols });
        }

        Ok(ResultSet::new(yield_cols, rows))
    }

    // ========== EXPLAIN / PROFILE ==========

    fn execute_explain(&self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
        let node_count = self.graph_ref().node_count() as u64;
        let edge_count = self.graph_ref().edge_count() as u64;
        let plan = crate::planner::build_plan(&stmt, node_count, edge_count);
        let plan_text = format!("{}", plan);

        let mut rows = Vec::new();
        for line in plan_text.lines() {
            rows.push(Row {
                columns: vec![Value::String(line.to_string())],
            });
        }

        Ok(ResultSet::new(vec!["plan".to_string()], rows))
    }

    fn execute_profile(&mut self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
        let node_count = self.graph_ref().node_count() as u64;
        let edge_count = self.graph_ref().edge_count() as u64;
        let mut plan = crate::planner::build_plan(&stmt, node_count, edge_count);

        // Execute the statement and measure time
        let start = std::time::Instant::now();
        let result = self.execute(stmt)?;
        let elapsed = start.elapsed();

        // Annotate plan nodes with actual stats
        let actual_rows = result.rows.len() as u64;
        let time_us = elapsed.as_micros() as u64;
        if let Some(last) = plan.nodes.last_mut() {
            last.actual_rows = Some(actual_rows);
            last.actual_time_us = Some(time_us);
        }

        // Return plan + result summary
        let plan_text = format!("{}", plan);
        let mut rows = Vec::new();
        for line in plan_text.lines() {
            rows.push(Row {
                columns: vec![Value::String(line.to_string())],
            });
        }
        rows.push(Row {
            columns: vec![Value::String(format!(
                "Rows: {}, Time: {} us",
                actual_rows, time_us
            ))],
        });

        Ok(ResultSet::new(vec!["profile".to_string()], rows))
    }

    fn value_to_property(&self, value: &Value) -> Result<PropertyValue, ExecuteError> {
        match value {
            Value::Null => Ok(PropertyValue::Null),
            Value::Bool(b) => Ok(PropertyValue::Bool(*b)),
            Value::Int(n) => Ok(PropertyValue::Int(*n)),
            Value::Float(n) => Ok(PropertyValue::Float(*n)),
            Value::String(s) => Ok(PropertyValue::String(s.clone())),
            Value::Date(d) => Ok(PropertyValue::Date(*d)),
            Value::DateTime(ms) => Ok(PropertyValue::DateTime(*ms)),
            Value::Duration { months, days, millis } => Ok(PropertyValue::Duration {
                months: *months,
                days: *days,
                millis: *millis,
            }),
            _ => Err(ExecuteError::TypeError(
                "cannot convert to property value".to_string(),
            )),
        }
    }

    // ========== MATCH ==========

    fn execute_match(&mut self, m: MatchStatement) -> Result<ResultSet, ExecuteError> {
        // Detect an early-termination limit: applicable only when there is no ORDER BY
        // and no aggregation, since both require collecting all rows before outputting.
        let has_aggregation = m
            .return_clause
            .items
            .iter()
            .any(|item| Self::is_aggregate(item));

        let early_limit: Option<usize> =
            if !has_aggregation && m.return_clause.order_by.is_none() {
                m.return_clause
                    .limit
                    .as_ref()
                    .and_then(|e| self.resolve_skip_limit(e).ok())
                    .map(|n| {
                        // Include SKIP in the early cutoff so ORDER-agnostic queries
                        // still yield the correct slice.
                        let skip = m
                            .return_clause
                            .skip
                            .as_ref()
                            .and_then(|e| self.resolve_skip_limit(e).ok())
                            .unwrap_or(0) as usize;
                        n as usize + skip
                    })
            } else {
                None
            };

        // Process each segment, applying the early cutoff after each one.
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &m.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;

            if let Some(limit) = early_limit
                && all_bindings.len() >= limit
            {
                all_bindings.truncate(limit);
                break;
            }
        }

        // Execute CALL subquery if present
        if let Some(ref call) = m.call_clause {
            all_bindings = self.execute_call_subquery(call, all_bindings)?;
        }

        // Build result set
        self.build_result_set(&m.return_clause, &all_bindings)
    }

    fn execute_call_subquery(
        &self,
        call: &CallSubquery,
        outer_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        for outer in outer_bindings {
            // Build inner starting bindings from WITH imports
            let inner_start = if let Some(ref imports) = call.with_import {
                let mut inner = Bindings::new();
                for var in imports {
                    if let Some(val) = outer.get(var) {
                        inner.insert(var.clone(), val.clone());
                    }
                }
                inner
            } else {
                // Without WITH, inner subquery has no access to outer bindings
                Bindings::new()
            };

            // Execute inner MATCH
            let mut inner_bindings = vec![inner_start];
            inner_bindings = self.execute_match_clause(&call.match_clause, inner_bindings)?;

            // Apply inner WHERE
            if let Some(ref where_expr) = call.where_clause {
                inner_bindings.retain(|b| {
                    self.evaluate_expression(where_expr, b)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                });
            }

            // Evaluate each return item for all inner bindings
            // We build a temporary ReturnClause to use build_result_set for aggregation support
            let temp_return_clause = ReturnClause {
                distinct: false,
                items: call
                    .return_items
                    .iter()
                    .map(|ri| ri.expression.clone())
                    .collect(),
                order_by: None,
                skip: None,
                limit: None,
            };
            let inner_result = self.build_result_set(&temp_return_clause, &inner_bindings)?;

            // Compute effective column names (alias takes priority over generated name)
            let col_names: Vec<String> = call
                .return_items
                .iter()
                .enumerate()
                .map(|(i, ri)| {
                    ri.alias.clone().unwrap_or_else(|| {
                        inner_result
                            .columns
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| format!("col{}", i))
                    })
                })
                .collect();

            if inner_result.rows.is_empty() {
                // No inner results - skip this outer row (inner join semantics)
                continue;
            }

            // Merge: for each inner row, combine outer binding + inner row columns as scalars
            for inner_row in &inner_result.rows {
                let mut merged = outer.clone();
                for (col_name, col_val) in col_names.iter().zip(inner_row.columns.iter()) {
                    merged.insert(col_name.clone(), BindingValue::Scalar(col_val.clone()));
                }
                result.push(merged);
            }
        }

        Ok(result)
    }

    fn execute_query_segment(
        &self,
        segment: &QuerySegment,
        mut bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Execute MATCH clauses
        for match_clause in &segment.match_clauses {
            bindings = self.execute_match_clause(match_clause, bindings)?;
        }

        // Apply WHERE filter
        if let Some(where_expr) = &segment.where_clause {
            bindings.retain(|b| {
                self.evaluate_expression(where_expr, b)
                    .map(|v| matches!(v, Value::Bool(true)))
                    .unwrap_or(false)
            });
        }

        // Apply WITH clause if present
        if let Some(with_clause) = &segment.with_clause {
            bindings = self.apply_with_clause(with_clause, bindings)?;
        }

        Ok(bindings)
    }

    fn apply_with_clause(
        &self,
        with_clause: &WithClause,
        bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Determine effective column names (alias takes priority)
        let col_names: Vec<String> = with_clause
            .items
            .iter()
            .map(|item| {
                if let Some(ref alias) = item.alias {
                    alias.clone()
                } else {
                    match &item.expression {
                        ReturnItem::Alias(_, name) => name.clone(),
                        ReturnItem::Variable(v) => v.clone(),
                        ReturnItem::Property(v, p) => format!("{}.{}", v, p),
                        ReturnItem::Aggregate(agg) => self.aggregate_to_name(agg),
                        ReturnItem::Function(func) => self.function_to_name(func),
                        ReturnItem::All => "*".to_string(),
                        ReturnItem::Expr(e) => Self::expression_to_display(e),
                    }
                }
            })
            .collect();

        // Check if any item is an aggregate — if so, delegate to aggregated path
        let has_aggregation = with_clause
            .items
            .iter()
            .any(|item| Self::is_aggregate(&item.expression));

        if has_aggregation {
            // Build a temporary ReturnClause so we can reuse build_aggregated_result_set
            let temp_return_clause = ReturnClause {
                distinct: with_clause.distinct,
                items: with_clause
                    .items
                    .iter()
                    .map(|wi| {
                        // Wrap the expression in an alias if one was provided so that
                        // return_item_to_column_name produces the right column name.
                        if let Some(ref alias) = wi.alias {
                            ReturnItem::Alias(Box::new(wi.expression.clone()), alias.clone())
                        } else {
                            wi.expression.clone()
                        }
                    })
                    .collect(),
                order_by: with_clause.order_by.clone(),
                skip: with_clause.skip.clone(),
                limit: with_clause.limit.clone(),
            };

            let result_set = self.build_aggregated_result_set(&temp_return_clause, &bindings)?;

            // Convert each result row back to Bindings
            let result: Vec<Bindings> = result_set
                .rows
                .into_iter()
                .map(|row| {
                    let mut new_binding = Bindings::new();
                    for (col, value) in col_names.iter().zip(row.columns.into_iter()) {
                        match value {
                            Value::Node(id) | Value::NodeData { id, .. } => {
                                new_binding.insert(col.clone(), BindingValue::Node(id));
                            }
                            Value::Path { nodes, edges } => {
                                new_binding
                                    .insert(col.clone(), BindingValue::Path { nodes, edges });
                            }
                            other => {
                                new_binding.insert(col.clone(), BindingValue::Scalar(other));
                            }
                        }
                    }
                    new_binding
                })
                .collect();

            return Ok(result);
        }

        // No aggregation: project bindings row-by-row
        let mut result: Vec<Bindings> = Vec::new();

        for binding in &bindings {
            let mut new_binding = Bindings::new();

            for (item, var_name) in with_clause.items.iter().zip(col_names.iter()) {
                let value = self.evaluate_return_item(&item.expression, binding)?;

                // Convert Value back to BindingValue for the new binding
                match value {
                    Value::Node(id) | Value::NodeData { id, .. } => {
                        new_binding.insert(var_name.clone(), BindingValue::Node(id));
                    }
                    Value::Path { nodes, edges } => {
                        new_binding
                            .insert(var_name.clone(), BindingValue::Path { nodes, edges });
                    }
                    other => {
                        new_binding.insert(var_name.clone(), BindingValue::Scalar(other));
                    }
                }
            }

            result.push(new_binding);
        }

        // Apply DISTINCT if needed
        if with_clause.distinct {
            result = self.apply_distinct_bindings(result);
        }

        // Apply SKIP
        if let Some(ref skip_expr) = with_clause.skip {
            let skip = self.resolve_skip_limit(skip_expr)? as usize;
            result = result.into_iter().skip(skip).collect();
        }

        // Apply LIMIT
        if let Some(ref limit_expr) = with_clause.limit {
            let limit = self.resolve_skip_limit(limit_expr)? as usize;
            result = result.into_iter().take(limit).collect();
        }

        Ok(result)
    }

    fn apply_distinct_bindings(&self, bindings: Vec<Bindings>) -> Vec<Bindings> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();

        for binding in bindings {
            let key = format!("{:?}", binding);
            if seen.insert(key) {
                result.push(binding);
            }
        }

        result
    }

    fn aggregate_to_name(&self, agg: &AggregateFunction) -> String {
        match agg {
            AggregateFunction::Count(_) => "count".to_string(),
            AggregateFunction::Sum(_) => "sum".to_string(),
            AggregateFunction::Avg(_) => "avg".to_string(),
            AggregateFunction::Min(_) => "min".to_string(),
            AggregateFunction::Max(_) => "max".to_string(),
            AggregateFunction::Collect(_) => "collect".to_string(),
            AggregateFunction::PercentileCont(..) => "percentileCont".to_string(),
            AggregateFunction::PercentileDisc(..) => "percentileDisc".to_string(),
            AggregateFunction::StDev(_) => "stDev".to_string(),
            AggregateFunction::StDevP(_) => "stDevP".to_string(),
            AggregateFunction::CountDistinct(_) => "count".to_string(),
            AggregateFunction::SumDistinct(_) => "sum".to_string(),
            AggregateFunction::AvgDistinct(_) => "avg".to_string(),
            AggregateFunction::CollectDistinct(_) => "collect".to_string(),
        }
    }

    fn expression_to_display(expr: &Expression) -> String {
        match expr {
            Expression::Variable(v) => v.clone(),
            Expression::Property(v, p) => format!("{}.{}", v, p),
            Expression::Literal(lit) => match lit {
                Literal::Null => "null".to_string(),
                Literal::Bool(b) => b.to_string(),
                Literal::Int(n) => n.to_string(),
                Literal::Float(n) => n.to_string(),
                Literal::String(s) => format!("\"{}\"", s),
            },
            _ => "expr".to_string(),
        }
    }

    fn function_to_name(&self, func: &ScalarFunction) -> String {
        match func {
            ScalarFunction::Nodes(_) => "nodes".to_string(),
            ScalarFunction::Relationships(_) => "relationships".to_string(),
            ScalarFunction::Length(_) => "length".to_string(),
            ScalarFunction::ShortestPath { .. } => "shortestPath".to_string(),
            ScalarFunction::AllShortestPaths { .. } => "allShortestPaths".to_string(),
            ScalarFunction::Trim(_) => "trim".to_string(),
            ScalarFunction::LTrim(_) => "ltrim".to_string(),
            ScalarFunction::RTrim(_) => "rtrim".to_string(),
            ScalarFunction::ToLower(_) => "toLower".to_string(),
            ScalarFunction::ToUpper(_) => "toUpper".to_string(),
            ScalarFunction::Reverse(_) => "reverse".to_string(),
            ScalarFunction::ToString(_) => "toString".to_string(),
            ScalarFunction::Size(_) => "size".to_string(),
            ScalarFunction::Left(..) => "left".to_string(),
            ScalarFunction::Right(..) => "right".to_string(),
            ScalarFunction::Substring(..) => "substring".to_string(),
            ScalarFunction::Split(..) => "split".to_string(),
            ScalarFunction::Replace(..) => "replace".to_string(),
            ScalarFunction::Abs(_) => "abs".to_string(),
            ScalarFunction::Ceil(_) => "ceil".to_string(),
            ScalarFunction::Floor(_) => "floor".to_string(),
            ScalarFunction::Round(..) => "round".to_string(),
            ScalarFunction::Sign(_) => "sign".to_string(),
            ScalarFunction::Rand => "rand".to_string(),
            ScalarFunction::IsNaN(_) => "isNaN".to_string(),
            ScalarFunction::Log(_) => "log".to_string(),
            ScalarFunction::Log10(_) => "log10".to_string(),
            ScalarFunction::Sqrt(_) => "sqrt".to_string(),
            ScalarFunction::E => "e".to_string(),
            ScalarFunction::Pi => "pi".to_string(),
            ScalarFunction::Id(_) => "id".to_string(),
            ScalarFunction::ElementId(_) => "elementId".to_string(),
            ScalarFunction::Type(_) => "type".to_string(),
            ScalarFunction::StartNode(_) => "startNode".to_string(),
            ScalarFunction::EndNode(_) => "endNode".to_string(),
            ScalarFunction::Labels(_) => "labels".to_string(),
            ScalarFunction::Properties(_) => "properties".to_string(),
            ScalarFunction::Keys(_) => "keys".to_string(),
            ScalarFunction::Coalesce(_) => "coalesce".to_string(),
            ScalarFunction::NullIf(..) => "nullIf".to_string(),
            ScalarFunction::ToBoolean(_) => "toBoolean".to_string(),
            ScalarFunction::ToFloat(_) => "toFloat".to_string(),
            ScalarFunction::ToInteger(_) => "toInteger".to_string(),
            ScalarFunction::Timestamp => "timestamp".to_string(),
            ScalarFunction::RandomUUID => "randomUUID".to_string(),
            ScalarFunction::Head(_) => "head".to_string(),
            ScalarFunction::Last(_) => "last".to_string(),
            ScalarFunction::Tail(_) => "tail".to_string(),
            ScalarFunction::Range(..) => "range".to_string(),
            ScalarFunction::Reduce { .. } => "reduce".to_string(),
            ScalarFunction::DateFunc(_) => "date".to_string(),
            ScalarFunction::DateTimeFunc(_) => "datetime".to_string(),
            ScalarFunction::DurationFunc(_) => "duration".to_string(),
        }
    }

    fn execute_match_clause(
        &self,
        clause: &MatchClause,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        if clause.optional {
            // OPTIONAL MATCH: if no matches, keep original bindings with NULL for new variables
            let mut result = Vec::new();

            for bindings in current_bindings {
                let mut matches = vec![bindings.clone()];
                for pattern in &clause.patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }

                if matches.is_empty() {
                    // No matches found - keep original bindings (variables from this clause will be NULL/unbound)
                    result.push(bindings);
                } else {
                    // Found matches - use them
                    result.extend(matches);
                }
            }

            Ok(result)
        } else {
            // Regular MATCH: process each input binding lazily through all patterns.
            // Processing one binding at a time reduces peak memory for multi-pattern
            // queries, since the intermediate expansion of one binding is discarded
            // before the next input binding is processed. This avoids the O(M^N) peak
            // that would occur if we fully expanded all bindings after each pattern step.
            let mut result = Vec::new();
            for b in current_bindings {
                let expanded = self.match_patterns_for_binding(&clause.patterns, b)?;
                result.extend(expanded);
            }
            Ok(result)
        }
    }

    fn match_pattern(
        &self,
        pattern: &Pattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        match pattern {
            Pattern::Node(node_pattern) => self.match_node_pattern(node_pattern, current_bindings),
            Pattern::Path(path_pattern) => self.match_path_pattern(path_pattern, current_bindings),
        }
    }

    /// Chains a single binding through multiple patterns lazily.
    ///
    /// Unlike [`match_pattern`] (which processes all input bindings as a batch),
    /// this method threads one input binding through each pattern in sequence and
    /// short-circuits as soon as any pattern produces zero matches, avoiding
    /// unnecessary work and reducing peak memory usage for multi-pattern queries.
    fn match_patterns_for_binding(
        &self,
        patterns: &[Pattern],
        initial: Bindings,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut matches = vec![initial];
        for pattern in patterns {
            if matches.is_empty() {
                break;
            }
            matches = self.match_pattern(pattern, matches)?;
        }
        Ok(matches)
    }

    fn match_node_pattern(
        &self,
        pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        // Collect all graph node IDs once for the scan-all path.
        // We reuse this Vec across bindings to avoid repeated allocations.
        let all_node_ids: Vec<NodeId> = self.graph_ref().node_ids();

        for bindings in current_bindings {
            // Check if variable is already bound
            if let Some(var) = &pattern.variable
                && let Some(bound_value) = bindings.get(var)
            {
                if let Some(bound_id) = bound_value.as_node() {
                    // Variable already bound, check if it matches
                    if self.node_matches_pattern(bound_id, pattern, &bindings)? {
                        result.push(bindings);
                    }
                }
                continue;
            }

            // Try index-based lookup first: if the pattern has a label and at least one
            // property that is indexed and has a literal value, use the index to avoid
            // a full scan.
            let mut used_index = false;
            if let Some(label) = pattern.labels.first() {
                'index_loop: for (prop_key, prop_expr) in &pattern.properties {
                    if self.property_index.has_index(label, prop_key)
                        && let Expression::Literal(lit) = prop_expr
                    {
                        let prop_val = PropertyValue::from(lit.clone());
                        let candidate_ids = self.property_index.find_by_property(prop_key, &prop_val);
                        for node_id in candidate_ids {
                            if self.node_matches_pattern(node_id, pattern, &bindings)? {
                                let mut new_bindings = bindings.clone();
                                if let Some(var) = &pattern.variable {
                                    new_bindings.insert(var.clone(), BindingValue::Node(node_id));
                                }
                                result.push(new_bindings);
                            }
                        }
                        used_index = true;
                        break 'index_loop;
                    }
                }
            }

            if used_index {
                continue;
            }

            // Find matching nodes.
            // For large graphs use parallel filtering to determine which node IDs
            // match the label/property predicates. Property expression evaluation
            // only reads the graph, so sharing `&self` across threads is safe here
            // through the pattern closure.
            if all_node_ids.len() >= PARALLEL_MATCH_THRESHOLD {
                // Phase 1 (parallel): determine which node IDs pass label + property filters.
                // Note: evaluate_expression needs &self; we wrap it in a closure that
                // captures an immutable `self` reference — safe because all reads are
                // independent (no mutation occurs during MATCH).
                let matching_ids: Vec<Result<Option<NodeId>, ExecuteError>> = all_node_ids
                    .par_iter()
                    .map(|&node_id| {
                        if self.node_matches_pattern(node_id, pattern, &bindings)? {
                            Ok(Some(node_id))
                        } else {
                            Ok(None)
                        }
                    })
                    .collect();

                // Phase 2 (sequential): assemble bindings for matched nodes.
                for res in matching_ids {
                    let node_id = match res? {
                        Some(id) => id,
                        None => continue,
                    };
                    let mut new_bindings = bindings.clone();
                    if let Some(var) = &pattern.variable {
                        new_bindings.insert(var.clone(), BindingValue::Node(node_id));
                    }
                    result.push(new_bindings);
                }
            } else {
                // Sequential path for small graphs.
                for &node_id in &all_node_ids {
                    if self.node_matches_pattern(node_id, pattern, &bindings)? {
                        let mut new_bindings = bindings.clone();
                        if let Some(var) = &pattern.variable {
                            new_bindings.insert(var.clone(), BindingValue::Node(node_id));
                        }
                        result.push(new_bindings);
                    }
                }
            }
        }

        Ok(result)
    }

    fn match_path_pattern(
        &self,
        pattern: &PathPattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Start with matching the start node
        let mut bindings = self.match_node_pattern(&pattern.start, current_bindings)?;

        // Match each segment
        for segment in &pattern.segments {
            bindings = self.match_segment(segment, &pattern.start, bindings)?;
        }

        Ok(bindings)
    }

    fn match_segment(
        &self,
        segment: &PathSegment,
        prev_pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Check if this is a variable-length path
        if let Some(ref range) = segment.edge.length_range {
            return self.match_variable_length_segment(
                segment,
                prev_pattern,
                current_bindings,
                range,
            );
        }

        // Single-hop matching
        let mut result = Vec::new();

        for bindings in current_bindings {
            let matches = self.match_single_hop(segment, prev_pattern, &bindings)?;
            result.extend(matches);
        }

        Ok(result)
    }

    fn match_single_hop(
        &self,
        segment: &PathSegment,
        prev_pattern: &NodePattern,
        bindings: &Bindings,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        // Get the previous node
        let prev_var = prev_pattern
            .variable
            .as_ref()
            .ok_or_else(|| ExecuteError::TypeError("path pattern requires variable".to_string()))?;

        let prev_id = bindings
            .get(prev_var)
            .and_then(|v| v.as_node())
            .ok_or_else(|| ExecuteError::UndefinedVariable(prev_var.clone()))?;

        // Get edges from previous node
        let edges = self.get_edges_by_direction(prev_id, segment.edge.direction);

        for edge in edges {
            // Check edge type
            if let Some(ref edge_type) = segment.edge.edge_type
                && &edge.label != edge_type
            {
                continue;
            }

            // Get the other node
            let next_id = self.get_next_node(prev_id, &edge, segment.edge.direction);

            // Check if next node matches pattern
            if self.node_matches_pattern(next_id, &segment.node, bindings)? {
                let mut new_bindings = bindings.clone();

                if let Some(var) = &segment.node.variable {
                    new_bindings.insert(var.clone(), BindingValue::Node(next_id));
                }
                if let Some(var) = &segment.edge.variable {
                    new_bindings.insert(var.clone(), BindingValue::Edge(edge.id));
                }

                result.push(new_bindings);
            }
        }

        Ok(result)
    }

    fn match_variable_length_segment(
        &self,
        segment: &PathSegment,
        prev_pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
        range: &LengthRange,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();
        let max_depth = range.max.unwrap_or(10); // Default max depth to prevent infinite loops

        for bindings in current_bindings {
            let prev_var = prev_pattern.variable.as_ref().ok_or_else(|| {
                ExecuteError::TypeError("path pattern requires variable".to_string())
            })?;

            let start_id = bindings
                .get(prev_var)
                .and_then(|v| v.as_node())
                .ok_or_else(|| ExecuteError::UndefinedVariable(prev_var.clone()))?;

            // BFS to find all reachable nodes within the range
            // Track: (current_node, depth, path_edges, visited_nodes)
            let mut visited_paths: Vec<(NodeId, u32, Vec<u64>, Vec<NodeId>)> =
                vec![(start_id, 0, vec![], vec![start_id])];
            let mut found_paths: Vec<(NodeId, Vec<u64>, Vec<NodeId>)> = Vec::new();

            while let Some((current_id, depth, path_edges, visited_nodes)) = visited_paths.pop() {
                // If within range and matches target pattern, add to results
                if depth >= range.min
                    && self.node_matches_pattern(current_id, &segment.node, &bindings)?
                {
                    found_paths.push((current_id, path_edges.clone(), visited_nodes.clone()));
                }

                // Don't explore further if at max depth
                if depth >= max_depth {
                    continue;
                }

                // Get edges and explore neighbors
                let edges = self.get_edges_by_direction(current_id, segment.edge.direction);

                for edge in edges {
                    // Check edge type
                    if let Some(ref edge_type) = segment.edge.edge_type
                        && &edge.label != edge_type
                    {
                        continue;
                    }

                    let next_id = self.get_next_node(current_id, &edge, segment.edge.direction);

                    // Avoid cycles: don't revisit nodes already in the current path
                    if visited_nodes.contains(&next_id) {
                        continue;
                    }

                    let mut new_path = path_edges.clone();
                    new_path.push(edge.id);

                    let mut new_visited = visited_nodes.clone();
                    new_visited.push(next_id);

                    visited_paths.push((next_id, depth + 1, new_path, new_visited));
                }
            }

            // Convert found paths to bindings
            for (end_id, path_edges, path_nodes) in found_paths {
                let mut new_bindings = bindings.clone();

                // Bind the end node
                if let Some(var) = &segment.node.variable {
                    new_bindings.insert(var.clone(), BindingValue::Node(end_id));
                }

                // Bind the path variable (edge list) if specified
                if let Some(var) = &segment.edge.variable {
                    new_bindings.insert(
                        var.clone(),
                        BindingValue::Path {
                            nodes: path_nodes,
                            edges: path_edges,
                        },
                    );
                }

                result.push(new_bindings);
            }
        }

        Ok(result)
    }

    fn get_edges_by_direction(&self, node_id: NodeId, direction: EdgeDirection) -> Vec<Edge> {
        match direction {
            EdgeDirection::Outgoing => self.graph_ref().outgoing_edges(node_id),
            EdgeDirection::Incoming => self.graph_ref().incoming_edges(node_id),
            EdgeDirection::Both => {
                let mut edges = self.graph_ref().outgoing_edges(node_id);
                edges.extend(self.graph_ref().incoming_edges(node_id));
                edges
            }
        }
    }

    fn get_next_node(&self, current_id: NodeId, edge: &Edge, direction: EdgeDirection) -> NodeId {
        match direction {
            EdgeDirection::Outgoing => edge.to,
            EdgeDirection::Incoming => edge.from,
            EdgeDirection::Both => {
                if edge.from == current_id {
                    edge.to
                } else {
                    edge.from
                }
            }
        }
    }

    fn node_matches_pattern(
        &self,
        node_id: NodeId,
        pattern: &NodePattern,
        bindings: &Bindings,
    ) -> Result<bool, ExecuteError> {
        let backend = self.graph_ref();

        // Check labels without cloning the whole node. A node must have ALL
        // labels specified in the pattern (AND). When the pattern has labels,
        // this also confirms the node exists.
        if !pattern.labels.is_empty() {
            if !backend.node_has_all_labels(node_id, &pattern.labels) {
                return Ok(false);
            }
        } else if pattern.properties.is_empty() && !backend.contains_node(node_id) {
            // No labels and no properties: still require the node to exist.
            return Ok(false);
        }

        // Check properties, cloning only the individual property values needed
        // rather than the entire node.
        for (key, expected_expr) in &pattern.properties {
            match backend.get_node_property(node_id, key) {
                Some(actual) => {
                    let expected_val = self.evaluate_expression(expected_expr, bindings)?;
                    if !self.property_value_matches(&actual, &expected_val) {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }

        Ok(true)
    }

    fn property_value_matches(&self, actual: &PropertyValue, expected: &Value) -> bool {
        match (actual, expected) {
            (PropertyValue::Null, Value::Null) => true,
            (PropertyValue::Bool(a), Value::Bool(e)) => a == e,
            (PropertyValue::Int(a), Value::Int(e)) => a == e,
            (PropertyValue::Float(a), Value::Float(e)) => (a - e).abs() < f64::EPSILON,
            (PropertyValue::Int(a), Value::Float(e)) => (*a as f64 - e).abs() < f64::EPSILON,
            (PropertyValue::Float(a), Value::Int(e)) => (a - *e as f64).abs() < f64::EPSILON,
            (PropertyValue::String(a), Value::String(e)) => a == e,
            _ => false,
        }
    }

    fn build_result_set(
        &self,
        return_clause: &ReturnClause,
        bindings_list: &[Bindings],
    ) -> Result<ResultSet, ExecuteError> {
        // Check if any aggregation is present
        let has_aggregation = return_clause
            .items
            .iter()
            .any(|item| Self::is_aggregate(item));

        if has_aggregation {
            return self.build_aggregated_result_set(return_clause, bindings_list);
        }

        // Build column names
        let columns: Vec<String> = return_clause
            .items
            .iter()
            .map(|item| self.return_item_to_column_name(item))
            .collect();

        // Build rows
        let mut rows = Vec::new();

        for bindings in bindings_list {
            let mut row_values = Vec::new();

            for item in &return_clause.items {
                row_values.push(self.evaluate_return_item(item, bindings)?);
            }

            rows.push(Row {
                columns: row_values,
            });
        }

        // Apply DISTINCT
        if return_clause.distinct {
            rows = self.apply_distinct(rows);
        }

        // Resolve SKIP and LIMIT expressions (integer literal or $parameter)
        let resolved_skip = return_clause
            .skip
            .as_ref()
            .map(|e| self.resolve_skip_limit(e))
            .transpose()?;
        let resolved_limit = return_clause
            .limit
            .as_ref()
            .map(|e| self.resolve_skip_limit(e))
            .transpose()?;

        // Apply ORDER BY with optional LIMIT optimization
        if let Some(ref order_by) = return_clause.order_by {
            // Calculate how many rows we actually need
            let needed = match (resolved_skip, resolved_limit) {
                (Some(skip), Some(limit)) => Some((skip + limit) as usize),
                (None, Some(limit)) => Some(limit as usize),
                _ => None,
            };

            // Use optimized TopN selection if we need fewer rows than we have
            if let Some(n) = needed {
                if n < rows.len() {
                    rows = self.apply_order_by_topn(rows, order_by, &columns, n);
                } else {
                    self.apply_order_by(&mut rows, order_by, &columns);
                }
            } else {
                self.apply_order_by(&mut rows, order_by, &columns);
            }
        }

        // Apply SKIP
        if let Some(skip) = resolved_skip {
            let skip = skip as usize;
            if skip < rows.len() {
                rows = rows.into_iter().skip(skip).collect();
            } else {
                rows.clear();
            }
        }

        // Apply LIMIT
        if let Some(limit) = resolved_limit {
            rows.truncate(limit as usize);
        }

        Ok(ResultSet::new(columns, rows))
    }

    fn apply_distinct(&self, rows: Vec<Row>) -> Vec<Row> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();

        for row in rows {
            let key = self.row_to_key(&row);
            if seen.insert(key) {
                result.push(row);
            }
        }

        result
    }

    fn row_to_key(&self, row: &Row) -> String {
        row.columns
            .iter()
            .map(|v| format!("{:?}", v))
            .collect::<Vec<_>>()
            .join("|")
    }

    fn apply_order_by(&self, rows: &mut [Row], order_by: &OrderByClause, columns: &[String]) {
        rows.sort_by(|a, b| self.compare_rows(a, b, order_by, columns));
    }

    /// Memory-efficient TopN selection using partial sort
    /// Only keeps the top N rows, reducing memory usage for large result sets
    fn apply_order_by_topn(
        &self,
        mut rows: Vec<Row>,
        order_by: &OrderByClause,
        columns: &[String],
        n: usize,
    ) -> Vec<Row> {
        if rows.len() <= n {
            self.apply_order_by(&mut rows, order_by, columns);
            return rows;
        }

        // Use partial_sort via select_nth_unstable_by for efficiency
        // This partitions the array so that the first n elements are the smallest
        rows.select_nth_unstable_by(n, |a, b| self.compare_rows(a, b, order_by, columns));

        // Truncate to keep only the top N
        rows.truncate(n);

        // Sort the top N elements
        self.apply_order_by(&mut rows, order_by, columns);

        rows
    }

    fn compare_rows(
        &self,
        a: &Row,
        b: &Row,
        order_by: &OrderByClause,
        columns: &[String],
    ) -> std::cmp::Ordering {
        for item in &order_by.items {
            let col_name = match &item.expression {
                OrderByExpression::Variable(v) => v.clone(),
                OrderByExpression::Property(v, p) => format!("{}.{}", v, p),
            };

            let col_idx = columns.iter().position(|c| c == &col_name);

            if let Some(idx) = col_idx {
                let cmp = self.compare_values_for_sort(
                    &a.columns[idx],
                    &b.columns[idx],
                    item.direction,
                    item.nulls_order,
                );
                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
        }
        std::cmp::Ordering::Equal
    }

    fn compare_values_for_sort(
        &self,
        a: &Value,
        b: &Value,
        direction: OrderDirection,
        nulls_order: NullsOrder,
    ) -> std::cmp::Ordering {
        // Determine where NULLs should go (independent of ASC/DESC for the actual values)
        let nulls_last = match nulls_order {
            NullsOrder::First => false,
            NullsOrder::Last => true,
            NullsOrder::Default => {
                // Default: ASC -> NULLS LAST, DESC -> NULLS FIRST
                matches!(direction, OrderDirection::Asc)
            }
        };

        // Handle NULL comparisons (these are final and not affected by ASC/DESC)
        match (a, b) {
            (Value::Null, Value::Null) => return std::cmp::Ordering::Equal,
            (Value::Null, _) => {
                return if nulls_last {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                };
            }
            (_, Value::Null) => {
                return if nulls_last {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            _ => {}
        }

        // Non-NULL comparisons: apply ASC/DESC
        let base_cmp = match (a, b) {
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Value::Int(a), Value::Float(b)) => (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::Float(a), Value::Int(b)) => a
                .partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::String(a), Value::String(b)) => a.cmp(b),
            _ => std::cmp::Ordering::Equal,
        };

        // Apply direction for non-NULL values
        match direction {
            OrderDirection::Asc => base_cmp,
            OrderDirection::Desc => base_cmp.reverse(),
        }
    }

    /// Alias の内側を再帰的に辿って、集計関数かどうかを判定するヘルパー。
    fn is_aggregate(item: &ReturnItem) -> bool {
        match item {
            ReturnItem::Aggregate(_) => true,
            ReturnItem::Alias(inner, _) => Self::is_aggregate(inner),
            _ => false,
        }
    }

    fn return_item_to_column_name(&self, item: &ReturnItem) -> String {
        match item {
            ReturnItem::Alias(_, name) => name.clone(),
            ReturnItem::Variable(v) => v.clone(),
            ReturnItem::Property(v, p) => format!("{}.{}", v, p),
            ReturnItem::All => "*".to_string(),
            ReturnItem::Expr(e) => Self::expression_to_display(e),
            ReturnItem::Aggregate(agg) => match agg {
                AggregateFunction::Count(inner) => match inner {
                    None => "COUNT(*)".to_string(),
                    Some(inner) => format!("count({})", self.return_item_to_column_name(inner)),
                },
                AggregateFunction::Sum(inner) => {
                    format!("SUM({})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::Avg(inner) => {
                    format!("AVG({})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::Min(inner) => {
                    format!("MIN({})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::Max(inner) => {
                    format!("MAX({})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::Collect(inner) => {
                    format!("COLLECT({})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::PercentileCont(e, p) => format!(
                    "percentileCont({}, {})",
                    self.return_item_to_column_name(e),
                    self.return_item_to_column_name(p)
                ),
                AggregateFunction::PercentileDisc(e, p) => format!(
                    "percentileDisc({}, {})",
                    self.return_item_to_column_name(e),
                    self.return_item_to_column_name(p)
                ),
                AggregateFunction::StDev(inner) => {
                    format!("stDev({})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::StDevP(inner) => {
                    format!("stDevP({})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::CountDistinct(inner) => {
                    format!("COUNT(DISTINCT {})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::SumDistinct(inner) => {
                    format!("SUM(DISTINCT {})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::AvgDistinct(inner) => {
                    format!("AVG(DISTINCT {})", self.return_item_to_column_name(inner))
                }
                AggregateFunction::CollectDistinct(inner) => {
                    format!(
                        "COLLECT(DISTINCT {})",
                        self.return_item_to_column_name(inner)
                    )
                }
            },
            ReturnItem::Function(func) => match func {
                ScalarFunction::Nodes(var) => format!("nodes({})", var),
                ScalarFunction::Relationships(var) => format!("relationships({})", var),
                ScalarFunction::Length(var) => format!("length({})", var),
                ScalarFunction::ShortestPath { start, end } => {
                    format!("shortestPath({}, {})", start, end)
                }
                ScalarFunction::AllShortestPaths { start, end } => {
                    format!("allShortestPaths({}, {})", start, end)
                }
                ScalarFunction::Trim(e) => format!("trim({})", Self::expression_to_display(e)),
                ScalarFunction::LTrim(e) => format!("ltrim({})", Self::expression_to_display(e)),
                ScalarFunction::RTrim(e) => format!("rtrim({})", Self::expression_to_display(e)),
                ScalarFunction::ToLower(e) => {
                    format!("toLower({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToUpper(e) => {
                    format!("toUpper({})", Self::expression_to_display(e))
                }
                ScalarFunction::Reverse(e) => {
                    format!("reverse({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToString(e) => {
                    format!("toString({})", Self::expression_to_display(e))
                }
                ScalarFunction::Size(e) => format!("size({})", Self::expression_to_display(e)),
                ScalarFunction::Left(e1, e2) => format!(
                    "left({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::Right(e1, e2) => format!(
                    "right({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::Substring(e1, e2, e3) => {
                    if let Some(e3) = e3 {
                        format!(
                            "substring({}, {}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2),
                            Self::expression_to_display(e3)
                        )
                    } else {
                        format!(
                            "substring({}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2)
                        )
                    }
                }
                ScalarFunction::Split(e1, e2) => format!(
                    "split({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::Replace(e1, e2, e3) => format!(
                    "replace({}, {}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2),
                    Self::expression_to_display(e3)
                ),
                ScalarFunction::Abs(e) => format!("abs({})", Self::expression_to_display(e)),
                ScalarFunction::Ceil(e) => format!("ceil({})", Self::expression_to_display(e)),
                ScalarFunction::Floor(e) => format!("floor({})", Self::expression_to_display(e)),
                ScalarFunction::Round(e, p) => {
                    if let Some(p) = p {
                        format!(
                            "round({}, {})",
                            Self::expression_to_display(e),
                            Self::expression_to_display(p)
                        )
                    } else {
                        format!("round({})", Self::expression_to_display(e))
                    }
                }
                ScalarFunction::Sign(e) => format!("sign({})", Self::expression_to_display(e)),
                ScalarFunction::Rand => "rand()".to_string(),
                ScalarFunction::IsNaN(e) => format!("isNaN({})", Self::expression_to_display(e)),
                ScalarFunction::Log(e) => format!("log({})", Self::expression_to_display(e)),
                ScalarFunction::Log10(e) => format!("log10({})", Self::expression_to_display(e)),
                ScalarFunction::Sqrt(e) => format!("sqrt({})", Self::expression_to_display(e)),
                ScalarFunction::E => "e()".to_string(),
                ScalarFunction::Pi => "pi()".to_string(),
                ScalarFunction::Id(var) => format!("id({})", var),
                ScalarFunction::ElementId(var) => format!("elementId({})", var),
                ScalarFunction::Type(var) => format!("type({})", var),
                ScalarFunction::StartNode(var) => format!("startNode({})", var),
                ScalarFunction::EndNode(var) => format!("endNode({})", var),
                ScalarFunction::Labels(var) => format!("labels({})", var),
                ScalarFunction::Properties(var) => format!("properties({})", var),
                ScalarFunction::Keys(var) => format!("keys({})", var),
                ScalarFunction::Coalesce(_) => "coalesce(...)".to_string(),
                ScalarFunction::NullIf(e1, e2) => format!(
                    "nullIf({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::ToBoolean(e) => {
                    format!("toBoolean({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToFloat(e) => {
                    format!("toFloat({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToInteger(e) => {
                    format!("toInteger({})", Self::expression_to_display(e))
                }
                ScalarFunction::Timestamp => "timestamp()".to_string(),
                ScalarFunction::RandomUUID => "randomUUID()".to_string(),
                ScalarFunction::Head(e) => format!("head({})", Self::expression_to_display(e)),
                ScalarFunction::Last(e) => format!("last({})", Self::expression_to_display(e)),
                ScalarFunction::Tail(e) => format!("tail({})", Self::expression_to_display(e)),
                ScalarFunction::Range(e1, e2, e3) => {
                    if let Some(e3) = e3 {
                        format!(
                            "range({}, {}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2),
                            Self::expression_to_display(e3)
                        )
                    } else {
                        format!(
                            "range({}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2)
                        )
                    }
                }
                ScalarFunction::Reduce { .. } => "reduce(...)".to_string(),
                ScalarFunction::DateFunc(_) => "date(...)".to_string(),
                ScalarFunction::DateTimeFunc(_) => "datetime(...)".to_string(),
                ScalarFunction::DurationFunc(_) => "duration(...)".to_string(),
            },
        }
    }

    fn evaluate_return_item(
        &self,
        item: &ReturnItem,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        match item {
            ReturnItem::Variable(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                Ok(Value::NodeData {
                                    id: *node_id,
                                    labels: node.labels.clone(),
                                    properties: Arc::clone(&node.properties),
                                })
                            } else {
                                Ok(Value::Node(*node_id))
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            // Return edge as a simple value
                            Ok(Value::Int(*edge_id as i64))
                        }
                        BindingValue::Path { nodes, edges } => Ok(Value::Path {
                            nodes: nodes.clone(),
                            edges: edges.clone(),
                        }),
                        BindingValue::Scalar(value) => Ok(value.clone()),
                    }
                } else {
                    Ok(Value::Null)
                }
            }
            ReturnItem::Property(var, prop) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                Ok(node.get_property(prop).map(Value::from).unwrap_or(Value::Null))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                Ok(edge.get_property(prop).map(Value::from).unwrap_or(Value::Null))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Scalar(scalar_val) => {
                            Self::access_temporal_field(scalar_val, prop)
                        }
                        _ => Ok(Value::Null),
                    }
                } else {
                    Ok(Value::Null)
                }
            }
            ReturnItem::All => {
                // For *, we return the first bound node variable
                for binding_value in bindings.values() {
                    if let BindingValue::Node(node_id) = binding_value
                        && let Some(node) = self.graph_ref().get_node(*node_id)
                    {
                        return Ok(Value::NodeData {
                            id: *node_id,
                            labels: node.labels.clone(),
                            properties: Arc::clone(&node.properties),
                        });
                    }
                }
                Ok(Value::Null)
            }
            ReturnItem::Alias(inner, _) => self.evaluate_return_item(inner, bindings),
            ReturnItem::Aggregate(_) => {
                // Aggregates are handled separately
                Ok(Value::Null)
            }
            ReturnItem::Function(func) => self.evaluate_scalar_function(func, bindings),
            ReturnItem::Expr(expr) => self.evaluate_expression(expr, bindings),
        }
    }

    fn evaluate_scalar_function(
        &self,
        func: &ScalarFunction,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        match func {
            ScalarFunction::Nodes(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    if let BindingValue::Path { nodes, .. } = binding_value {
                        // Return list of node data
                        let node_values: Vec<Value> = nodes
                            .iter()
                            .map(|&node_id| {
                                if let Some(node) = self.graph_ref().get_node(node_id) {
                                    Value::NodeData {
                                        id: node_id,
                                        labels: node.labels.clone(),
                                        properties: Arc::clone(&node.properties),
                                    }
                                } else {
                                    Value::Node(node_id)
                                }
                            })
                            .collect();
                        Ok(Value::List(node_values))
                    } else {
                        Err(ExecuteError::TypeError(format!(
                            "nodes() requires a path variable, got {:?}",
                            binding_value
                        )))
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Relationships(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    if let BindingValue::Path { edges, .. } = binding_value {
                        // Return list of edge IDs (or edge data if available)
                        let edge_values: Vec<Value> = edges
                            .iter()
                            .map(|&edge_id| Value::Int(edge_id as i64))
                            .collect();
                        Ok(Value::List(edge_values))
                    } else {
                        Err(ExecuteError::TypeError(format!(
                            "relationships() requires a path variable, got {:?}",
                            binding_value
                        )))
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Length(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    if let BindingValue::Path { edges, .. } = binding_value {
                        Ok(Value::Int(edges.len() as i64))
                    } else {
                        Err(ExecuteError::TypeError(format!(
                            "length() requires a path variable, got {:?}",
                            binding_value
                        )))
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::ShortestPath { start, end } => {
                let start_id = bindings
                    .get(start)
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(start.clone()))?;
                let end_id = bindings
                    .get(end)
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(end.clone()))?;

                if let Some(path) = traversal::shortest_path(self.graph_ref(),start_id, end_id) {
                    let edges = self.extract_edge_ids(&path.nodes);
                    Ok(Value::Path {
                        nodes: path.nodes,
                        edges,
                    })
                } else {
                    Ok(Value::Null)
                }
            }
            ScalarFunction::AllShortestPaths { start, end } => {
                let start_id = bindings
                    .get(start)
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(start.clone()))?;
                let end_id = bindings
                    .get(end)
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(end.clone()))?;

                let paths = traversal::all_shortest_paths(self.graph_ref(),start_id, end_id);
                let path_values: Vec<Value> = paths
                    .into_iter()
                    .map(|path| {
                        let edges = self.extract_edge_ids(&path.nodes);
                        Value::Path {
                            nodes: path.nodes,
                            edges,
                        }
                    })
                    .collect();

                Ok(Value::List(path_values))
            }
            ScalarFunction::Trim(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.trim().to_string())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "trim() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::LTrim(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.trim_start().to_string())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "ltrim() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::RTrim(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.trim_end().to_string())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "rtrim() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::ToLower(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.to_lowercase())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "toLower() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::ToUpper(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.to_uppercase())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "toUpper() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::Reverse(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.chars().rev().collect())),
                    Value::List(items) => Ok(Value::List(items.into_iter().rev().collect())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "reverse() requires a string or list".to_string(),
                    )),
                }
            }
            ScalarFunction::ToString(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s)),
                    Value::Int(n) => Ok(Value::String(format!("{}", n))),
                    Value::Float(n) => Ok(Value::String(format!("{}", n))),
                    Value::Bool(b) => {
                        Ok(Value::String(if b { "true" } else { "false" }.to_string()))
                    }
                    Value::Null => Ok(Value::String("null".to_string())),
                    _ => Err(ExecuteError::TypeError(
                        "toString() unsupported type".to_string(),
                    )),
                }
            }
            ScalarFunction::Size(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::Int(s.chars().count() as i64)),
                    Value::List(items) => Ok(Value::Int(items.len() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "size() requires a string or list".to_string(),
                    )),
                }
            }
            ScalarFunction::Left(str_expr, len_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let len_val = self.evaluate_expression(len_expr, bindings)?;
                match (&s_val, &len_val) {
                    (Value::String(s), Value::Int(len)) => {
                        let len = *len as usize;
                        Ok(Value::String(s.chars().take(len).collect()))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "left() requires (string, int)".to_string(),
                    )),
                }
            }
            ScalarFunction::Right(str_expr, len_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let len_val = self.evaluate_expression(len_expr, bindings)?;
                match (&s_val, &len_val) {
                    (Value::String(s), Value::Int(len)) => {
                        let len = *len as usize;
                        let chars: Vec<char> = s.chars().collect();
                        let start = chars.len().saturating_sub(len);
                        Ok(Value::String(chars[start..].iter().collect()))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "right() requires (string, int)".to_string(),
                    )),
                }
            }
            ScalarFunction::Substring(str_expr, start_expr, len_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let start_val = self.evaluate_expression(start_expr, bindings)?;
                let len_val = len_expr
                    .as_ref()
                    .map(|e| self.evaluate_expression(e, bindings))
                    .transpose()?;
                match (&s_val, &start_val) {
                    (Value::String(s), Value::Int(start)) => {
                        let start = *start as usize;
                        let chars: Vec<char> = s.chars().collect();
                        if start >= chars.len() {
                            return Ok(Value::String(String::new()));
                        }
                        match len_val {
                            Some(Value::Int(len)) => {
                                let len = len as usize;
                                Ok(Value::String(chars[start..].iter().take(len).collect()))
                            }
                            None => Ok(Value::String(chars[start..].iter().collect())),
                            Some(Value::Null) => Ok(Value::Null),
                            _ => Err(ExecuteError::TypeError(
                                "substring() length must be an integer".to_string(),
                            )),
                        }
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "substring() requires (string, int)".to_string(),
                    )),
                }
            }
            ScalarFunction::Split(str_expr, delim_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let d_val = self.evaluate_expression(delim_expr, bindings)?;
                match (&s_val, &d_val) {
                    (Value::String(s), Value::String(delim)) => {
                        let parts: Vec<Value> = s
                            .split(delim.as_str())
                            .map(|p| Value::String(p.to_string()))
                            .collect();
                        Ok(Value::List(parts))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "split() requires (string, string)".to_string(),
                    )),
                }
            }
            ScalarFunction::Replace(str_expr, search_expr, rep_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let search_val = self.evaluate_expression(search_expr, bindings)?;
                let rep_val = self.evaluate_expression(rep_expr, bindings)?;
                match (&s_val, &search_val, &rep_val) {
                    (Value::String(s), Value::String(search), Value::String(rep)) => {
                        Ok(Value::String(s.replace(search.as_str(), rep.as_str())))
                    }
                    (Value::Null, _, _) | (_, Value::Null, _) | (_, _, Value::Null) => {
                        Ok(Value::Null)
                    }
                    _ => Err(ExecuteError::TypeError(
                        "replace() requires (string, string, string)".to_string(),
                    )),
                }
            }
            ScalarFunction::Abs(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n.abs())),
                    Value::Float(n) => Ok(Value::Float(n.abs())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "abs() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Ceil(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n)),
                    Value::Float(n) => Ok(Value::Int(n.ceil() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "ceil() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Floor(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n)),
                    Value::Float(n) => Ok(Value::Int(n.floor() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "floor() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Round(expr, precision_expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => {
                        if precision_expr.is_some() {
                            Ok(Value::Float(n as f64))
                        } else {
                            Ok(Value::Int(n))
                        }
                    }
                    Value::Float(n) => {
                        if let Some(p_expr) = precision_expr {
                            let p_val = self.evaluate_expression(p_expr, bindings)?;
                            match p_val {
                                Value::Int(p) => {
                                    let factor = 10f64.powi(p as i32);
                                    Ok(Value::Float((n * factor).round() / factor))
                                }
                                Value::Null => Ok(Value::Null),
                                _ => Err(ExecuteError::TypeError(
                                    "round() precision must be an integer".to_string(),
                                )),
                            }
                        } else {
                            Ok(Value::Int(n.round() as i64))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "round() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Sign(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n.signum())),
                    Value::Float(n) => Ok(Value::Int(n.signum() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "sign() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Rand => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                use std::time::SystemTime;
                let mut hasher = DefaultHasher::new();
                SystemTime::now().hash(&mut hasher);
                std::thread::current().id().hash(&mut hasher);
                let hash = hasher.finish();
                let result = (hash as f64) / (u64::MAX as f64);
                Ok(Value::Float(result))
            }
            ScalarFunction::IsNaN(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Float(n) => Ok(Value::Bool(n.is_nan())),
                    Value::Int(_) => Ok(Value::Bool(false)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "isNaN() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Log(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Float((n as f64).ln())),
                    Value::Float(n) => Ok(Value::Float(n.ln())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "log() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Log10(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Float((n as f64).log10())),
                    Value::Float(n) => Ok(Value::Float(n.log10())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "log10() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Sqrt(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Float((n as f64).sqrt())),
                    Value::Float(n) => Ok(Value::Float(n.sqrt())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "sqrt() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::E => Ok(Value::Float(std::f64::consts::E)),
            ScalarFunction::Pi => Ok(Value::Float(std::f64::consts::PI)),
            ScalarFunction::Id(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(id) => Ok(Value::Int(*id as i64)),
                        BindingValue::Edge(id) => Ok(Value::Int(*id as i64)),
                        _ => Err(ExecuteError::TypeError(
                            "id() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::ElementId(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(id) => Ok(Value::String(format!("node:{}", id))),
                        BindingValue::Edge(id) => Ok(Value::String(format!("edge:{}", id))),
                        _ => Err(ExecuteError::TypeError(
                            "elementId() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Type(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                Ok(Value::String(edge.label.clone()))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "type() requires an edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::StartNode(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let node_id = edge.from;
                                if let Some(node) = self.graph_ref().get_node(node_id) {
                                    Ok(Value::NodeData {
                                        id: node_id,
                                        labels: node.labels.clone(),
                                        properties: Arc::clone(&node.properties),
                                    })
                                } else {
                                    Ok(Value::Node(node_id))
                                }
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "startNode() requires an edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::EndNode(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let node_id = edge.to;
                                if let Some(node) = self.graph_ref().get_node(node_id) {
                                    Ok(Value::NodeData {
                                        id: node_id,
                                        labels: node.labels.clone(),
                                        properties: Arc::clone(&node.properties),
                                    })
                                } else {
                                    Ok(Value::Node(node_id))
                                }
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "endNode() requires an edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Labels(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                let labels: Vec<Value> = node
                                    .labels
                                    .iter()
                                    .map(|l| Value::String(l.clone()))
                                    .collect();
                                Ok(Value::List(labels))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "labels() requires a node variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Properties(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                let props: Vec<Value> = node
                                    .properties
                                    .iter()
                                    .map(|(k, v)| {
                                        Value::List(vec![Value::String(k.clone()), Value::from(v)])
                                    })
                                    .collect();
                                Ok(Value::List(props))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let props: Vec<Value> = edge
                                    .properties
                                    .iter()
                                    .map(|(k, v)| {
                                        Value::List(vec![Value::String(k.clone()), Value::from(v)])
                                    })
                                    .collect();
                                Ok(Value::List(props))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "properties() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Keys(var) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                let keys: Vec<Value> = node
                                    .properties
                                    .keys()
                                    .map(|k| Value::String(k.clone()))
                                    .collect();
                                Ok(Value::List(keys))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let keys: Vec<Value> = edge
                                    .properties
                                    .keys()
                                    .map(|k| Value::String(k.clone()))
                                    .collect();
                                Ok(Value::List(keys))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "keys() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Coalesce(exprs) => {
                for expr in exprs {
                    let val = self.evaluate_expression(expr, bindings)?;
                    if val != Value::Null {
                        return Ok(val);
                    }
                }
                Ok(Value::Null)
            }
            ScalarFunction::NullIf(expr1, expr2) => {
                let val1 = self.evaluate_expression(expr1, bindings)?;
                let val2 = self.evaluate_expression(expr2, bindings)?;
                if val1 == val2 {
                    Ok(Value::Null)
                } else {
                    Ok(val1)
                }
            }
            ScalarFunction::ToBoolean(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Bool(b) => Ok(Value::Bool(b)),
                    Value::String(s) => match s.to_lowercase().as_str() {
                        "true" => Ok(Value::Bool(true)),
                        "false" => Ok(Value::Bool(false)),
                        _ => Ok(Value::Null),
                    },
                    Value::Null => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::ToFloat(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Float(f) => Ok(Value::Float(f)),
                    Value::Int(n) => Ok(Value::Float(n as f64)),
                    Value::String(s) => match s.parse::<f64>() {
                        Ok(f) => Ok(Value::Float(f)),
                        Err(_) => Ok(Value::Null),
                    },
                    Value::Null => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::ToInteger(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n)),
                    Value::Float(f) => Ok(Value::Int(f as i64)),
                    Value::String(s) => match s.parse::<i64>() {
                        Ok(n) => Ok(Value::Int(n)),
                        Err(_) => {
                            // Try parsing as float first, then truncate
                            match s.parse::<f64>() {
                                Ok(f) => Ok(Value::Int(f as i64)),
                                Err(_) => Ok(Value::Null),
                            }
                        }
                    },
                    Value::Null => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::Timestamp => {
                use std::time::SystemTime;
                let millis = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                Ok(Value::Int(millis))
            }
            ScalarFunction::RandomUUID => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                use std::time::SystemTime;
                // Simple UUID v4 generation without external crate
                let mut bytes = [0u8; 16];
                for i in 0..4 {
                    let mut hasher = DefaultHasher::new();
                    SystemTime::now().hash(&mut hasher);
                    std::thread::current().id().hash(&mut hasher);
                    (i as u64).hash(&mut hasher);
                    let hash = hasher.finish();
                    let offset = i * 4;
                    bytes[offset] = (hash >> 56) as u8;
                    bytes[offset + 1] = (hash >> 48) as u8;
                    bytes[offset + 2] = (hash >> 40) as u8;
                    bytes[offset + 3] = (hash >> 32) as u8;
                }
                // Set version 4
                bytes[6] = (bytes[6] & 0x0f) | 0x40;
                // Set variant
                bytes[8] = (bytes[8] & 0x3f) | 0x80;
                let uuid = format!(
                    "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                    bytes[0],
                    bytes[1],
                    bytes[2],
                    bytes[3],
                    bytes[4],
                    bytes[5],
                    bytes[6],
                    bytes[7],
                    bytes[8],
                    bytes[9],
                    bytes[10],
                    bytes[11],
                    bytes[12],
                    bytes[13],
                    bytes[14],
                    bytes[15],
                );
                Ok(Value::String(uuid))
            }
            ScalarFunction::Head(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => Ok(items.into_iter().next().unwrap_or(Value::Null)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "head() requires a list".to_string(),
                    )),
                }
            }
            ScalarFunction::Last(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => Ok(items.into_iter().last().unwrap_or(Value::Null)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "last() requires a list".to_string(),
                    )),
                }
            }
            ScalarFunction::Tail(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => {
                        if items.is_empty() {
                            Ok(Value::List(vec![]))
                        } else {
                            Ok(Value::List(items.into_iter().skip(1).collect()))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "tail() requires a list".to_string(),
                    )),
                }
            }
            ScalarFunction::Range(start_expr, end_expr, step_expr) => {
                let start_val = self.evaluate_expression(start_expr, bindings)?;
                let end_val = self.evaluate_expression(end_expr, bindings)?;
                let step_val = step_expr
                    .as_ref()
                    .map(|e| self.evaluate_expression(e, bindings))
                    .transpose()?;
                match (start_val, end_val) {
                    (Value::Int(start), Value::Int(end)) => {
                        let step = match step_val {
                            Some(Value::Int(s)) => s,
                            None => 1,
                            Some(Value::Null) => return Ok(Value::Null),
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "range() step must be an integer".to_string(),
                                ));
                            }
                        };
                        if step == 0 {
                            return Err(ExecuteError::TypeError(
                                "range() step cannot be zero".to_string(),
                            ));
                        }
                        let mut result = Vec::new();
                        let mut i = start;
                        if step > 0 {
                            while i <= end {
                                result.push(Value::Int(i));
                                i += step;
                            }
                        } else {
                            while i >= end {
                                result.push(Value::Int(i));
                                i += step;
                            }
                        }
                        Ok(Value::List(result))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "range() requires integer arguments".to_string(),
                    )),
                }
            }
            ScalarFunction::Reduce {
                acc_var,
                init,
                item_var,
                list,
                body,
            } => {
                let list_val = self.evaluate_expression(list, bindings)?;
                let init_val = self.evaluate_expression(init, bindings)?;
                let items = match list_val {
                    Value::List(items) => items,
                    Value::Null => return Ok(Value::Null),
                    _ => {
                        return Err(ExecuteError::TypeError(
                            "reduce() requires a list".to_string(),
                        ));
                    }
                };
                let mut acc = init_val;
                for item in items {
                    let mut local_bindings = bindings.clone();
                    local_bindings.insert(acc_var.clone(), BindingValue::Scalar(acc));
                    local_bindings.insert(item_var.clone(), BindingValue::Scalar(item));
                    acc = self.evaluate_expression(body, &local_bindings)?;
                }
                Ok(acc)
            }
            ScalarFunction::DateFunc(arg) => {
                use maharit_core::temporal;
                use std::time::SystemTime;
                match arg {
                    None => {
                        let ms = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64;
                        let days = (ms / 86_400_000) as i32;
                        Ok(Value::Date(days))
                    }
                    Some(expr) => {
                        let val = self.evaluate_expression(expr, bindings)?;
                        match val {
                            Value::String(s) => {
                                match temporal::parse_date(&s) {
                                    Some(d) => Ok(Value::Date(d)),
                                    None => Err(ExecuteError::TypeError(format!("invalid date string: {}", s))),
                                }
                            }
                            Value::Date(d) => Ok(Value::Date(d)),
                            Value::Map(map) => {
                                let get_i32 = |key: &str| -> Option<i32> {
                                    match map.get(key) {
                                        Some(Value::Int(n)) => Some(*n as i32),
                                        _ => None,
                                    }
                                };
                                let year = get_i32("year").unwrap_or(1970);
                                let month = get_i32("month").unwrap_or(1);
                                let day = get_i32("day").unwrap_or(1);
                                let days = temporal::ymd_to_days(year, month, day);
                                Ok(Value::Date(days))
                            }
                            _ => Err(ExecuteError::TypeError("date() requires a string or map argument".to_string())),
                        }
                    }
                }
            }
            ScalarFunction::DateTimeFunc(arg) => {
                use maharit_core::temporal;
                use std::time::SystemTime;
                match arg {
                    None => {
                        let ms = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64;
                        Ok(Value::DateTime(ms))
                    }
                    Some(expr) => {
                        let val = self.evaluate_expression(expr, bindings)?;
                        match val {
                            Value::String(s) => {
                                match temporal::parse_datetime(&s) {
                                    Some(ms) => Ok(Value::DateTime(ms)),
                                    None => Err(ExecuteError::TypeError(format!("invalid datetime string: {}", s))),
                                }
                            }
                            Value::DateTime(ms) => Ok(Value::DateTime(ms)),
                            _ => Err(ExecuteError::TypeError("datetime() requires a string argument".to_string())),
                        }
                    }
                }
            }
            ScalarFunction::DurationFunc(expr) => {
                use maharit_core::temporal;
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => {
                        match temporal::parse_duration(&s) {
                            Some((months, days, millis)) => Ok(Value::Duration { months, days, millis }),
                            None => Err(ExecuteError::TypeError(format!("invalid duration string: {}", s))),
                        }
                    }
                    Value::Map(map) => {
                        let get_i64 = |key: &str| -> i64 {
                            match map.get(key) {
                                Some(Value::Int(n)) => *n,
                                _ => 0,
                            }
                        };
                        let years = get_i64("years");
                        let months_v = get_i64("months");
                        let weeks = get_i64("weeks");
                        let days = get_i64("days");
                        let hours = get_i64("hours");
                        let minutes = get_i64("minutes");
                        let seconds = get_i64("seconds");
                        let milliseconds = get_i64("milliseconds");
                        let total_months = (years * 12 + months_v) as i32;
                        let total_days = (weeks * 7 + days) as i32;
                        let total_millis = hours * 3_600_000
                            + minutes * 60_000
                            + seconds * 1_000
                            + milliseconds;
                        Ok(Value::Duration {
                            months: total_months,
                            days: total_days,
                            millis: total_millis,
                        })
                    }
                    _ => Err(ExecuteError::TypeError("duration() requires a string or map argument".to_string())),
                }
            }
        }
    }

    /// ノードのシーケンスから連続するノード間のエッジIDを抽出する
    fn extract_edge_ids(&self, nodes: &[NodeId]) -> Vec<u64> {
        let mut edges = Vec::new();
        for window in nodes.windows(2) {
            let from = window[0];
            let to = window[1];
            // Find edge from 'from' to 'to'
            for edge in self.graph_ref().outgoing_edges(from) {
                if edge.to == to {
                    edges.push(edge.id);
                    break;
                }
            }
        }
        edges
    }

    fn build_aggregated_result_set(
        &self,
        return_clause: &ReturnClause,
        bindings_list: &[Bindings],
    ) -> Result<ResultSet, ExecuteError> {
        let columns: Vec<String> = return_clause
            .items
            .iter()
            .map(|item| self.return_item_to_column_name(item))
            .collect();

        // Identify group key positions (non-aggregate items)
        let group_key_indices: Vec<usize> = return_clause
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !Self::is_aggregate(item))
            .map(|(i, _)| i)
            .collect();

        if group_key_indices.is_empty() {
            // Simple aggregation: no GROUP BY keys → single row
            let mut row_values = Vec::new();
            for item in &return_clause.items {
                let value = self.evaluate_aggregate(item, bindings_list)?;
                row_values.push(value);
            }
            return Ok(ResultSet::new(columns, vec![Row { columns: row_values }]));
        }

        // GROUP BY: group binding indices by serialized key values
        // Use insertion-order-preserving Vec + HashMap for deterministic output
        let mut group_order: Vec<Vec<String>> = Vec::new();
        let mut group_map: HashMap<Vec<String>, Vec<usize>> = HashMap::new();

        for (idx, bindings) in bindings_list.iter().enumerate() {
            let key: Vec<String> = group_key_indices
                .iter()
                .map(|&col_idx| {
                    self.evaluate_return_item(&return_clause.items[col_idx], bindings)
                        .map(|v| format!("{}", v))
                        .unwrap_or_default()
                })
                .collect();

            if !group_map.contains_key(&key) {
                group_order.push(key.clone());
            }
            group_map.entry(key).or_default().push(idx);
        }

        // Build one result row per group
        let mut rows = Vec::with_capacity(group_order.len());

        for key_strs in &group_order {
            let indices = &group_map[key_strs];

            // Collect this group's bindings
            let group_bindings: Vec<Bindings> = indices
                .iter()
                .map(|&i| bindings_list[i].clone())
                .collect();

            let mut row_values = vec![Value::Null; return_clause.items.len()];

            // Fill group key columns using the first binding of the group
            if let Some(&first_idx) = indices.first() {
                for &col_idx in &group_key_indices {
                    row_values[col_idx] = self.evaluate_return_item(
                        &return_clause.items[col_idx],
                        &bindings_list[first_idx],
                    )?;
                }
            }

            // Fill aggregate columns
            for (col_idx, item) in return_clause.items.iter().enumerate() {
                if Self::is_aggregate(item) {
                    row_values[col_idx] = self.evaluate_aggregate(item, &group_bindings)?;
                }
            }

            rows.push(Row { columns: row_values });
        }

        // Apply ORDER BY / SKIP / LIMIT on the grouped result
        if let Some(ref order_by) = return_clause.order_by {
            let resolved_skip = return_clause
                .skip
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;
            let resolved_limit = return_clause
                .limit
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;

            let needed = match (resolved_skip, resolved_limit) {
                (Some(skip), Some(limit)) => Some((skip + limit) as usize),
                (None, Some(limit)) => Some(limit as usize),
                _ => None,
            };

            if let Some(n) = needed {
                if n < rows.len() {
                    rows = self.apply_order_by_topn(rows, order_by, &columns, n);
                } else {
                    self.apply_order_by(&mut rows, order_by, &columns);
                }
            } else {
                self.apply_order_by(&mut rows, order_by, &columns);
            }

            if let Some(skip) = resolved_skip {
                let skip = skip as usize;
                if skip < rows.len() {
                    rows = rows.into_iter().skip(skip).collect();
                } else {
                    rows.clear();
                }
            }
            if let Some(limit) = resolved_limit {
                rows.truncate(limit as usize);
            }
        } else {
            let resolved_skip = return_clause
                .skip
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;
            let resolved_limit = return_clause
                .limit
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;

            if let Some(skip) = resolved_skip {
                let skip = skip as usize;
                if skip < rows.len() {
                    rows = rows.into_iter().skip(skip).collect();
                } else {
                    rows.clear();
                }
            }
            if let Some(limit) = resolved_limit {
                rows.truncate(limit as usize);
            }
        }

        Ok(ResultSet::new(columns, rows))
    }

    fn evaluate_aggregate(
        &self,
        item: &ReturnItem,
        bindings_list: &[Bindings],
    ) -> Result<Value, ExecuteError> {
        match item {
            ReturnItem::Aggregate(agg) => match agg {
                AggregateFunction::Count(inner) => {
                    if inner.is_none() {
                        // COUNT(*)
                        return Ok(Value::Int(bindings_list.len() as i64));
                    }
                    let inner = inner.as_ref().unwrap();
                    // Fast path: bound Node/Edge variables are always non-null in MATCH results
                    if let ReturnItem::Variable(var) = inner.as_ref() {
                        let is_always_bound = bindings_list
                            .first()
                            .and_then(|b| b.get(var))
                            .map(|bv| matches!(bv, BindingValue::Node(_) | BindingValue::Edge(_)))
                            .unwrap_or(false);
                        if is_always_bound {
                            return Ok(Value::Int(bindings_list.len() as i64));
                        }
                    }
                    // General path: count non-null values
                    let count = bindings_list
                        .iter()
                        .filter(|bindings| {
                            self.evaluate_return_item(inner, bindings)
                                .map(|v| !matches!(v, Value::Null))
                                .unwrap_or(false)
                        })
                        .count();
                    Ok(Value::Int(count as i64))
                }
                AggregateFunction::Sum(inner) => {
                    let mut sum = 0.0;
                    let mut has_float = false;
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => sum += n as f64,
                            Value::Float(n) => {
                                sum += n;
                                has_float = true;
                            }
                            _ => {}
                        }
                    }
                    if has_float {
                        Ok(Value::Float(sum))
                    } else {
                        Ok(Value::Int(sum as i64))
                    }
                }
                AggregateFunction::Avg(inner) => {
                    let mut sum = 0.0;
                    let mut count = 0;
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => {
                                sum += n as f64;
                                count += 1;
                            }
                            Value::Float(n) => {
                                sum += n;
                                count += 1;
                            }
                            _ => {}
                        }
                    }
                    if count == 0 {
                        Ok(Value::Null)
                    } else {
                        Ok(Value::Float(sum / count as f64))
                    }
                }
                AggregateFunction::Min(inner) => {
                    let mut min: Option<Value> = None;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        min = Some(match min {
                            None => val,
                            Some(current) => {
                                if self
                                    .compare_values(&val, &current, |o| o.is_lt())
                                    .map(|v| matches!(v, Value::Bool(true)))
                                    .unwrap_or(false)
                                {
                                    val
                                } else {
                                    current
                                }
                            }
                        });
                    }
                    Ok(min.unwrap_or(Value::Null))
                }
                AggregateFunction::Max(inner) => {
                    let mut max: Option<Value> = None;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        max = Some(match max {
                            None => val,
                            Some(current) => {
                                if self
                                    .compare_values(&val, &current, |o| o.is_gt())
                                    .map(|v| matches!(v, Value::Bool(true)))
                                    .unwrap_or(false)
                                {
                                    val
                                } else {
                                    current
                                }
                            }
                        });
                    }
                    Ok(max.unwrap_or(Value::Null))
                }
                AggregateFunction::Collect(inner) => {
                    let collected: Vec<String> = bindings_list
                        .iter()
                        .filter_map(|bindings| {
                            self.evaluate_return_item(inner, bindings)
                                .ok()
                                .filter(|v| !matches!(v, Value::Null))
                                .map(|v| format!("{}", v))
                        })
                        .collect();
                    Ok(Value::String(format!("[{}]", collected.join(", "))))
                }
                AggregateFunction::PercentileCont(inner, percentile_item) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    if values.is_empty() {
                        return Ok(Value::Null);
                    }
                    let p = match self.evaluate_return_item(
                        percentile_item,
                        bindings_list.first().unwrap_or(&Bindings::new()),
                    )? {
                        Value::Float(f) => f,
                        Value::Int(n) => n as f64,
                        _ => {
                            return Err(ExecuteError::TypeError(
                                "percentileCont() percentile must be a number between 0 and 1"
                                    .to_string(),
                            ));
                        }
                    };
                    if !(0.0..=1.0).contains(&p) {
                        return Err(ExecuteError::TypeError(
                            "percentileCont() percentile must be between 0 and 1".to_string(),
                        ));
                    }
                    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let idx = p * (values.len() - 1) as f64;
                    let lower = idx.floor() as usize;
                    let upper = idx.ceil() as usize;
                    if lower == upper || upper >= values.len() {
                        Ok(Value::Float(values[lower]))
                    } else {
                        let frac = idx - lower as f64;
                        Ok(Value::Float(
                            values[lower] + (values[upper] - values[lower]) * frac,
                        ))
                    }
                }
                AggregateFunction::PercentileDisc(inner, percentile_item) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    if values.is_empty() {
                        return Ok(Value::Null);
                    }
                    let p = match self.evaluate_return_item(
                        percentile_item,
                        bindings_list.first().unwrap_or(&Bindings::new()),
                    )? {
                        Value::Float(f) => f,
                        Value::Int(n) => n as f64,
                        _ => {
                            return Err(ExecuteError::TypeError(
                                "percentileDisc() percentile must be a number between 0 and 1"
                                    .to_string(),
                            ));
                        }
                    };
                    if !(0.0..=1.0).contains(&p) {
                        return Err(ExecuteError::TypeError(
                            "percentileDisc() percentile must be between 0 and 1".to_string(),
                        ));
                    }
                    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let idx = if p == 0.0 {
                        0
                    } else {
                        ((p * values.len() as f64).ceil() as usize)
                            .saturating_sub(1)
                            .min(values.len() - 1)
                    };
                    let val = values[idx];
                    if val.fract() == 0.0 && val.abs() < i64::MAX as f64 {
                        Ok(Value::Int(val as i64))
                    } else {
                        Ok(Value::Float(val))
                    }
                }
                AggregateFunction::StDev(inner) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    let n = values.len();
                    if n < 2 {
                        return Ok(Value::Float(0.0));
                    }
                    let mean = values.iter().sum::<f64>() / n as f64;
                    let variance =
                        values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
                    Ok(Value::Float(variance.sqrt()))
                }
                AggregateFunction::StDevP(inner) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    let n = values.len();
                    if n == 0 {
                        return Ok(Value::Float(0.0));
                    }
                    let mean = values.iter().sum::<f64>() / n as f64;
                    let variance =
                        values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n as f64;
                    Ok(Value::Float(variance.sqrt()))
                }
                AggregateFunction::CountDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut count = 0i64;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            count += 1;
                        }
                    }
                    Ok(Value::Int(count))
                }
                AggregateFunction::SumDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut sum = 0.0f64;
                    let mut has_float = false;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            match val {
                                Value::Int(n) => sum += n as f64,
                                Value::Float(n) => {
                                    sum += n;
                                    has_float = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    if has_float {
                        Ok(Value::Float(sum))
                    } else {
                        Ok(Value::Int(sum as i64))
                    }
                }
                AggregateFunction::AvgDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut sum = 0.0f64;
                    let mut count = 0usize;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            match val {
                                Value::Int(n) => {
                                    sum += n as f64;
                                    count += 1;
                                }
                                Value::Float(n) => {
                                    sum += n;
                                    count += 1;
                                }
                                _ => {}
                            }
                        }
                    }
                    if count == 0 {
                        Ok(Value::Null)
                    } else {
                        Ok(Value::Float(sum / count as f64))
                    }
                }
                AggregateFunction::CollectDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut result = Vec::new();
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            result.push(val);
                        }
                    }
                    Ok(Value::List(result))
                }
            },
            // Alias wrapping an aggregate: delegate to the inner aggregate
            ReturnItem::Alias(inner, _) => self.evaluate_aggregate(inner, bindings_list),
            // Non-aggregate items in an aggregated query just use the first binding
            _ => {
                if let Some(bindings) = bindings_list.first() {
                    self.evaluate_return_item(item, bindings)
                } else {
                    Ok(Value::Null)
                }
            }
        }
    }

    /// SKIP / LIMIT の式を評価して非負整数に変換するヘルパー。
    /// 整数リテラルまたはパラメータ参照に対応する。
    fn resolve_skip_limit(
        &self,
        expr: &Expression,
    ) -> Result<u64, ExecuteError> {
        let val = self.evaluate_expression(expr, &Bindings::new())?;
        match val {
            Value::Int(n) if n >= 0 => Ok(n as u64),
            Value::Int(n) => Err(ExecuteError::TypeError(format!(
                "SKIP/LIMIT value must be a non-negative integer, got {}",
                n
            ))),
            other => Err(ExecuteError::TypeError(format!(
                "SKIP/LIMIT value must be an integer, got {:?}",
                other
            ))),
        }
    }

    /// テンポラル値（Date / DateTime / Duration）のフィールドアクセスを行う。
    ///
    /// `d.year`, `d.month`, `d.day`, `d.hour`, `d.minute`, `d.second`,
    /// `dur.years`, `dur.months`, `dur.days`, `dur.hours`, `dur.minutes`, `dur.seconds`
    /// に対応する。一致するフィールドがない場合は `Value::Null` を返す。
    fn access_temporal_field(val: &Value, field: &str) -> Result<Value, ExecuteError> {
        use maharit_core::temporal;
        match val {
            Value::Date(days) => {
                let (y, m, d) = temporal::days_to_ymd(*days);
                let v = match field {
                    "year" => Value::Int(y as i64),
                    "month" => Value::Int(m as i64),
                    "day" => Value::Int(d as i64),
                    _ => Value::Null,
                };
                Ok(v)
            }
            Value::DateTime(ms) => {
                let (y, mo, d, h, mi, s, _frac) = temporal::millis_to_datetime(*ms);
                let v = match field {
                    "year" => Value::Int(y as i64),
                    "month" => Value::Int(mo as i64),
                    "day" => Value::Int(d as i64),
                    "hour" => Value::Int(h as i64),
                    "minute" => Value::Int(mi as i64),
                    "second" => Value::Int(s as i64),
                    _ => Value::Null,
                };
                Ok(v)
            }
            Value::Duration { months, days, millis } => {
                let v = match field {
                    "years" => Value::Int((*months / 12) as i64),
                    "months" => Value::Int((*months % 12) as i64),
                    "days" => Value::Int(*days as i64),
                    "hours" => Value::Int(*millis / 3_600_000),
                    "minutes" => Value::Int(*millis % 3_600_000 / 60_000),
                    "seconds" => Value::Int(*millis % 60_000 / 1_000),
                    "milliseconds" => Value::Int(*millis % 1_000),
                    _ => Value::Null,
                };
                Ok(v)
            }
            _ => Ok(Value::Null),
        }
    }

    fn evaluate_expression(
        &self,
        expr: &Expression,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        match expr {
            Expression::Literal(lit) => Ok(Value::from(lit.clone())),
            Expression::Variable(var) => {
                let binding_value = bindings
                    .get(var)
                    .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;
                match binding_value {
                    BindingValue::Node(id) => Ok(Value::Node(*id)),
                    BindingValue::Edge(id) => Ok(Value::Int(*id as i64)),
                    BindingValue::Path { nodes, edges } => Ok(Value::Path {
                        nodes: nodes.clone(),
                        edges: edges.clone(),
                    }),
                    BindingValue::Scalar(value) => Ok(value.clone()),
                }
            }
            Expression::Property(var, prop) => {
                let binding_value = bindings
                    .get(var)
                    .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;

                match binding_value {
                    BindingValue::Node(node_id) => {
                        let node = self
                            .graph_ref()
                            .get_node(*node_id)
                            .ok_or_else(|| ExecuteError::TypeError("node not found".to_string()))?;
                        Ok(node.get_property(prop).map(Value::from).unwrap_or(Value::Null))
                    }
                    BindingValue::Edge(edge_id) => {
                        let edge = self
                            .graph_ref()
                            .get_edge(*edge_id)
                            .ok_or_else(|| ExecuteError::TypeError("edge not found".to_string()))?;
                        Ok(edge.get_property(prop).map(Value::from).unwrap_or(Value::Null))
                    }
                    BindingValue::Scalar(scalar_val) => {
                        if let Value::Map(map) = scalar_val {
                            Ok(map.get(prop).cloned().unwrap_or(Value::Null))
                        } else {
                            Self::access_temporal_field(scalar_val, prop)
                        }
                    }
                    _ => Err(ExecuteError::TypeError(format!(
                        "cannot access property '{}' on path value",
                        prop
                    ))),
                }
            }
            Expression::BinaryOp(left, op, right) => {
                let left_val = self.evaluate_expression(left, bindings)?;
                let right_val = self.evaluate_expression(right, bindings)?;
                self.apply_binary_op(&left_val, *op, &right_val)
            }
            Expression::UnaryOp(op, expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                self.apply_unary_op(*op, &val)
            }
            Expression::Case(case_expr) => self.evaluate_case_expression(case_expr, bindings),
            Expression::List(elements) => {
                let values: Vec<Value> = elements
                    .iter()
                    .map(|e| self.evaluate_expression(e, bindings))
                    .collect::<Result<_, _>>()?;
                Ok(Value::List(values))
            }
            Expression::IndexAccess(list_expr, index_expr) => {
                let list_val = self.evaluate_expression(list_expr, bindings)?;
                let idx_val = self.evaluate_expression(index_expr, bindings)?;
                match (list_val, idx_val) {
                    (Value::List(items), Value::Int(i)) => {
                        let idx = if i < 0 { items.len() as i64 + i } else { i };
                        if idx >= 0 {
                            Ok(items.get(idx as usize).cloned().unwrap_or(Value::Null))
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            Expression::ListSlice(list_expr, start_expr, end_expr) => {
                let list_val = self.evaluate_expression(list_expr, bindings)?;
                let start_val = self.evaluate_expression(start_expr, bindings)?;
                let end_val = self.evaluate_expression(end_expr, bindings)?;
                match (list_val, start_val, end_val) {
                    (Value::List(items), Value::Int(s), Value::Int(e)) => {
                        let len = items.len() as i64;
                        let s = if s < 0 { (len + s).max(0) } else { s.min(len) } as usize;
                        let e = if e < 0 { (len + e).max(0) } else { e.min(len) } as usize;
                        Ok(Value::List(items[s.min(e)..e.max(s)].to_vec()))
                    }
                    (Value::Null, _, _) => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            Expression::ListComprehension {
                variable,
                list,
                predicate,
                result,
            } => {
                let list_val = self.evaluate_expression(list, bindings)?;
                let items = match list_val {
                    Value::List(items) => items,
                    Value::Null => return Ok(Value::Null),
                    _ => {
                        return Err(ExecuteError::TypeError(
                            "list comprehension requires a list".to_string(),
                        ));
                    }
                };
                let mut output = Vec::new();
                for item in items {
                    let mut local_bindings = bindings.clone();
                    local_bindings.insert(variable.clone(), BindingValue::Scalar(item));
                    // Apply predicate filter
                    if let Some(pred) = predicate {
                        let pred_val = self.evaluate_expression(pred, &local_bindings)?;
                        if !matches!(pred_val, Value::Bool(true)) {
                            continue;
                        }
                    }
                    // Evaluate result expression
                    let result_val = self.evaluate_expression(result, &local_bindings)?;
                    output.push(result_val);
                }
                Ok(Value::List(output))
            }
            Expression::Parameter(name) => self
                .params
                .get(name)
                .cloned()
                .ok_or_else(|| ExecuteError::UndefinedVariable(format!("${}", name))),
            Expression::ExistsSubquery(subquery) => {
                // Evaluate patterns starting from current bindings
                let mut matches = vec![bindings.clone()];
                for pattern in &subquery.patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                // Apply WHERE filter
                if let Some(ref where_expr) = subquery.where_clause {
                    matches.retain(|b| {
                        self.evaluate_expression(where_expr, b)
                            .map(|v| matches!(v, Value::Bool(true)))
                            .unwrap_or(false)
                    });
                }
                Ok(Value::Bool(!matches.is_empty()))
            }
            Expression::CountSubquery(subquery) => {
                // Evaluate patterns starting from current bindings
                let mut matches = vec![bindings.clone()];
                for pattern in &subquery.patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                // Apply WHERE filter
                if let Some(ref where_expr) = subquery.where_clause {
                    matches.retain(|b| {
                        self.evaluate_expression(where_expr, b)
                            .map(|v| matches!(v, Value::Bool(true)))
                            .unwrap_or(false)
                    });
                }
                Ok(Value::Int(matches.len() as i64))
            }
            Expression::CollectSubquery(body) => {
                // Evaluate patterns starting from current bindings
                let mut matches = vec![bindings.clone()];
                for pattern in &body.patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                // Apply WHERE filter
                if let Some(ref where_expr) = body.where_clause {
                    matches.retain(|b| {
                        self.evaluate_expression(where_expr, b)
                            .map(|v| matches!(v, Value::Bool(true)))
                            .unwrap_or(false)
                    });
                }
                // Evaluate return item for each match
                let values: Vec<Value> = matches
                    .iter()
                    .map(|b| self.evaluate_return_item(&body.return_item, b))
                    .collect::<Result<_, _>>()?;
                Ok(Value::List(values))
            }
            Expression::ListPredicate {
                kind,
                variable,
                list,
                predicate,
            } => {
                let list_val = self.evaluate_expression(list, bindings)?;
                let items = match list_val {
                    Value::List(items) => items,
                    Value::Null => return Ok(Value::Null),
                    _ => {
                        return Err(ExecuteError::TypeError(
                            "list predicate requires a list".to_string(),
                        ));
                    }
                };
                let mut count = 0usize;
                for item in &items {
                    let mut local_bindings = bindings.clone();
                    local_bindings.insert(variable.clone(), BindingValue::Scalar(item.clone()));
                    let pred_val = self.evaluate_expression(predicate, &local_bindings)?;
                    if matches!(pred_val, Value::Bool(true)) {
                        count += 1;
                    }
                }
                let result = match kind {
                    ListPredicateKind::All => count == items.len(),
                    ListPredicateKind::Any => count > 0,
                    ListPredicateKind::None => count == 0,
                    ListPredicateKind::Single => count == 1,
                };
                Ok(Value::Bool(result))
            }
            Expression::Exists(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                Ok(Value::Bool(!matches!(val, Value::Null)))
            }
            Expression::IsEmpty(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => Ok(Value::Bool(items.is_empty())),
                    Value::String(s) => Ok(Value::Bool(s.is_empty())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "isEmpty() requires a list or string".to_string(),
                    )),
                }
            }
            Expression::PatternPredicate(patterns) => {
                // Evaluate patterns against current bindings; return true if any match exists.
                // After matching, verify that variables already bound in the outer scope still
                // hold the same value (pattern matching may overwrite them with different nodes).
                let mut matches = vec![bindings.clone()];
                for pattern in patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                matches.retain(|new_b| {
                    bindings
                        .iter()
                        .all(|(k, v)| new_b.get(k).is_none_or(|nv| nv == v))
                });
                Ok(Value::Bool(!matches.is_empty()))
            }
            Expression::Map(map) => {
                let mut result = std::collections::HashMap::new();
                for (k, v) in map {
                    result.insert(k.clone(), self.evaluate_expression(v, bindings)?);
                }
                Ok(Value::Map(result))
            }
            Expression::ScalarFn(func) => self.evaluate_scalar_function(func, bindings),
        }
    }

    fn evaluate_case_expression(
        &self,
        case_expr: &CaseExpression,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        // Evaluate operand for simple CASE
        let operand_value = case_expr
            .operand
            .as_ref()
            .map(|op| self.evaluate_expression(op, bindings))
            .transpose()?;

        // Check each WHEN clause
        for when_clause in &case_expr.when_clauses {
            let condition_value = self.evaluate_expression(&when_clause.condition, bindings)?;

            let matches = if let Some(ref op_val) = operand_value {
                // Simple CASE: compare operand with condition value
                self.values_equal(op_val, &condition_value)
            } else {
                // Searched CASE: condition should evaluate to boolean
                matches!(condition_value, Value::Bool(true))
            };

            if matches {
                return self.evaluate_expression(&when_clause.result, bindings);
            }
        }

        // No WHEN clause matched, return ELSE or NULL
        if let Some(ref else_expr) = case_expr.else_clause {
            self.evaluate_expression(else_expr, bindings)
        } else {
            Ok(Value::Null)
        }
    }

    fn apply_binary_op(
        &self,
        left: &Value,
        op: BinaryOp,
        right: &Value,
    ) -> Result<Value, ExecuteError> {
        match op {
            BinaryOp::Eq => Ok(Value::Bool(self.values_equal(left, right))),
            BinaryOp::Neq => Ok(Value::Bool(!self.values_equal(left, right))),
            BinaryOp::Lt => self.compare_values(left, right, |ord| ord.is_lt()),
            BinaryOp::Gt => self.compare_values(left, right, |ord| ord.is_gt()),
            BinaryOp::Lte => self.compare_values(left, right, |ord| ord.is_le()),
            BinaryOp::Gte => self.compare_values(left, right, |ord| ord.is_ge()),
            BinaryOp::And => match (left, right) {
                (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a && *b)),
                _ => Err(ExecuteError::TypeError("AND requires booleans".to_string())),
            },
            BinaryOp::Or => match (left, right) {
                (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a || *b)),
                _ => Err(ExecuteError::TypeError("OR requires booleans".to_string())),
            },
            BinaryOp::Add => {
                if let (Value::List(a), Value::List(b)) = (left, right) {
                    let mut result = a.clone();
                    result.extend(b.iter().cloned());
                    return Ok(Value::List(result));
                }
                // テンポラル演算: Date + Duration, DateTime + Duration
                match (left, right) {
                    (Value::Date(d), Value::Duration { months, days, millis }) => {
                        use maharit_core::temporal;
                        let new_days = temporal::add_duration_to_date(*d, *months, *days, *millis);
                        return Ok(Value::Date(new_days));
                    }
                    (Value::DateTime(ms), Value::Duration { months, days, millis }) => {
                        use maharit_core::temporal;
                        let (y, mo, day, h, mi, s, frac) = temporal::millis_to_datetime(*ms);
                        let base_days = temporal::ymd_to_days(y, mo, day);
                        let new_days = temporal::add_duration_to_date(base_days, *months, *days, 0);
                        let (ny, nmo, nd) = temporal::days_to_ymd(new_days);
                        let new_ms = temporal::datetime_to_millis(ny, nmo, nd, h, mi, s, frac) + millis;
                        return Ok(Value::DateTime(new_ms));
                    }
                    _ => {}
                }
                self.arithmetic_op(left, right, |a, b| a + b, |a, b| a + b)
            }
            BinaryOp::Sub => {
                // テンポラル演算: Date - Date = Duration, DateTime - DateTime = Duration
                match (left, right) {
                    (Value::Date(a), Value::Date(b)) => {
                        let diff_days = a - b;
                        return Ok(Value::Duration { months: 0, days: diff_days, millis: 0 });
                    }
                    (Value::DateTime(a), Value::DateTime(b)) => {
                        let diff_ms = a - b;
                        return Ok(Value::Duration { months: 0, days: 0, millis: diff_ms });
                    }
                    (Value::Date(d), Value::Duration { months, days, millis }) => {
                        use maharit_core::temporal;
                        let new_days = temporal::add_duration_to_date(*d, -*months, -*days, -*millis);
                        return Ok(Value::Date(new_days));
                    }
                    (Value::DateTime(ms), Value::Duration { months, days, millis }) => {
                        use maharit_core::temporal;
                        let (y, mo, day, h, mi, s, frac) = temporal::millis_to_datetime(*ms);
                        let base_days = temporal::ymd_to_days(y, mo, day);
                        let new_days = temporal::add_duration_to_date(base_days, -*months, -*days, 0);
                        let (ny, nmo, nd) = temporal::days_to_ymd(new_days);
                        let new_ms = temporal::datetime_to_millis(ny, nmo, nd, h, mi, s, frac) - millis;
                        return Ok(Value::DateTime(new_ms));
                    }
                    _ => {}
                }
                self.arithmetic_op(left, right, |a, b| a - b, |a, b| a - b)
            }
            BinaryOp::Mul => self.arithmetic_op(left, right, |a, b| a * b, |a, b| a * b),
            BinaryOp::Div => self.arithmetic_op(left, right, |a, b| a / b, |a, b| a / b),
            BinaryOp::Regex => match (left, right) {
                (Value::String(s), Value::String(pattern)) => {
                    // Cypher =~ is full-match, so anchor the pattern
                    let anchored = format!("(?s)\\A(?:{})\\z", pattern);
                    let re = Regex::new(&anchored)
                        .map_err(|e| ExecuteError::TypeError(format!("invalid regex: {}", e)))?;
                    Ok(Value::Bool(re.is_match(s)))
                }
                _ => Err(ExecuteError::TypeError(
                    "=~ requires string operands".to_string(),
                )),
            },
            BinaryOp::Contains => match (left, right) {
                (Value::String(haystack), Value::String(needle)) => {
                    let result = self.string_contains(haystack, needle);
                    Ok(Value::Bool(result))
                }
                _ => Err(ExecuteError::TypeError(
                    "CONTAINS requires string operands".to_string(),
                )),
            },
            BinaryOp::StartsWith => match (left, right) {
                (Value::String(s), Value::String(prefix)) => Ok(Value::Bool(
                    s.to_lowercase().starts_with(&prefix.to_lowercase()),
                )),
                _ => Err(ExecuteError::TypeError(
                    "STARTS WITH requires string operands".to_string(),
                )),
            },
            BinaryOp::EndsWith => match (left, right) {
                (Value::String(s), Value::String(suffix)) => Ok(Value::Bool(
                    s.to_lowercase().ends_with(&suffix.to_lowercase()),
                )),
                _ => Err(ExecuteError::TypeError(
                    "ENDS WITH requires string operands".to_string(),
                )),
            },
            BinaryOp::In => match right {
                Value::List(items) => Ok(Value::Bool(
                    items.iter().any(|item| self.values_equal(left, item)),
                )),
                Value::Null => Ok(Value::Null),
                _ => Ok(Value::Bool(false)),
            },
        }
    }

    fn values_equal(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                ((*a as f64) - b).abs() < f64::EPSILON
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Node(a), Value::Node(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::DateTime(a), Value::DateTime(b)) => a == b,
            (Value::Duration { months: ma, days: da, millis: msa },
             Value::Duration { months: mb, days: db, millis: msb }) => {
                ma == mb && da == db && msa == msb
            }
            _ => false,
        }
    }

    fn compare_values<F>(&self, left: &Value, right: &Value, pred: F) -> Result<Value, ExecuteError>
    where
        F: Fn(std::cmp::Ordering) -> bool,
    {
        let ordering = match (left, right) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Value::Int(a), Value::Float(b)) => (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::Float(a), Value::Int(b)) => a
                .partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Date(a), Value::Date(b)) => a.cmp(b),
            (Value::DateTime(a), Value::DateTime(b)) => a.cmp(b),
            _ => return Err(ExecuteError::TypeError("cannot compare values".to_string())),
        };

        Ok(Value::Bool(pred(ordering)))
    }

    fn arithmetic_op<F, G>(
        &self,
        left: &Value,
        right: &Value,
        int_op: F,
        float_op: G,
    ) -> Result<Value, ExecuteError>
    where
        F: Fn(i64, i64) -> i64,
        G: Fn(f64, f64) -> f64,
    {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(int_op(*a, *b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(*a, *b))),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(float_op(*a as f64, *b))),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(float_op(*a, *b as f64))),
            _ => Err(ExecuteError::TypeError(
                "arithmetic requires numbers".to_string(),
            )),
        }
    }

    fn apply_unary_op(&self, op: UnaryOp, val: &Value) -> Result<Value, ExecuteError> {
        match op {
            UnaryOp::Not => match val {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                _ => Err(ExecuteError::TypeError("NOT requires boolean".to_string())),
            },
            UnaryOp::Neg => match val {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(n) => Ok(Value::Float(-n)),
                _ => Err(ExecuteError::TypeError(
                    "negation requires number".to_string(),
                )),
            },
            UnaryOp::IsNormalized => match val {
                Value::String(s) => {
                    use unicode_normalization::UnicodeNormalization;
                    let nfc: String = s.nfc().collect();
                    Ok(Value::Bool(*s == nfc))
                }
                _ => Err(ExecuteError::TypeError(
                    "IS NORMALIZED requires string operand".to_string(),
                )),
            },
        }
    }

    /// Evaluate the CONTAINS operator with support for phrase and fuzzy search syntax.
    ///
    /// Query formats:
    /// - `"phrase terms"` — phrase search: all tokens must appear consecutively in haystack
    /// - `term~` or `term~N` — fuzzy search: any token in haystack within edit distance N
    /// - plain text — standard case-insensitive substring containment
    fn string_contains(&self, haystack: &str, needle: &str) -> bool {
        // Phrase search: needle surrounded by double quotes
        if needle.len() >= 2 {
            let nb = needle.as_bytes();
            if nb[0] == b'"' && nb[needle.len() - 1] == b'"' {
                let phrase = &needle[1..needle.len() - 1];
                return self.contains_phrase(haystack, phrase);
            }
        }

        // Fuzzy search: needle ends with '~' (optionally followed by digit)
        if let Some(tilde_pos) = needle.rfind('~') {
            let term = &needle[..tilde_pos];
            let suffix = &needle[tilde_pos + 1..];
            if !term.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
                let max_distance: usize = if suffix.is_empty() {
                    2
                } else {
                    suffix.parse().unwrap_or(2)
                };
                return self.contains_fuzzy(haystack, term, max_distance);
            }
        }

        // Standard case-insensitive substring search
        haystack.to_lowercase().contains(&needle.to_lowercase())
    }

    /// Check whether all tokens of `phrase` appear consecutively in `haystack`.
    fn contains_phrase(&self, haystack: &str, phrase: &str) -> bool {
        let haystack_tokens: Vec<&str> = haystack
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        let phrase_tokens: Vec<String> = phrase
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect();

        if phrase_tokens.is_empty() {
            return false;
        }
        if phrase_tokens.len() > haystack_tokens.len() {
            return false;
        }

        'outer: for i in 0..=(haystack_tokens.len() - phrase_tokens.len()) {
            for (j, phrase_token) in phrase_tokens.iter().enumerate() {
                if haystack_tokens[i + j].to_lowercase() != *phrase_token {
                    continue 'outer;
                }
            }
            return true;
        }

        false
    }

    /// Check whether any token in `haystack` is within `max_distance` Levenshtein
    /// distance from `term`.
    fn contains_fuzzy(&self, haystack: &str, term: &str, max_distance: usize) -> bool {
        let term_lower = term.to_lowercase();
        haystack
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .any(|token| levenshtein_distance(token, &term_lower) <= max_distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn execute(graph: &mut Graph, query: &str) -> Result<ResultSet, ExecuteError> {
        let stmt = Parser::new(query).unwrap().parse().unwrap();
        Executor::new(graph).execute(stmt)
    }

    fn execute_with(executor: &mut Executor<'_>, query: &str) -> Result<ResultSet, ExecuteError> {
        let stmt = Parser::new(query).unwrap().parse().unwrap();
        executor.execute(stmt)
    }

    // ────────────────────────────────────────────────────────────────
    // Value::to_json (型情報を保ったまま JSON 表現に変換)
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn value_to_json_preserves_primitive_types() {
        assert_eq!(Value::Null.to_json(), serde_json::Value::Null);
        assert_eq!(Value::Bool(true).to_json(), serde_json::Value::Bool(true));
        assert_eq!(
            Value::Int(30).to_json(),
            serde_json::Value::Number(30.into())
        );
        assert_eq!(
            Value::String("alice".to_string()).to_json(),
            serde_json::Value::String("alice".to_string())
        );
    }

    #[test]
    fn value_to_json_float_preserved() {
        let v = Value::Float(3.14).to_json();
        assert_eq!(v.as_f64(), Some(3.14));
    }

    #[test]
    fn value_to_json_list_is_array() {
        let v = Value::List(vec![
            Value::Int(1),
            Value::String("two".to_string()),
            Value::Null,
        ])
        .to_json();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0], serde_json::json!(1));
        assert_eq!(arr[1], serde_json::Value::String("two".to_string()));
        assert_eq!(arr[2], serde_json::Value::Null);
    }

    #[test]
    fn value_to_json_map_is_object() {
        let mut map = std::collections::HashMap::new();
        map.insert("name".to_string(), Value::String("Alice".to_string()));
        map.insert("age".to_string(), Value::Int(30));
        let v = Value::Map(map).to_json();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.get("name"), Some(&serde_json::json!("Alice")));
        assert_eq!(obj.get("age"), Some(&serde_json::json!(30)));
    }

    #[test]
    fn value_to_string_unquoted_skips_outer_quotes() {
        assert_eq!(
            Value::String("alice".to_string()).to_string_unquoted(),
            "alice"
        );
        assert_eq!(Value::Int(30).to_string_unquoted(), "30");
        assert_eq!(Value::Null.to_string_unquoted(), "null");
    }

    #[test]
    fn test_create_node() {
        let mut graph = Graph::new();
        let result = execute(&mut graph, "CREATE (n:Person)").unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(graph.node_count(), 1);

        let node = graph.nodes().next().unwrap();
        assert!(node.has_label("Person"));
    }

    #[test]
    fn test_create_node_with_properties() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        let node = graph.nodes().next().unwrap();
        assert_eq!(
            node.get_property("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(30)));
    }

    #[test]
    fn test_create_path() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (a:Person)-[:KNOWS]->(b:Person)").unwrap();

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);

        let edge = graph.edges().next().unwrap();
        assert_eq!(edge.label, "KNOWS");
    }

    #[test]
    fn test_match_all_nodes() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_match_with_where() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) WHERE n.age > 28 RETURN n").unwrap();

        assert_eq!(result.row_count(), 1);
    }

    #[test]
    fn test_match_return_property() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN n.name").unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_match_path() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name",
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
    }

    #[test]
    fn test_count_star() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN COUNT(*)").unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(3));
    }

    #[test]
    fn test_count_variable() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN COUNT(n)").unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(2));
    }

    #[test]
    fn test_sum() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN SUM(n.age)").unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(90));
    }

    #[test]
    fn test_avg() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN AVG(n.age)").unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Float(30.0));
    }

    #[test]
    fn test_min_max() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();

        let min_result = execute(&mut graph, "MATCH (n:Person) RETURN MIN(n.age)").unwrap();
        let max_result = execute(&mut graph, "MATCH (n:Person) RETURN MAX(n.age)").unwrap();

        assert_eq!(min_result.rows[0].columns[0], Value::Int(25));
        assert_eq!(max_result.rows[0].columns[0], Value::Int(35));
    }

    #[test]
    fn test_variable_length_path() {
        let mut graph = Graph::new();
        // Create a chain: Alice -> Bob -> Charlie -> David
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // Get Alice and Bob IDs
        let _alice_id = graph
            .nodes()
            .find(|n| n.properties.get("name") == Some(&PropertyValue::String("Alice".to_string())))
            .unwrap()
            .id;
        let bob_id = graph
            .nodes()
            .find(|n| n.properties.get("name") == Some(&PropertyValue::String("Bob".to_string())))
            .unwrap()
            .id;

        // Create Charlie and David
        let charlie_id = graph.create_node("Person");
        graph
            .get_node_mut(charlie_id)
            .unwrap()
            .set_property("name", PropertyValue::String("Charlie".to_string()));
        graph.create_edge(bob_id, charlie_id, "KNOWS").unwrap();

        let david_id = graph.create_node("Person");
        graph
            .get_node_mut(david_id)
            .unwrap()
            .set_property("name", PropertyValue::String("David".to_string()));
        graph.create_edge(charlie_id, david_id, "KNOWS").unwrap();

        // Test: Find all people reachable from Alice in 2 hops
        let result = execute(
            &mut graph,
            "MATCH (a:Person {name: \"Alice\"})-[:KNOWS*2]->(b:Person) RETURN b.name",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Charlie".to_string())
        );

        // Test: Find all people reachable from Alice in 1 to 3 hops
        let result = execute(
            &mut graph,
            "MATCH (a:Person {name: \"Alice\"})-[:KNOWS*1..3]->(b:Person) RETURN b.name",
        )
        .unwrap();
        assert_eq!(result.row_count(), 3); // Bob, Charlie, David
    }

    #[test]
    fn test_variable_length_path_range() {
        let mut graph = Graph::new();
        // Create: A -> B -> C
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        // *2..3 should find C (2 hops) but not B (1 hop)
        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"})-[:NEXT*2..3]->(b:Node) RETURN b.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::String("C".to_string()));
    }

    // ========== Path function tests ==========

    #[test]
    fn test_path_length_function() {
        let mut graph = Graph::new();
        // Create: A -> B -> C
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        // Test length() function - path from A to C has 2 edges
        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"})-[r:NEXT*1..2]->(b:Node) RETURN b.name, length(r)"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 2);

        // Find the row for C (2 hops)
        let c_row = result
            .rows
            .iter()
            .find(|row| row.columns[0] == Value::String("C".to_string()))
            .expect("Should find C");
        assert_eq!(c_row.columns[1], Value::Int(2));

        // Find the row for B (1 hop)
        let b_row = result
            .rows
            .iter()
            .find(|row| row.columns[0] == Value::String("B".to_string()))
            .expect("Should find B");
        assert_eq!(b_row.columns[1], Value::Int(1));
    }

    #[test]
    fn test_path_nodes_function() {
        let mut graph = Graph::new();
        // Create: A -> B -> C
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        // Test nodes() function - returns list of nodes in path
        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"})-[r:NEXT*2]->(b:Node) RETURN nodes(r)"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);

        // Should be a list with 3 nodes (A, B, C)
        if let Value::List(nodes) = &result.rows[0].columns[0] {
            assert_eq!(nodes.len(), 3);
        } else {
            panic!("Expected List value");
        }
    }

    #[test]
    fn test_path_relationships_function() {
        let mut graph = Graph::new();
        // Create: A -> B -> C
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        // Test relationships() function - returns list of edges in path
        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"})-[r:NEXT*2]->(b:Node) RETURN relationships(r)"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);

        // Should be a list with 2 edges
        if let Value::List(edges) = &result.rows[0].columns[0] {
            assert_eq!(edges.len(), 2);
        } else {
            panic!("Expected List value");
        }
    }

    #[test]
    fn test_variable_length_path_star_only() {
        let mut graph = Graph::new();
        // Create: A -> B -> C
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        // [*] means 1..unlimited, should find B and C (1 and 2 hops)
        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"})-[:NEXT*]->(b:Node) RETURN b.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 2); // B (1 hop) and C (2 hops)
    }

    #[test]
    fn test_variable_length_path_zero_hop() {
        let mut graph = Graph::new();
        // Create: A -> B -> C
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        // [*0..2] means 0, 1, or 2 hops
        // 0 hops: A itself (if it matches the target pattern)
        // 1 hop: B
        // 2 hops: C
        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"})-[:NEXT*0..2]->(b:Node) RETURN b.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 3); // A (0 hops), B (1 hop), C (2 hops)
    }

    // ========== ORDER BY tests ==========

    #[test]
    fn test_order_by_asc() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string()));
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(
            result.rows[2].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    #[test]
    fn test_order_by_desc() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Charlie".to_string())
        );
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string()));
    }

    #[test]
    fn test_order_by_string() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name ORDER BY n.name ASC",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
        assert_eq!(
            result.rows[2].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    // ========== LIMIT tests ==========

    #[test]
    fn test_limit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN n.name LIMIT 2").unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_limit_larger_than_result() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN n.name LIMIT 10").unwrap();

        assert_eq!(result.row_count(), 2);
    }

    // ========== SKIP tests ==========

    #[test]
    fn test_skip() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age SKIP 1",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    #[test]
    fn test_skip_and_limit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "David", age: 40})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age SKIP 1 LIMIT 2",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    // ========== DISTINCT tests ==========

    #[test]
    fn test_distinct() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice", city: "Tokyo"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Bob", city: "Tokyo"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", city: "Osaka"})"#,
        )
        .unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN DISTINCT n.city").unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_distinct_with_order_by() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice", city: "Tokyo"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Bob", city: "Tokyo"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", city: "Osaka"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN DISTINCT n.city ORDER BY n.city",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Osaka".to_string())
        );
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Tokyo".to_string())
        );
    }

    // ========== NULLS FIRST/LAST tests ==========

    #[test]
    fn test_nulls_last_default_asc() {
        let mut graph = Graph::new();
        // Create nodes with and without age property
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age ASC",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // ASC default: NULLS LAST
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Charlie".to_string())
        ); // age 25
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Alice".to_string())
        ); // age 30
        assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string())); // NULL
        assert_eq!(result.rows[2].columns[1], Value::Null);
    }

    #[test]
    fn test_nulls_first_default_desc() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // DESC default: NULLS FIRST
        assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string())); // NULL
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Alice".to_string())
        ); // age 30
        assert_eq!(
            result.rows[2].columns[0],
            Value::String("Charlie".to_string())
        ); // age 25
    }

    #[test]
    fn test_nulls_first_explicit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age ASC NULLS FIRST",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // NULLS FIRST explicitly
        assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string())); // NULL
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Charlie".to_string())
        ); // age 25
        assert_eq!(
            result.rows[2].columns[0],
            Value::String("Alice".to_string())
        ); // age 30
    }

    #[test]
    fn test_nulls_last_explicit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC NULLS LAST",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // NULLS LAST explicitly with DESC
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        ); // age 30
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Charlie".to_string())
        ); // age 25
        assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string())); // NULL
    }

    // ========== TopN optimization tests ==========

    #[test]
    fn test_topn_optimization() {
        let mut graph = Graph::new();
        // Create 10 nodes
        for i in 1..=10 {
            execute(
                &mut graph,
                &format!(
                    r#"CREATE (n:Person {{name: "Person{}", age: {}}})"#,
                    i,
                    i * 10
                ),
            )
            .unwrap();
        }

        // Request only top 3 by age DESC
        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC LIMIT 3",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(result.rows[0].columns[1], Value::Int(100)); // age 100
        assert_eq!(result.rows[1].columns[1], Value::Int(90)); // age 90
        assert_eq!(result.rows[2].columns[1], Value::Int(80)); // age 80
    }

    #[test]
    fn test_topn_with_skip() {
        let mut graph = Graph::new();
        // Create 10 nodes
        for i in 1..=10 {
            execute(
                &mut graph,
                &format!(
                    r#"CREATE (n:Person {{name: "Person{}", age: {}}})"#,
                    i,
                    i * 10
                ),
            )
            .unwrap();
        }

        // Skip 2, take 3 (should get 3rd, 4th, 5th highest)
        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC SKIP 2 LIMIT 3",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(result.rows[0].columns[1], Value::Int(80)); // 3rd highest
        assert_eq!(result.rows[1].columns[1], Value::Int(70)); // 4th highest
        assert_eq!(result.rows[2].columns[1], Value::Int(60)); // 5th highest
    }

    // ========== shortestPath / allShortestPaths tests ==========

    #[test]
    fn test_shortest_path_function() {
        let mut graph = Graph::new();
        // Create: A -> B -> C -> D
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        let d = graph.create_node("Node");
        graph
            .get_node_mut(d)
            .unwrap()
            .set_property("name", PropertyValue::String("D".to_string()));
        graph.create_edge(c, d, "NEXT").unwrap();

        // Find shortest path from A to D
        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"}), (d:Node {name: "D"}) RETURN shortestPath(a, d)"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        if let Value::Path { nodes, edges } = &result.rows[0].columns[0] {
            assert_eq!(nodes.len(), 4); // A, B, C, D
            assert_eq!(edges.len(), 3); // 3 edges
            assert_eq!(nodes[0], a);
            assert_eq!(nodes[3], d);
        } else {
            panic!("Expected Path value, got {:?}", result.rows[0].columns[0]);
        }
    }

    #[test]
    fn test_shortest_path_no_path() {
        let mut graph = Graph::new();
        // Create disconnected nodes: A, B (no edge between them)
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));

        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"}), (b:Node {name: "B"}) RETURN shortestPath(a, b)"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Null);
    }

    #[test]
    fn test_all_shortest_paths_single() {
        let mut graph = Graph::new();
        // Create: A -> B -> C (only one path)
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));
        graph.create_edge(a, b, "NEXT").unwrap();

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));
        graph.create_edge(b, c, "NEXT").unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"}), (c:Node {name: "C"}) RETURN allShortestPaths(a, c)"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        if let Value::List(paths) = &result.rows[0].columns[0] {
            assert_eq!(paths.len(), 1);
            if let Value::Path { nodes, edges } = &paths[0] {
                assert_eq!(nodes.len(), 3); // A, B, C
                assert_eq!(edges.len(), 2);
            } else {
                panic!("Expected Path value in list");
            }
        } else {
            panic!("Expected List value");
        }
    }

    #[test]
    fn test_all_shortest_paths_multiple() {
        let mut graph = Graph::new();
        // Create diamond: A -> B -> D, A -> C -> D
        let a = graph.create_node("Node");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("name", PropertyValue::String("A".to_string()));

        let b = graph.create_node("Node");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("name", PropertyValue::String("B".to_string()));

        let c = graph.create_node("Node");
        graph
            .get_node_mut(c)
            .unwrap()
            .set_property("name", PropertyValue::String("C".to_string()));

        let d = graph.create_node("Node");
        graph
            .get_node_mut(d)
            .unwrap()
            .set_property("name", PropertyValue::String("D".to_string()));

        graph.create_edge(a, b, "E1").unwrap();
        graph.create_edge(a, c, "E2").unwrap();
        graph.create_edge(b, d, "E3").unwrap();
        graph.create_edge(c, d, "E4").unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (a:Node {name: "A"}), (d:Node {name: "D"}) RETURN allShortestPaths(a, d)"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        if let Value::List(paths) = &result.rows[0].columns[0] {
            assert_eq!(paths.len(), 2); // Two shortest paths: A-B-D and A-C-D
            for path in paths {
                if let Value::Path { nodes, edges } = path {
                    assert_eq!(nodes.len(), 3); // A, middle, D
                    assert_eq!(edges.len(), 2);
                    assert_eq!(nodes[0], a);
                    assert_eq!(nodes[2], d);
                } else {
                    panic!("Expected Path value in list");
                }
            }
        } else {
            panic!("Expected List value");
        }
    }

    // ========== OPTIONAL MATCH tests ==========

    #[test]
    fn test_optional_match_with_match() {
        let mut graph = Graph::new();
        // Create Alice -> Bob, Charlie (no outgoing edge)
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

        // OPTIONAL MATCH: Alice has a friend, Charlie doesn't
        let result = execute(
            &mut graph,
            r#"MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a.name, b.name ORDER BY a.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 3); // Alice, Bob, Charlie
        // Alice -> Bob
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
        // Bob has no outgoing KNOWS
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
        assert_eq!(result.rows[1].columns[1], Value::Null);
        // Charlie has no outgoing KNOWS
        assert_eq!(
            result.rows[2].columns[0],
            Value::String("Charlie".to_string())
        );
        assert_eq!(result.rows[2].columns[1], Value::Null);
    }

    #[test]
    fn test_optional_match_no_match() {
        let mut graph = Graph::new();
        // Create just Alice with no relationships
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a.name, b.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[0].columns[1], Value::Null);
    }

    #[test]
    fn test_optional_match_all_match() {
        let mut graph = Graph::new();
        // Everyone knows someone
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a.name, b.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
    }

    // ========== CASE WHEN tests ==========

    #[test]
    fn test_case_when_searched_in_where() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 15})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 65})"#,
        )
        .unwrap();

        // Use CASE in WHERE to filter: only adults (age >= 18)
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE CASE WHEN n.age >= 18 THEN true ELSE false END RETURN n.name ORDER BY n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    #[test]
    fn test_case_when_with_else() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 15})"#).unwrap();

        // CASE with multiple WHEN and ELSE
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE CASE WHEN n.age < 18 THEN false WHEN n.age >= 18 THEN true ELSE false END RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_case_when_no_match_returns_null() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        // CASE without ELSE and no match returns NULL
        // This test uses the fact that NULL is not true, so WHERE filters it out
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE CASE WHEN n.age < 18 THEN true END RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 0); // Alice's age is 30, CASE returns NULL, not true
    }

    #[test]
    fn test_case_when_simple_form() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice", status: 1})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", status: 2})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", status: 1})"#,
        )
        .unwrap();

        // Simple CASE: CASE n.status WHEN 1 THEN true ELSE false END
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE CASE n.status WHEN 1 THEN true ELSE false END RETURN n.name ORDER BY n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    // ========== WITH clause tests ==========

    #[test]
    fn test_with_passthrough() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

        // WITH n simply passes bindings through
        let result = execute(
            &mut graph,
            "MATCH (n:Person) WITH n RETURN n.name ORDER BY n.name",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
    }

    #[test]
    fn test_with_alias() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

        // WITH n.name AS name projects property into a new variable
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WITH n.name AS name RETURN name ORDER BY name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
    }

    #[test]
    fn test_with_limit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();

        // WITH n LIMIT 2 restricts intermediate results
        let result = execute(&mut graph, "MATCH (n:Person) WITH n LIMIT 2 RETURN n.name").unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_with_where_filter() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
        )
        .unwrap();

        // WITH + WHERE filters on projected values
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WITH n WHERE n.age > 28 RETURN n.name ORDER BY n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(
            result.rows[1].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    // ========== WITH group aggregation tests (task_95) ==========

    #[test]
    fn test_with_group_count() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (:Person {name: "Alice", city: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Person {name: "Charlie", city: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Person {name: "Bob", city: "Osaka"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WITH n.city AS city, COUNT(n) AS cnt RETURN city, cnt ORDER BY cnt DESC"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2, "Expected 2 groups (Tokyo, Osaka)");
        // First row: Tokyo with cnt=2
        assert_eq!(result.rows[0].columns[0], Value::String("Tokyo".to_string()));
        assert_eq!(result.rows[0].columns[1], Value::Int(2));
        // Second row: Osaka with cnt=1
        assert_eq!(result.rows[1].columns[0], Value::String("Osaka".to_string()));
        assert_eq!(result.rows[1].columns[1], Value::Int(1));
    }

    #[test]
    fn test_with_group_sum() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (:Sale {region: "East", amount: 100})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Sale {region: "East", amount: 200})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Sale {region: "West", amount: 50})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (o:Sale) WITH o.region AS region, SUM(o.amount) AS total RETURN region, total ORDER BY total DESC"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("East".to_string()));
        assert_eq!(result.rows[0].columns[1], Value::Int(300));
        assert_eq!(result.rows[1].columns[0], Value::String("West".to_string()));
        assert_eq!(result.rows[1].columns[1], Value::Int(50));
    }

    #[test]
    fn test_with_aggregate_pipeline_then_return() {
        // WITH aggregation の結果をさらに RETURN で絞り込む
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (:Item {cat: "A", val: 10})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Item {cat: "A", val: 20})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Item {cat: "B", val: 5})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (i:Item) WITH i.cat AS cat, COUNT(i) AS cnt RETURN cat, cnt ORDER BY cat"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("A".to_string()));
        assert_eq!(result.rows[0].columns[1], Value::Int(2));
        assert_eq!(result.rows[1].columns[0], Value::String("B".to_string()));
        assert_eq!(result.rows[1].columns[1], Value::Int(1));
    }

    // ========== Regex match (=~) tests ==========

    #[test]
    fn test_regex_match() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Anna"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name =~ "A.*" RETURN n.name ORDER BY n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[1].columns[0], Value::String("Anna".to_string()));
    }

    #[test]
    fn test_regex_no_match() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name =~ "B.*" RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 0);
    }

    #[test]
    fn test_regex_full_match() {
        // =~ should be a full match, not partial
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

        // "lic" should NOT match "Alice" (partial match)
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name =~ "lic" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 0);

        // ".*lic.*" should match "Alice"
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name =~ ".*lic.*" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
    }

    #[test]
    fn test_regex_type_mismatch_returns_no_rows() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        // =~ on non-string operand evaluates to false in WHERE context
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.age =~ ".*" RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 0);
    }

    // ========== UNION / UNION ALL tests ==========

    #[test]
    fn test_union_basic() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Company {name: "Acme"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.columns, vec!["n.name".to_string()]);
    }

    #[test]
    fn test_union_dedup() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Company {name: "Alice"})"#).unwrap();

        // UNION should remove duplicates
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_union_all() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Company {name: "Alice"})"#).unwrap();

        // UNION ALL should keep duplicates
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) RETURN n.name UNION ALL MATCH (n:Company) RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_union_column_count_mismatch() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Company {name: "Acme"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) RETURN n.name, n.age UNION MATCH (n:Company) RETURN n.name"#,
        );

        assert!(result.is_err());
    }

    // ========== MATCH + CREATE tests ==========

    #[test]
    fn test_match_create_relationship() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
        )
        .unwrap();

        assert_eq!(graph.edge_count(), 1);
        let edge = graph.edges().next().unwrap();
        assert_eq!(edge.label, "KNOWS");
        // created_nodes should be 0 (reusing existing), created_edges should be 1
        assert_eq!(result.rows[0].columns[1], Value::Int(1));
    }

    #[test]
    fn test_match_create_new_node_and_relationship() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}) CREATE (a)-[:OWNS]->(c:Car {model: "Tesla"})"#,
        )
        .unwrap();

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);

        let car = graph
            .nodes()
            .find(|n| n.has_label("Car"))
            .expect("Car node should exist");
        assert_eq!(
            car.get_property("model"),
            Some(&PropertyValue::String("Tesla".to_string()))
        );
    }

    #[test]
    fn test_match_create_with_where() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice", age: 25})"#).unwrap();
        execute(&mut graph, r#"CREATE (b:Person {name: "Bob", age: 15})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (a:Person) WHERE a.age > 20 CREATE (a)-[:MEMBER_OF]->(g:Group {name: "Adults"})"#,
        )
        .unwrap();

        // Only Alice (age 25) should have the relationship
        assert_eq!(graph.edge_count(), 1);
    }

    // バインディングキャッシュ + プロパティインデックス連携テスト (#74)

    #[test]
    fn test_bound_variable_reused_not_duplicated() {
        // MATCH で束縛した変数を CREATE で再利用するとノードが新規作成されないことを確認
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
        )
        .unwrap();

        // ノードが新規作成されていないこと
        assert_eq!(graph.node_count(), 2);
        // エッジが1本だけ作成されていること
        assert_eq!(graph.edge_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(0)); // created_nodes = 0
        assert_eq!(result.rows[0].columns[1], Value::Int(1)); // created_edges = 1
    }

    #[test]
    fn test_same_variable_matched_twice_uses_cache() {
        // 同一変数を MATCH 内で2パターン参照した場合、2回目は既存バインドを流用
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice", age: 30})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (b:Person {name: "Alice", age: 25})"#, // name 同じ・age 違う
        )
        .unwrap();

        // name="Alice" かつ age=30 のノードだけがマッチするはず
        let result = execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}), (a:Person {age: 30}) RETURN a.name, a.age"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[1], Value::Int(30));
    }

    #[test]
    fn test_index_accelerated_match_create() {
        // プロパティインデックスが CREATE KNOWS の MATCH フェーズを高速化することを確認
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        // インデックス作成
        execute_with(&mut executor, "CREATE INDEX ON :Person(name)").unwrap();

        // ノード作成（インデックスに自動登録される）
        execute_with(&mut executor, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute_with(&mut executor, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

        // インデックス経由で MATCH し、エッジを作成
        let result = execute_with(
            &mut executor,
            r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
        )
        .unwrap();

        // ノード再作成なし、エッジ1本
        assert_eq!(result.rows[0].columns[0], Value::Int(0)); // created_nodes
        assert_eq!(result.rows[0].columns[1], Value::Int(1)); // created_edges

        // インデックスが a・b を引けていることをプロパティで確認
        let check = execute_with(
            &mut executor,
            r#"MATCH (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"}) RETURN a.name, b.name"#,
        )
        .unwrap();
        assert_eq!(check.row_count(), 1);
    }

    // ========== MATCH + SET tests ==========

    #[test]
    fn test_match_set_property() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(31)));
    }

    #[test]
    fn test_match_set_multiple_properties() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31, n.city = "Tokyo""#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(31)));
        assert_eq!(
            node.get_property("city"),
            Some(&PropertyValue::String("Tokyo".to_string()))
        );
    }

    #[test]
    fn test_match_set_with_return() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31 RETURN n.name, n.age"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[0].columns[1], Value::Int(31));
    }

    #[test]
    fn test_match_set_edge_property() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        execute(
            &mut graph,
            r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) SET r.since = 2024"#,
        )
        .unwrap();

        let edge = graph.edges().next().unwrap();
        assert_eq!(edge.get_property("since"), Some(&PropertyValue::Int(2024)));
    }

    // ========== SET += and SET n:Label tests ==========

    #[test]
    fn test_set_merge_properties_adds_new_props() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) SET n += {age: 31, city: "Tokyo"}"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        // Existing property 'name' must be preserved
        assert_eq!(
            node.get_property("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        // 'age' should be updated to 31
        assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(31)));
        // New property 'city' should be added
        assert_eq!(
            node.get_property("city"),
            Some(&PropertyValue::String("Tokyo".to_string()))
        );
    }

    #[test]
    fn test_set_merge_properties_with_return() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) SET n += {age: 31, city: "Tokyo"} RETURN n.name, n.age, n.city"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[0].columns[1], Value::Int(31));
        assert_eq!(
            result.rows[0].columns[2],
            Value::String("Tokyo".to_string())
        );
    }

    #[test]
    fn test_set_add_label_to_node() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.age >= 20 SET n:Adult"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        // Node should now be matchable by both Person and Adult labels
        assert!(node.has_label("Person"));
        assert!(node.has_label("Adult"));
    }

    #[test]
    fn test_set_add_label_idempotent() {
        // Adding the same label twice should not duplicate it
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

        execute(&mut graph, r#"MATCH (n:Person) SET n:Adult"#).unwrap();
        execute(&mut graph, r#"MATCH (n:Person) SET n:Adult"#).unwrap();

        let node = graph.nodes().next().unwrap();
        let label_count = node.labels.iter().filter(|l| *l == "Adult").count();
        assert_eq!(label_count, 1, "Label 'Adult' should appear only once");
    }

    #[test]
    fn test_set_add_label_match_by_new_label() {
        // After SET n:Adult, the node should be matchable by (n:Adult)
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.age >= 20 SET n:Adult"#,
        )
        .unwrap();

        let result = execute(&mut graph, r#"MATCH (n:Adult) RETURN n.name"#).unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_set_add_label_with_return() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) SET n:Adult RETURN n.name, labels(n)"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        // labels(n) should contain both Person and Adult
        let labels_val = &result.rows[0].columns[1];
        if let Value::List(labels) = labels_val {
            assert!(labels.contains(&Value::String("Person".to_string())));
            assert!(labels.contains(&Value::String("Adult".to_string())));
        } else {
            panic!("Expected list of labels, got {:?}", labels_val);
        }
    }

    #[test]
    fn test_set_merge_properties_edge() {
        // SET += on an edge should also merge properties
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS {since: 2020}]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        execute(
            &mut graph,
            r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) SET r += {weight: 5}"#,
        )
        .unwrap();

        let edge = graph.edges().next().unwrap();
        // Existing property 'since' must be preserved
        assert_eq!(edge.get_property("since"), Some(&PropertyValue::Int(2020)));
        // New property 'weight' should be added
        assert_eq!(edge.get_property("weight"), Some(&PropertyValue::Int(5)));
    }

    // ========== MERGE tests ==========

    #[test]
    fn test_merge_create_new() {
        let mut graph = Graph::new();

        execute(&mut graph, r#"MERGE (n:Person {name: "Alice"})"#).unwrap();

        assert_eq!(graph.node_count(), 1);
        let node = graph.nodes().next().unwrap();
        assert!(node.has_label("Person"));
        assert_eq!(
            node.get_property("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_merge_match_existing() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        assert_eq!(graph.node_count(), 1);

        execute(&mut graph, r#"MERGE (n:Person {name: "Alice"})"#).unwrap();

        // Should still be 1 node (matched existing)
        assert_eq!(graph.node_count(), 1);
    }

    #[test]
    fn test_merge_on_create_set() {
        let mut graph = Graph::new();

        execute(
            &mut graph,
            r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25"#,
        )
        .unwrap();

        assert_eq!(graph.node_count(), 1);
        let node = graph.nodes().next().unwrap();
        assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(25)));
    }

    #[test]
    fn test_merge_on_match_set() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

        execute(
            &mut graph,
            r#"MERGE (n:Person {name: "Alice"}) ON MATCH SET n.age = 30"#,
        )
        .unwrap();

        assert_eq!(graph.node_count(), 1);
        let node = graph.nodes().next().unwrap();
        assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(30)));
    }

    #[test]
    fn test_merge_with_match_prefix() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) MERGE (a)-[:KNOWS]->(b)"#,
        )
        .unwrap();

        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_merge_with_return() {
        let mut graph = Graph::new();

        let result = execute(
            &mut graph,
            r#"MERGE (n:Person {name: "Charlie"}) ON CREATE SET n.age = 25 RETURN n.name, n.age"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Charlie".to_string())
        );
        assert_eq!(result.rows[0].columns[1], Value::Int(25));
    }

    // ========== MATCH + REMOVE tests ==========

    #[test]
    fn test_match_remove_property() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice", age: 30, city: "Tokyo"})"#,
        )
        .unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        assert_eq!(node.get_property("age"), None);
        assert_eq!(
            node.get_property("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_match_remove_multiple_properties() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice", age: 30, city: "Tokyo"})"#,
        )
        .unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age, n.city"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        assert_eq!(node.get_property("age"), None);
        assert_eq!(node.get_property("city"), None);
        assert_eq!(
            node.get_property("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_match_remove_with_return() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age RETURN n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_match_remove_edge_property() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // First set a property on the edge
        execute(
            &mut graph,
            r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) SET r.since = 2024"#,
        )
        .unwrap();

        // Then remove it
        execute(
            &mut graph,
            r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) REMOVE r.since"#,
        )
        .unwrap();

        let edge = graph.edges().next().unwrap();
        assert_eq!(edge.get_property("since"), None);
    }

    #[test]
    fn test_match_remove_label() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n:Person"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        assert!(!node.has_label("Person"), "Person label should be removed");
    }

    // ========== UNWIND tests ==========

    #[test]
    fn test_unwind_list() {
        let mut graph = Graph::new();

        let result = execute(&mut graph, "UNWIND [1, 2, 3] AS x RETURN x").unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(result.rows[0].columns[0], Value::Int(1));
        assert_eq!(result.rows[1].columns[0], Value::Int(2));
        assert_eq!(result.rows[2].columns[0], Value::Int(3));
    }

    #[test]
    fn test_unwind_string_list() {
        let mut graph = Graph::new();

        let result = execute(
            &mut graph,
            r#"UNWIND ["Alice", "Bob", "Charlie"] AS name RETURN name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
        assert_eq!(
            result.rows[2].columns[0],
            Value::String("Charlie".to_string())
        );
    }

    #[test]
    fn test_unwind_empty_list() {
        let mut graph = Graph::new();

        let result = execute(&mut graph, "UNWIND [] AS x RETURN x").unwrap();

        assert_eq!(result.row_count(), 0);
    }

    // ========== Parser tests for new features ==========

    #[test]
    fn test_parse_match_create() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

        // Verify the query parses and executes correctly
        let result = execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:FRIENDS]->(b)"#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_match_set() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31 RETURN n.age"#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_merge() {
        let mut graph = Graph::new();

        let result = execute(
            &mut graph,
            r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25 ON MATCH SET n.age = 30 RETURN n"#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_remove() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age RETURN n.name"#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_unwind() {
        let mut graph = Graph::new();

        let result = execute(&mut graph, "UNWIND [1, 2, 3] AS x RETURN x");
        assert!(result.is_ok());
    }

    // ========== Constraint DDL tests ==========

    fn execute_with_constraints(
        graph: &mut Graph,
        queries: &[&str],
    ) -> Vec<Result<ResultSet, ExecuteError>> {
        let mut executor = Executor::new(graph);
        queries
            .iter()
            .map(|q| {
                let stmt = Parser::new(q).unwrap().parse().unwrap();
                executor.execute(stmt)
            })
            .collect()
    }

    #[test]
    fn test_create_unique_constraint() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &["CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE"],
        );
        assert!(results[0].is_ok());
        let rs = results[0].as_ref().unwrap();
        assert_eq!(
            rs.rows[0].columns[0],
            Value::String("Constraint 'unique_email' created".to_string())
        );
    }

    #[test]
    fn test_create_not_null_constraint() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &["CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL"],
        );
        assert!(results[0].is_ok());
    }

    #[test]
    fn test_create_type_constraint() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &["CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER"],
        );
        assert!(results[0].is_ok());
    }

    #[test]
    fn test_drop_constraint() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
                "DROP CONSTRAINT unique_email",
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        let rs = results[1].as_ref().unwrap();
        assert_eq!(
            rs.rows[0].columns[0],
            Value::String("Constraint 'unique_email' dropped".to_string())
        );
    }

    #[test]
    fn test_drop_nonexistent_constraint() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(&mut graph, &["DROP CONSTRAINT nonexistent"]);
        assert!(matches!(results[0], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_show_constraints() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
                "CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL",
                "SHOW CONSTRAINTS",
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(results[2].is_ok());
        let rs = results[2].as_ref().unwrap();
        assert_eq!(rs.columns, vec!["name", "label", "properties", "type"]);
        assert_eq!(rs.rows.len(), 2);
    }

    #[test]
    fn test_show_constraints_empty() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(&mut graph, &["SHOW CONSTRAINTS"]);
        assert!(results[0].is_ok());
        let rs = results[0].as_ref().unwrap();
        assert_eq!(rs.rows.len(), 0);
    }

    #[test]
    fn test_unique_constraint_enforced_on_create() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
                r#"CREATE (n:Person {email: "alice@example.com"})"#,
                r#"CREATE (n:Person {email: "alice@example.com"})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(matches!(results[2], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_not_null_constraint_enforced_on_create() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL",
                r#"CREATE (n:Person {age: 30})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_type_constraint_enforced_on_create() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER",
                r#"CREATE (n:Person {age: "thirty"})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_type_constraint_allows_valid_type() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER",
                r#"CREATE (n:Person {age: 30})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
    }

    #[test]
    fn test_unique_constraint_enforced_on_set() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
                r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#,
                r#"CREATE (n:Person {name: "Bob", email: "bob@example.com"})"#,
                r#"MATCH (n:Person {name: "Bob"}) SET n.email = "alice@example.com""#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(results[2].is_ok());
        assert!(matches!(results[3], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_not_null_constraint_enforced_on_remove() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL",
                r#"CREATE (n:Person {name: "Alice"})"#,
                r#"MATCH (n:Person {name: "Alice"}) REMOVE n.name"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(matches!(results[2], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_constraint_different_label_not_enforced() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
                r#"CREATE (n:Person {email: "shared@example.com"})"#,
                r#"CREATE (n:Company {email: "shared@example.com"})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(results[2].is_ok()); // different label, should pass
    }

    #[test]
    fn test_composite_unique_constraint_enforced() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT unique_name_email FOR (n:Person) REQUIRE (n.name, n.email) IS UNIQUE",
                r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#,
                r#"CREATE (n:Person {name: "Alice", email: "different@example.com"})"#, // Different email, should pass
                r#"CREATE (n:Person {name: "Bob", email: "alice@example.com"})"#, // Different name, should pass
                r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#, // Same combo, should fail
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(results[2].is_ok()); // different combo, should pass
        assert!(results[3].is_ok()); // different combo, should pass
        assert!(matches!(results[4], Err(ExecuteError::ConstraintError(_)))); // duplicate combo
    }

    #[test]
    fn test_composite_unique_constraint_missing_property() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT unique_name_email FOR (n:Person) REQUIRE (n.name, n.email) IS UNIQUE",
                r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#,
                r#"CREATE (n:Person {name: "Bob"})"#, // Missing email, constraint doesn't apply
                r#"CREATE (n:Person {email: "alice@example.com"})"#, // Missing name, constraint doesn't apply
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(results[2].is_ok()); // missing property, should pass
        assert!(results[3].is_ok()); // missing property, should pass
    }

    #[test]
    fn test_parse_composite_unique_constraint() {
        let stmt = Parser::new("CREATE CONSTRAINT unique_name_email FOR (n:Person) REQUIRE (n.name, n.email) IS UNIQUE")
            .unwrap()
            .parse()
            .unwrap();
        if let Statement::CreateConstraint(cc) = stmt {
            assert_eq!(cc.name, "unique_name_email");
            assert_eq!(cc.label, "Person");
            assert_eq!(cc.properties, vec!["name".to_string(), "email".to_string()]);
            assert_eq!(cc.constraint_type, ConstraintTypeAst::Unique);
        } else {
            panic!("expected CreateConstraint statement");
        }
    }

    #[test]
    fn test_duplicate_constraint_name_error() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT c1 FOR (n:Person) REQUIRE n.email IS UNIQUE",
                "CREATE CONSTRAINT c1 FOR (n:Person) REQUIRE n.name IS NOT NULL",
            ],
        );
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_parse_create_constraint() {
        let stmt =
            Parser::new("CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE")
                .unwrap()
                .parse()
                .unwrap();
        if let Statement::CreateConstraint(cc) = stmt {
            assert_eq!(cc.name, "unique_email");
            assert_eq!(cc.label, "Person");
            assert_eq!(cc.properties, vec!["email".to_string()]);
            assert_eq!(cc.constraint_type, ConstraintTypeAst::Unique);
        } else {
            panic!("expected CreateConstraint statement");
        }
    }

    #[test]
    fn test_parse_create_constraint_not_null() {
        let stmt =
            Parser::new("CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL")
                .unwrap()
                .parse()
                .unwrap();
        if let Statement::CreateConstraint(cc) = stmt {
            assert_eq!(cc.constraint_type, ConstraintTypeAst::NotNull);
        } else {
            panic!("expected CreateConstraint statement");
        }
    }

    #[test]
    fn test_parse_create_constraint_type_check() {
        let stmt =
            Parser::new("CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER")
                .unwrap()
                .parse()
                .unwrap();
        if let Statement::CreateConstraint(cc) = stmt {
            assert_eq!(
                cc.constraint_type,
                ConstraintTypeAst::TypeCheck(PropertyTypeAst::Integer)
            );
        } else {
            panic!("expected CreateConstraint statement");
        }
    }

    #[test]
    fn test_parse_drop_constraint() {
        let stmt = Parser::new("DROP CONSTRAINT unique_email")
            .unwrap()
            .parse()
            .unwrap();
        if let Statement::DropConstraint(dc) = stmt {
            assert_eq!(dc.name, "unique_email");
        } else {
            panic!("expected DropConstraint statement");
        }
    }

    #[test]
    fn test_parse_show_constraints() {
        let stmt = Parser::new("SHOW CONSTRAINTS").unwrap().parse().unwrap();
        assert_eq!(stmt, Statement::ShowConstraints);
    }

    // ========== Label constraint tests ==========

    #[test]
    fn test_parse_required_label_constraint() {
        let stmt = Parser::new(
            "CREATE CONSTRAINT employee_is_person FOR (n:Employee) REQUIRE n:Person",
        )
        .unwrap()
        .parse()
        .unwrap();
        if let Statement::CreateConstraint(cc) = stmt {
            assert_eq!(cc.name, "employee_is_person");
            assert_eq!(cc.label, "Employee");
            assert_eq!(cc.properties, Vec::<String>::new());
            assert_eq!(
                cc.constraint_type,
                ConstraintTypeAst::RequiredLabel("Person".to_string())
            );
        } else {
            panic!("expected CreateConstraint statement");
        }
    }

    #[test]
    fn test_required_label_constraint_pass() {
        let mut graph = Graph::new();
        // Employee requires Person label; since our graph model has single labels,
        // this passes when the node's label matches the required label (Person)
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT emp_is_person FOR (n:Employee) REQUIRE n:Person",
                // Creating a Person node satisfies the Employee->Person constraint
                // (the constraint says: if label=Employee, require label=Person)
                // Since Person != Employee, creating Employee would fail
                r#"CREATE (n:Person {name: "Alice"})"#, // Person node, not Employee - not affected by constraint
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok()); // Person node bypasses Employee constraint
    }

    #[test]
    fn test_required_label_constraint_violation() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT emp_is_person FOR (n:Employee) REQUIRE n:Person",
                r#"CREATE (n:Employee {name: "Bob"})"#, // Employee but not Person - violates constraint
            ],
        );
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_required_label_constraint_satisfied_by_same_label() {
        let mut graph = Graph::new();
        // If required label equals the node label, the constraint is trivially satisfied
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT person_is_person FOR (n:Person) REQUIRE n:Person",
                r#"CREATE (n:Person {name: "Alice"})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok()); // Person satisfies Person requirement
    }

    #[test]
    fn test_parse_endpoint_label_constraint() {
        let stmt = Parser::new(
            "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
        )
        .unwrap()
        .parse()
        .unwrap();
        if let Statement::CreateConstraint(cc) = stmt {
            assert_eq!(cc.name, "knows_persons");
            assert_eq!(cc.label, "KNOWS");
            assert_eq!(cc.properties, Vec::<String>::new());
            assert_eq!(
                cc.constraint_type,
                ConstraintTypeAst::EndpointLabel {
                    source_label: "Person".to_string(),
                    target_label: "Person".to_string(),
                }
            );
        } else {
            panic!("expected CreateConstraint statement");
        }
    }

    #[test]
    fn test_endpoint_label_constraint_violation() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
                r#"CREATE (p:Person {name: "Alice"})-[:KNOWS]->(c:Company {name: "Acme"})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_endpoint_label_constraint_pass() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
                r#"CREATE (p:Person {name: "Alice"})-[:KNOWS]->(q:Person {name: "Bob"})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
    }

    #[test]
    fn test_endpoint_label_constraint_source_violation() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
                r#"CREATE (c:Company {name: "Acme"})-[:KNOWS]->(p:Person {name: "Alice"})"#,
            ],
        );
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
    }

    #[test]
    fn test_show_constraints_includes_label_constraints() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &[
                "CREATE CONSTRAINT emp_is_person FOR (n:Employee) REQUIRE n:Person",
                "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
                "SHOW CONSTRAINTS",
            ],
        );
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert!(results[2].is_ok());
        let rs = results[2].as_ref().unwrap();
        assert_eq!(rs.rows.len(), 2);
        // Both constraints should be listed
        let names: Vec<String> = rs
            .rows
            .iter()
            .map(|r| {
                if let Value::String(s) = &r.columns[0] {
                    s.clone()
                } else {
                    String::new()
                }
            })
            .collect();
        assert!(names.contains(&"emp_is_person".to_string()));
        assert!(names.contains(&"knows_persons".to_string()));
    }

    // ========== EXPLAIN / PROFILE tests ==========

    #[test]
    fn test_explain_match() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        let result = execute(&mut graph, "EXPLAIN MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(result.columns, vec!["plan"]);
        assert!(!result.rows.is_empty());

        // Check that plan contains expected operators
        let plan_text: String = result
            .rows
            .iter()
            .map(|r| {
                if let Value::String(s) = &r.columns[0] {
                    s.clone()
                } else {
                    String::new()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan_text.contains("NodeByLabelScan"));
        assert!(plan_text.contains("Projection"));
    }

    #[test]
    fn test_explain_match_with_filter() {
        let mut graph = Graph::new();
        let result = execute(
            &mut graph,
            "EXPLAIN MATCH (n:Person) WHERE n.age > 30 RETURN n",
        )
        .unwrap();
        let plan_text: String = result
            .rows
            .iter()
            .filter_map(|r| {
                if let Value::String(s) = &r.columns[0] {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan_text.contains("NodeByLabelScan"));
        assert!(plan_text.contains("Filter"));
        assert!(plan_text.contains("Projection"));
    }

    #[test]
    fn test_explain_create() {
        let mut graph = Graph::new();
        let result = execute(&mut graph, r#"EXPLAIN CREATE (n:Person {name: "Alice"})"#).unwrap();
        let plan_text: String = result
            .rows
            .iter()
            .filter_map(|r| {
                if let Value::String(s) = &r.columns[0] {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan_text.contains("CreateNode"));
        // EXPLAIN should NOT actually create the node
        assert_eq!(graph.node_count(), 0);
    }

    #[test]
    fn test_explain_does_not_mutate_graph() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        assert_eq!(graph.node_count(), 1);

        // EXPLAIN should not add/remove nodes
        execute(&mut graph, r#"EXPLAIN CREATE (m:Person {name: "Bob"})"#).unwrap();
        assert_eq!(graph.node_count(), 1);
    }

    #[test]
    fn test_profile_match() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

        let result = execute(&mut graph, "PROFILE MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(result.columns, vec!["profile"]);
        assert!(!result.rows.is_empty());

        let plan_text: String = result
            .rows
            .iter()
            .filter_map(|r| {
                if let Value::String(s) = &r.columns[0] {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        // PROFILE output should contain actual execution stats
        assert!(plan_text.contains("Rows:"));
        assert!(plan_text.contains("Time:"));
    }

    #[test]
    fn test_profile_executes_query() {
        let mut graph = Graph::new();
        // PROFILE CREATE should actually create the node (unlike EXPLAIN)
        let result = execute(&mut graph, r#"PROFILE CREATE (n:Person {name: "Alice"})"#).unwrap();
        assert_eq!(graph.node_count(), 1);
        assert!(!result.rows.is_empty());
    }

    #[test]
    fn test_explain_path_pattern() {
        let mut graph = Graph::new();
        let result = execute(
            &mut graph,
            "EXPLAIN MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b",
        )
        .unwrap();
        let plan_text: String = result
            .rows
            .iter()
            .filter_map(|r| {
                if let Value::String(s) = &r.columns[0] {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan_text.contains("NodeByLabelScan"));
        assert!(plan_text.contains("Expand"));
    }

    #[test]
    fn test_explain_merge() {
        let mut graph = Graph::new();
        let result = execute(&mut graph, r#"EXPLAIN MERGE (n:Person {name: "Alice"})"#).unwrap();
        let plan_text: String = result
            .rows
            .iter()
            .filter_map(|r| {
                if let Value::String(s) = &r.columns[0] {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan_text.contains("Merge"));
        // EXPLAIN should not create the node
        assert_eq!(graph.node_count(), 0);
    }

    // ========== CONTAINS operator tests ==========

    #[test]
    fn test_contains_operator() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Article {title: "Graph Database Tutorial"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Article {title: "Relational Database Guide"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Article {title: "Machine Learning Basics"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.title CONTAINS "Database" RETURN n.title"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn test_contains_case_insensitive() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Article {title: "Graph Database"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.title CONTAINS "database" RETURN n.title"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);

        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.title CONTAINS "DATABASE" RETURN n.title"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    // ========== Fulltext index tests ==========

    #[test]
    fn test_create_fulltext_index() {
        let mut graph = Graph::new();
        let result = execute(
            &mut graph,
            r#"CREATE FULLTEXT INDEX article_search FOR (n:Article) ON (n.title, n.body)"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        if let Value::String(s) = &result.rows[0].columns[0] {
            assert!(s.contains("article_search"));
        }
    }

    #[test]
    fn test_drop_fulltext_index() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        let stmt =
            Parser::new(r#"CREATE FULLTEXT INDEX article_search FOR (n:Article) ON (n.title)"#)
                .unwrap()
                .parse()
                .unwrap();
        executor.execute(stmt).unwrap();

        let stmt = Parser::new("DROP FULLTEXT INDEX article_search")
            .unwrap()
            .parse()
            .unwrap();
        let result = executor.execute(stmt).unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn test_drop_nonexistent_fulltext_index() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);
        let stmt = Parser::new("DROP FULLTEXT INDEX nonexistent")
            .unwrap()
            .parse()
            .unwrap();
        let result = executor.execute(stmt);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_fulltext_index_indexes_existing_nodes() {
        let mut graph = Graph::new();
        // Create nodes first
        execute(
            &mut graph,
            r#"CREATE (n:Article {title: "Graph Database Tutorial", body: "Learn about graphs"})"#,
        )
        .unwrap();

        // Then create the index - it should index existing nodes
        let stmt = Parser::new(
            r#"CREATE FULLTEXT INDEX article_search FOR (n:Article) ON (n.title, n.body)"#,
        )
        .unwrap()
        .parse()
        .unwrap();
        let mut executor = Executor::new(&mut graph);
        executor.execute(stmt).unwrap();

        // Verify the index has content
        let results = executor
            .fulltext_manager()
            .get_index("article_search")
            .unwrap()
            .search("graph");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_parse_create_fulltext_index() {
        let stmt = Parser::new(r#"CREATE FULLTEXT INDEX my_idx FOR (n:Person) ON (n.name, n.bio)"#)
            .unwrap()
            .parse()
            .unwrap();
        if let Statement::CreateFulltextIndex(cfi) = stmt {
            assert_eq!(cfi.name, "my_idx");
            assert_eq!(cfi.label, "Person");
            assert_eq!(cfi.variable, "n");
            assert_eq!(cfi.properties, vec!["name", "bio"]);
        } else {
            panic!("expected CreateFulltextIndex statement");
        }
    }

    #[test]
    fn test_parse_drop_fulltext_index() {
        let stmt = Parser::new("DROP FULLTEXT INDEX my_idx")
            .unwrap()
            .parse()
            .unwrap();
        if let Statement::DropFulltextIndex(dfi) = stmt {
            assert_eq!(dfi.name, "my_idx");
        } else {
            panic!("expected DropFulltextIndex statement");
        }
    }

    // ========== Phrase search tests (CONTAINS with quoted phrase) ==========

    #[test]
    fn test_phrase_search_via_contains_matches_adjacent() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "graph database systems are powerful"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "database graph systems"})"#,
        )
        .unwrap();

        // Phrase "graph database" should only match the first article
        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.body CONTAINS '"graph database"' RETURN n.body"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("graph database systems are powerful".to_string())
        );
    }

    #[test]
    fn test_phrase_search_order_matters() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "graph database"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "database graph"})"#,
        )
        .unwrap();

        // "graph database" must match only the first document (order matters)
        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.body CONTAINS '"graph database"' RETURN n.body"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("graph database".to_string())
        );
    }

    #[test]
    fn test_phrase_search_no_match_non_adjacent() {
        let mut graph = Graph::new();
        // Words exist but not adjacent
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "graph systems and database management"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.body CONTAINS '"graph database"' RETURN n.body"#,
        )
        .unwrap();
        assert!(result.rows.is_empty());
    }

    // ========== Fuzzy search tests (CONTAINS with ~ suffix) ==========

    #[test]
    fn test_fuzzy_search_via_contains_default_distance() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "database management systems"})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (n:Article {body: "python code"})"#).unwrap();

        // "dtabase~" has distance 2 from "database" (2 character transpositions)
        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.body CONTAINS 'dtabase~' RETURN n.body"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("database management systems".to_string())
        );
    }

    #[test]
    fn test_fuzzy_search_via_contains_explicit_distance() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "database systems"})"#,
        )
        .unwrap();

        // "databse" has distance 1 from "database"
        // with limit 0, should NOT match
        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.body CONTAINS 'databse~0' RETURN n.body"#,
        )
        .unwrap();
        assert!(result.rows.is_empty());

        // with limit 1, should match
        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.body CONTAINS 'databse~1' RETURN n.body"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn test_fuzzy_search_exact_match_with_tilde() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Article {body: "rust programming language"})"#,
        )
        .unwrap();

        // Exact term with ~ should still match (distance 0 <= 2)
        let result = execute(
            &mut graph,
            r#"MATCH (n:Article) WHERE n.body CONTAINS 'rust~' RETURN n.body"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    // ========== CALL db.index.fulltext.search tests ==========

    #[test]
    fn test_procedure_call_fulltext_search() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        // Create fulltext index
        executor
            .execute(
                Parser::new(
                    r#"CREATE FULLTEXT INDEX article_idx FOR (n:Article) ON (n.title, n.body)"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        // Create nodes (they get indexed automatically)
        executor
            .execute(
                Parser::new(
                    r#"CREATE (n:Article {title: "Graph Database Systems", body: "graph database tutorial"})"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        executor
            .execute(
                Parser::new(
                    r#"CREATE (n:Article {title: "Relational Databases", body: "SQL and tables"})"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        executor
            .execute(
                Parser::new(
                    r#"CREATE (n:Article {title: "Machine Learning", body: "neural networks"})"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        // Search using the procedure call
        let result = executor
            .execute(
                Parser::new(
                    r#"CALL db.index.fulltext.search('article_idx', 'graph') YIELD node, score"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        // Should return rows with "graph" in them (2 articles)
        assert!(!result.rows.is_empty());
        // Scores should be positive
        for row in &result.rows {
            if let Value::Float(score) = row.columns[1] {
                assert!(score > 0.0, "score should be positive");
            } else {
                panic!("second column should be a float score");
            }
        }
        // Columns should be ["node", "score"]
        assert_eq!(result.columns[0], "node");
        assert_eq!(result.columns[1], "score");
    }

    #[test]
    fn test_procedure_call_fulltext_search_unknown_index() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        let result = executor.execute(
            Parser::new(
                r#"CALL db.index.fulltext.search('nonexistent', 'query') YIELD node, score"#,
            )
            .unwrap()
            .parse()
            .unwrap(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_procedure_call_fulltext_search_returns_sorted_scores() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        executor
            .execute(
                Parser::new(
                    r#"CREATE FULLTEXT INDEX idx FOR (n:Doc) ON (n.text)"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        // Document with "graph" appearing many times should rank higher
        executor
            .execute(
                Parser::new(
                    r#"CREATE (n:Doc {text: "graph graph graph graph graph"})"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        executor
            .execute(
                Parser::new(
                    r#"CREATE (n:Doc {text: "graph systems"})"#,
                )
                .unwrap()
                .parse()
                .unwrap(),
            )
            .unwrap();

        let result = executor
            .execute(
                Parser::new(r#"CALL db.index.fulltext.search('idx', 'graph') YIELD node, score"#)
                    .unwrap()
                    .parse()
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(result.rows.len(), 2);
        // Scores should be in descending order
        if let (Value::Float(s1), Value::Float(s2)) = (
            result.rows[0].columns[1].clone(),
            result.rows[1].columns[1].clone(),
        ) {
            assert!(s1 >= s2, "results should be sorted by score descending");
        }
    }

    #[test]
    fn test_parse_procedure_call() {
        let stmt = Parser::new(
            r#"CALL db.index.fulltext.search('my_idx', 'query') YIELD node, score"#,
        )
        .unwrap()
        .parse()
        .unwrap();

        if let Statement::ProcedureCall(pc) = stmt {
            assert_eq!(pc.procedure, "db.index.fulltext.search");
            assert_eq!(pc.arguments.len(), 2);
            assert_eq!(pc.yield_columns, vec!["node", "score"]);
        } else {
            panic!("expected ProcedureCall statement");
        }
    }

    #[test]
    fn test_starts_with_operator() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name STARTS WITH "Ali" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_ends_with_operator() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name ENDS WITH "ice" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_starts_with_case_insensitive() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name STARTS WITH "ali" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn test_ends_with_case_insensitive() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name ENDS WITH "ICE" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn test_is_normalized() {
        let mut graph = Graph::new();
        // NFC normalized string (precomposed)
        execute(&mut graph, "CREATE (n:Text {value: \"\u{00e9}\"})").unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Text) WHERE n.value IS NORMALIZED RETURN n.value"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn test_is_not_normalized() {
        let mut graph = Graph::new();
        // NFD string (decomposed: e + combining acute accent)
        execute(&mut graph, "CREATE (n:Text {value: \"e\u{0301}\"})").unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Text) WHERE n.value IS NORMALIZED RETURN n.value"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 0);
    }

    #[test]
    fn test_string_operators_with_non_string() {
        // WHERE clause errors are treated as non-matching (unwrap_or(false)),
        // so type mismatches result in 0 rows rather than errors.
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {age: 30})"#).unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.age STARTS WITH "3" RETURN n"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 0);

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.age ENDS WITH "0" RETURN n"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 0);

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.age IS NORMALIZED RETURN n"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 0);
    }

    // ========== String function tests ==========

    #[test]
    fn test_trim_functions() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "  hello  "})"#).unwrap();

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN trim(n.val)"#).unwrap();
        assert_eq!(result.columns, vec!["trim(n.val)"]);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("hello".to_string())
        );

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN ltrim(n.val)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("hello  ".to_string())
        );

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN rtrim(n.val)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("  hello".to_string())
        );
    }

    #[test]
    fn test_case_conversion() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "Hello World"})"#).unwrap();

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toLower(n.val)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("hello world".to_string())
        );

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toUpper(n.val)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("HELLO WORLD".to_string())
        );
    }

    #[test]
    fn test_reverse() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "abcde"})"#).unwrap();

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN reverse(n.val)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("edcba".to_string())
        );
    }

    #[test]
    fn test_substring() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "hello world"})"#).unwrap();

        // 2引数: start から末尾まで
        let result = execute(&mut graph, r#"MATCH (n:T) RETURN substring(n.val, 6)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("world".to_string())
        );

        // 3引数: start から len 文字
        let result = execute(&mut graph, r#"MATCH (n:T) RETURN substring(n.val, 0, 5)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_left_right() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "hello world"})"#).unwrap();

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN left(n.val, 5)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("hello".to_string())
        );

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN right(n.val, 5)"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("world".to_string())
        );
    }

    #[test]
    fn test_split() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "a,b,c"})"#).unwrap();

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN split(n.val, ",")"#).unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::List(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
                Value::String("c".to_string()),
            ])
        );
    }

    #[test]
    fn test_replace() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "hello world"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:T) RETURN replace(n.val, "world", "rust")"#,
        )
        .unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("hello rust".to_string())
        );
    }

    #[test]
    fn test_to_string() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:T {i: 42, f: 3.14, b: true, s: "hi"})"#,
        )
        .unwrap();

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.i)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::String("42".to_string()));

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.f)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::String("3.14".to_string()));

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.b)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::String("true".to_string()));

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.s)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::String("hi".to_string()));
    }

    #[test]
    fn test_size() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: "hello"})"#).unwrap();

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN size(n.val)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(5));
    }

    #[test]
    fn test_abs() {
        let mut graph = Graph::new();
        // Test negative int and float
        execute(&mut graph, "CREATE (n:T)").unwrap();
        graph
            .get_node_mut(0)
            .unwrap()
            .set_property("i", PropertyValue::Int(-5));
        graph
            .get_node_mut(0)
            .unwrap()
            .set_property("f", PropertyValue::Float(-3.14));

        let result = execute(&mut graph, "MATCH (n:T) RETURN abs(n.i)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(5));

        let result = execute(&mut graph, "MATCH (n:T) RETURN abs(n.f)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(3.14));

        // Positive values stay the same
        let mut graph2 = Graph::new();
        execute(&mut graph2, "CREATE (n:T {i: 5, f: 3.14})").unwrap();
        let result = execute(&mut graph2, "MATCH (n:T) RETURN abs(n.i)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(5));
    }

    #[test]
    fn test_ceil_floor() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {f: 3.2, i: 5})").unwrap();
        graph
            .get_node_mut(0)
            .unwrap()
            .set_property("g", PropertyValue::Float(-2.7));

        let result = execute(&mut graph, "MATCH (n:T) RETURN ceil(n.f)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(4));

        let result = execute(&mut graph, "MATCH (n:T) RETURN floor(n.f)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(3));

        let result = execute(&mut graph, "MATCH (n:T) RETURN ceil(n.g)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(-2));

        let result = execute(&mut graph, "MATCH (n:T) RETURN floor(n.g)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(-3));

        // Int passthrough
        let result = execute(&mut graph, "MATCH (n:T) RETURN ceil(n.i)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(5));

        let result = execute(&mut graph, "MATCH (n:T) RETURN floor(n.i)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(5));
    }

    #[test]
    fn test_round() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {f: 3.567, i: 5})").unwrap();

        // 1 argument: round to integer
        let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.f)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(4));

        // Int passthrough
        let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.i)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(5));

        // 2 arguments: round to precision
        let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.f, 2)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(3.57));

        let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.f, 1)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(3.6));
    }

    #[test]
    fn test_sign() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {pos: 5, zero: 0, fpos: 2.5})").unwrap();
        graph
            .get_node_mut(0)
            .unwrap()
            .set_property("neg", PropertyValue::Int(-3));
        graph
            .get_node_mut(0)
            .unwrap()
            .set_property("fneg", PropertyValue::Float(-1.5));

        let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.pos)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(1));

        let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.neg)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(-1));

        let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.zero)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(0));

        let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.fpos)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(1));

        let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.fneg)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(-1));
    }

    #[test]
    fn test_rand() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

        let result = execute(&mut graph, "MATCH (n:T) RETURN rand()").unwrap();
        match &result.rows[0].columns[0] {
            Value::Float(f) => {
                assert!(
                    *f >= 0.0 && *f < 1.0,
                    "rand() should be in [0.0, 1.0), got {}",
                    f
                );
            }
            other => panic!("Expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_isnan() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {f: 1.0, i: 5})").unwrap();

        // Normal float is not NaN
        let result = execute(&mut graph, "MATCH (n:T) RETURN isNaN(n.f)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        // Int is never NaN
        let result = execute(&mut graph, "MATCH (n:T) RETURN isNaN(n.i)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));
    }

    #[test]
    fn test_log_log10_sqrt() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            "CREATE (n:T {val: 100, fval: 2.718281828459045})",
        )
        .unwrap();

        // log(e) ≈ 1.0
        let result = execute(&mut graph, "MATCH (n:T) RETURN log(n.fval)").unwrap();
        match &result.rows[0].columns[0] {
            Value::Float(f) => assert!((f - 1.0).abs() < 0.0001),
            other => panic!("Expected Float, got {:?}", other),
        }

        // log10(100) = 2.0
        let result = execute(&mut graph, "MATCH (n:T) RETURN log10(n.val)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(2.0));

        // sqrt(100) = 10.0
        let result = execute(&mut graph, "MATCH (n:T) RETURN sqrt(n.val)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(10.0));
    }

    #[test]
    fn test_e_pi() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

        let result = execute(&mut graph, "MATCH (n:T) RETURN e()").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(std::f64::consts::E));

        let result = execute(&mut graph, "MATCH (n:T) RETURN pi()").unwrap();
        assert_eq!(
            result.rows[0].columns[0],
            Value::Float(std::f64::consts::PI)
        );
    }

    // ========== Metadata function tests ==========

    #[test]
    fn test_id() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[r:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // Node ID
        let result = execute(&mut graph, "MATCH (n:Person) RETURN id(n) ORDER BY id(n)").unwrap();
        assert_eq!(result.rows.len(), 2);
        // IDs should be non-negative integers
        match &result.rows[0].columns[0] {
            Value::Int(id) => assert!(*id >= 0),
            other => panic!("expected Int, got {:?}", other),
        }

        // Edge ID
        let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN id(r)").unwrap();
        assert_eq!(result.rows.len(), 1);
        match &result.rows[0].columns[0] {
            Value::Int(id) => assert!(*id >= 0),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn test_element_id() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[r:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // Node element ID
        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN elementId(n) ORDER BY id(n)",
        )
        .unwrap();
        match &result.rows[0].columns[0] {
            Value::String(s) => assert!(s.starts_with("node:")),
            other => panic!("expected String starting with 'node:', got {:?}", other),
        }

        // Edge element ID
        let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN elementId(r)").unwrap();
        match &result.rows[0].columns[0] {
            Value::String(s) => assert!(s.starts_with("edge:")),
            other => panic!("expected String starting with 'edge:', got {:?}", other),
        }
    }

    #[test]
    fn test_type_function() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person)-[:KNOWS]->(b:Person)"#).unwrap();

        let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN type(r)").unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("KNOWS".to_string())
        );
    }

    #[test]
    fn test_start_end_node() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // startNode
        let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN startNode(r)").unwrap();
        assert_eq!(result.rows.len(), 1);
        match &result.rows[0].columns[0] {
            Value::NodeData {
                labels, properties, ..
            } => {
                assert!(labels.contains(&"Person".to_string()));
                assert_eq!(
                    properties.get("name"),
                    Some(&PropertyValue::String("Alice".to_string()))
                );
            }
            other => panic!("expected NodeData, got {:?}", other),
        }

        // endNode
        let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN endNode(r)").unwrap();
        match &result.rows[0].columns[0] {
            Value::NodeData {
                labels, properties, ..
            } => {
                assert!(labels.contains(&"Person".to_string()));
                assert_eq!(
                    properties.get("name"),
                    Some(&PropertyValue::String("Bob".to_string()))
                );
            }
            other => panic!("expected NodeData, got {:?}", other),
        }
    }

    #[test]
    fn test_labels() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN labels(n)").unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::List(vec![Value::String("Person".to_string())])
        );
    }

    #[test]
    fn test_properties_keys() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        // properties - returns list of [key, value] pairs
        let result = execute(&mut graph, "MATCH (n:Person) RETURN properties(n)").unwrap();
        assert_eq!(result.rows.len(), 1);
        match &result.rows[0].columns[0] {
            Value::List(props) => {
                assert_eq!(props.len(), 2);
            }
            other => panic!("expected List, got {:?}", other),
        }

        // keys - returns list of key strings
        let result = execute(&mut graph, "MATCH (n:Person) RETURN keys(n)").unwrap();
        match &result.rows[0].columns[0] {
            Value::List(keys) => {
                assert_eq!(keys.len(), 2);
                // All should be strings
                for k in keys {
                    match k {
                        Value::String(_) => {}
                        other => panic!("expected String key, got {:?}", other),
                    }
                }
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    // ========== NULL handling function tests ==========

    #[test]
    fn test_coalesce() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: 42})"#).unwrap();

        // coalesce with non-null first value
        let result = execute(&mut graph, "MATCH (n:T) RETURN coalesce(n.val, 0)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(42));

        // coalesce with null first value - n.missing is null
        let result = execute(&mut graph, "MATCH (n:T) RETURN coalesce(n.missing, 99)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(99));

        // coalesce with all nulls
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN coalesce(n.missing, n.also_missing)",
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Null);
    }

    #[test]
    fn test_nullif() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {val: 42})"#).unwrap();

        // Equal values -> NULL
        let result = execute(&mut graph, "MATCH (n:T) RETURN nullIf(n.val, 42)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Null);

        // Different values -> first value
        let result = execute(&mut graph, "MATCH (n:T) RETURN nullIf(n.val, 99)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(42));
    }

    // ========== Type conversion function tests ==========

    #[test]
    fn test_type_conversions() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:T {s_true: "true", s_false: "false", s_int: "42", s_float: "3.14", i: 10, f: 2.5, b: true})"#,
        )
        .unwrap();

        // toBoolean
        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toBoolean(n.s_true)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toBoolean(n.s_false)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toBoolean(n.b)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // toFloat
        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toFloat(n.i)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(10.0));

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toFloat(n.s_float)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Float(3.14));

        // toInteger
        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toInteger(n.f)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(2));

        let result = execute(&mut graph, r#"MATCH (n:T) RETURN toInteger(n.s_int)"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Int(42));
    }

    // ========== Utility function tests ==========

    #[test]
    fn test_timestamp() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

        let result = execute(&mut graph, "MATCH (n:T) RETURN timestamp()").unwrap();
        match &result.rows[0].columns[0] {
            Value::Int(ts) => {
                // Should be a positive integer (Unix millis)
                assert!(*ts > 0);
                // Should be after year 2020 (1577836800000 millis)
                assert!(*ts > 1_577_836_800_000);
            }
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn test_random_uuid() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

        let result = execute(&mut graph, "MATCH (n:T) RETURN randomUUID()").unwrap();
        match &result.rows[0].columns[0] {
            Value::String(uuid) => {
                // UUID format: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
                assert_eq!(uuid.len(), 36);
                assert_eq!(&uuid[8..9], "-");
                assert_eq!(&uuid[13..14], "-");
                assert_eq!(&uuid[14..15], "4"); // version 4
                assert_eq!(&uuid[18..19], "-");
                assert_eq!(&uuid[23..24], "-");
            }
            other => panic!("expected String UUID, got {:?}", other),
        }
    }

    // ========== Task 41: List Operations ==========

    #[test]
    fn test_in_operator() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Item {val: 2})"#).unwrap();
        let result = execute(
            &mut graph,
            "MATCH (n:Item) WHERE n.val IN [1, 2, 3] RETURN n.val",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(2));

        let result2 = execute(
            &mut graph,
            "MATCH (n:Item) WHERE n.val IN [5, 6, 7] RETURN n.val",
        )
        .unwrap();
        assert_eq!(result2.row_count(), 0);
    }

    #[test]
    fn test_in_operator_null() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T {x: 1})").unwrap();
        let r = execute(&mut graph, "MATCH (n:T) WHERE 1 IN [1, 2] RETURN n.x").unwrap();
        assert_eq!(r.row_count(), 1);
    }

    #[test]
    fn test_list_concatenation() {
        let mut graph = Graph::new();
        let result = execute(&mut graph, "UNWIND [1, 2] + [3, 4] AS x RETURN x").unwrap();
        assert_eq!(result.row_count(), 4);
        assert_eq!(result.rows[0].columns[0], Value::Int(1));
        assert_eq!(result.rows[3].columns[0], Value::Int(4));
    }

    #[test]
    fn test_in_operator_multiple_nodes() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (a:Item {val: 200})").unwrap();
        execute(&mut graph, "CREATE (b:Item {val: 300})").unwrap();
        execute(&mut graph, "CREATE (c:Item {val: 999})").unwrap();
        let result = execute(
            &mut graph,
            "MATCH (n:Item) WHERE n.val IN [200, 300] RETURN n.val",
        )
        .unwrap();
        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_size_list() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(&mut graph, "MATCH (n:T) RETURN size([1, 2, 3])").unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(3));
    }

    #[test]
    fn test_reverse_list() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(&mut graph, "MATCH (n:T) RETURN reverse([1, 2, 3])").unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
        );
    }

    #[test]
    fn test_head_last_tail() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let r1 = execute(&mut graph, "MATCH (n:T) RETURN head([1, 2, 3])").unwrap();
        assert_eq!(r1.rows[0].columns[0], Value::Int(1));
        let r2 = execute(&mut graph, "MATCH (n:T) RETURN last([1, 2, 3])").unwrap();
        assert_eq!(r2.rows[0].columns[0], Value::Int(3));
        let r3 = execute(&mut graph, "MATCH (n:T) RETURN tail([1, 2, 3])").unwrap();
        assert_eq!(
            r3.rows[0].columns[0],
            Value::List(vec![Value::Int(2), Value::Int(3)])
        );
    }

    #[test]
    fn test_range() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let r1 = execute(&mut graph, "MATCH (n:T) RETURN range(1, 5)").unwrap();
        assert_eq!(
            r1.rows[0].columns[0],
            Value::List(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5)
            ])
        );
        let r2 = execute(&mut graph, "MATCH (n:T) RETURN range(1, 9, 2)").unwrap();
        assert_eq!(
            r2.rows[0].columns[0],
            Value::List(vec![
                Value::Int(1),
                Value::Int(3),
                Value::Int(5),
                Value::Int(7),
                Value::Int(9)
            ])
        );
    }

    #[test]
    fn test_reduce() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN reduce(acc = 0, x IN [1, 2, 3, 4, 5] | acc + x)",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(15));
    }

    #[test]
    fn test_list_comprehension() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN [x IN [1, 2, 3, 4, 5] WHERE x > 2 | x * 2]",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::List(vec![Value::Int(6), Value::Int(8), Value::Int(10)])
        );
    }

    // ========== Task 42: Extended Aggregation Functions ==========

    #[test]
    fn test_percentile_cont() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {age: 10})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {age: 20})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {age: 40})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {age: 50})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN percentileCont(n.age, 0.5)",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        // Median of [10,20,30,40,50] with linear interpolation = 30.0
        assert_eq!(result.rows[0].columns[0], Value::Float(30.0));
    }

    #[test]
    fn test_percentile_disc() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person2 {age: 10})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person2 {age: 20})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person2 {age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person2 {age: 40})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person2 {age: 50})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person2) RETURN percentileDisc(n.age, 0.5)",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        // Discrete median: ceil(0.5 * 5) - 1 = ceil(2.5) - 1 = 3 - 1 = 2 → value at index 2 = 30
        assert_eq!(result.rows[0].columns[0], Value::Int(30));
    }

    #[test]
    fn test_stdev() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Score {v: 2})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score {v: 4})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score {v: 4})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score {v: 4})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score {v: 5})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score {v: 5})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score {v: 7})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score {v: 9})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Score) RETURN stDev(n.v)").unwrap();
        assert_eq!(result.row_count(), 1);
        // Sample std dev of [2,4,4,4,5,5,7,9] = sqrt(32/7) ≈ 2.138
        if let Value::Float(v) = result.rows[0].columns[0] {
            assert!((v - 2.138).abs() < 0.01, "expected ~2.138, got {}", v);
        } else {
            panic!("expected Float, got {:?}", result.rows[0].columns[0]);
        }
    }

    #[test]
    fn test_stdevp() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 2})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 4})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 4})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 4})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 5})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 5})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 7})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Score2 {v: 9})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Score2) RETURN stDevP(n.v)").unwrap();
        assert_eq!(result.row_count(), 1);
        // Population std dev of [2,4,4,4,5,5,7,9] = sqrt(32/8) = 2.0
        if let Value::Float(v) = result.rows[0].columns[0] {
            assert!((v - 2.0).abs() < 0.01, "expected ~2.0, got {}", v);
        } else {
            panic!("expected Float, got {:?}", result.rows[0].columns[0]);
        }
    }

    #[test]
    fn test_count_distinct() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:City {name: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:City {name: "Osaka"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:City {name: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:City {name: "Kyoto"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:City {name: "Osaka"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:City) RETURN COUNT(DISTINCT n.name)").unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(3));
    }

    #[test]
    fn test_sum_distinct() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Val {v: 1})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Val {v: 2})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Val {v: 2})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Val {v: 3})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Val) RETURN SUM(DISTINCT n.v)").unwrap();
        assert_eq!(result.row_count(), 1);
        // Sum of distinct values: 1 + 2 + 3 = 6
        assert_eq!(result.rows[0].columns[0], Value::Int(6));
    }

    #[test]
    fn test_avg_distinct() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Val2 {v: 1})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Val2 {v: 2})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Val2 {v: 2})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Val2 {v: 3})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Val2) RETURN AVG(DISTINCT n.v)").unwrap();
        assert_eq!(result.row_count(), 1);
        // Avg of distinct values: (1+2+3)/3 = 2.0
        assert_eq!(result.rows[0].columns[0], Value::Float(2.0));
    }

    #[test]
    fn test_collect_distinct() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Tag {name: "rust"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Tag {name: "python"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Tag {name: "rust"})"#).unwrap();

        let result = execute(&mut graph, "MATCH (n:Tag) RETURN COLLECT(DISTINCT n.name)").unwrap();
        assert_eq!(result.row_count(), 1);
        // Should be a list with 2 distinct values
        if let Value::List(items) = &result.rows[0].columns[0] {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected List, got {:?}", result.rows[0].columns[0]);
        }
    }

    // ========== Task 44: Query Parameters ==========

    #[test]
    fn test_param_in_where() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

        let stmt = Parser::new("MATCH (n:Person) WHERE n.name = $name RETURN n.name")
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert("name".to_string(), Value::String("Alice".to_string()));

        let result = Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_param_in_create_property() {
        let mut graph = Graph::new();

        let stmt = Parser::new(r#"CREATE (n:Person {name: $name, age: $age})"#)
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert("name".to_string(), Value::String("Charlie".to_string()));
        params.insert("age".to_string(), Value::Int(35));

        Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(graph.node_count(), 1);
        let node = graph.nodes().next().unwrap();
        assert_eq!(
            node.get_property("name"),
            Some(&maharit_core::PropertyValue::String("Charlie".to_string()))
        );
        assert_eq!(
            node.get_property("age"),
            Some(&maharit_core::PropertyValue::Int(35))
        );
    }

    #[test]
    fn test_param_in_match_pattern() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:City {name: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:City {name: "Osaka"})"#).unwrap();

        let stmt = Parser::new(r#"MATCH (n:City {name: $city}) RETURN n.name"#)
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert("city".to_string(), Value::String("Tokyo".to_string()));

        let result = Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Tokyo".to_string())
        );
    }

    #[test]
    fn test_param_in_set() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Dave", age: 20})"#).unwrap();

        let stmt =
            Parser::new(r#"MATCH (n:Person {name: "Dave"}) SET n.age = $new_age RETURN n.age"#)
                .unwrap()
                .parse()
                .unwrap();

        let mut params = HashMap::new();
        params.insert("new_age".to_string(), Value::Int(21));

        let result = Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(21));
    }

    #[test]
    fn test_param_undefined_error() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:T {x: 1})"#).unwrap();

        let stmt = Parser::new("MATCH (n:T) WHERE n.x = $missing RETURN n")
            .unwrap()
            .parse()
            .unwrap();

        let result = Executor::new(&mut graph).execute_with_params(stmt, HashMap::new());
        // Should not panic, may return empty result or error
        // Undefined params during WHERE evaluation cause the filter to fail (row excluded)
        // because evaluate_expression returns Err which is treated as non-match
        // Just verify it doesn't panic
        let _ = result;
    }

    // ========== SKIP/LIMIT parameter tests ==========

    #[test]
    fn test_limit_with_param() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Carol"})"#).unwrap();

        let stmt = Parser::new("MATCH (n:Person) RETURN n.name LIMIT $count")
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert("count".to_string(), Value::Int(2));

        let result = Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_skip_with_param() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Carol"})"#).unwrap();

        let stmt = Parser::new("MATCH (n:Person) RETURN n.name SKIP $offset")
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert("offset".to_string(), Value::Int(2));

        let result = Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(result.row_count(), 1);
    }

    #[test]
    fn test_skip_and_limit_with_params() {
        let mut graph = Graph::new();
        for i in 1..=5i64 {
            execute(
                &mut graph,
                &format!(r#"CREATE (n:Item {{val: {}}})"#, i),
            )
            .unwrap();
        }

        let stmt =
            Parser::new("MATCH (n:Item) RETURN n.val ORDER BY n.val SKIP $offset LIMIT $count")
                .unwrap()
                .parse()
                .unwrap();

        let mut params = HashMap::new();
        params.insert("offset".to_string(), Value::Int(1));
        params.insert("count".to_string(), Value::Int(2));

        let result = Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        // Items: 1,2,3,4,5 -> skip 1 -> 2,3,4,5 -> limit 2 -> 2,3
        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::Int(2));
        assert_eq!(result.rows[1].columns[0], Value::Int(3));
    }

    #[test]
    fn test_limit_with_param_undefined_error() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

        let stmt = Parser::new("MATCH (n:Person) RETURN n.name LIMIT $missing")
            .unwrap()
            .parse()
            .unwrap();

        let result = Executor::new(&mut graph).execute_with_params(stmt, HashMap::new());
        // $missing is not defined: should return an error
        assert!(result.is_err());
    }

    // ========== Subquery tests ==========

    #[test]
    fn test_exists_subquery_true() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // Alice KNOWS Bob, so EXISTS should be true for Alice
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person {name: "Alice"}) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_exists_subquery_false() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"}), (b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // No KNOWS edges, EXISTS should filter out all results
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 0);
    }

    #[test]
    fn test_exists_subquery_filters_correctly() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

        // Only Alice has KNOWS relationship
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_count_subquery_basic() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (:Person {name: "Alice"})-[:KNOWS]->(:Person {name: "Carol"})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (:Person {name: "Dave"})"#).unwrap();

        // People with more than 0 KNOWS connections (Alice instances have 1 each)
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person) WHERE COUNT { MATCH (p)-[:KNOWS]->() } > 0 RETURN p.name"#,
        )
        .unwrap();

        // Alice (x2) should match, Dave should not
        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_count_subquery_returns_integer() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}), (c:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(c)"#,
        )
        .unwrap();

        // Return the count as a value
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person {name: "Alice"}) RETURN COUNT { MATCH (p)-[:KNOWS]->() }"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        // COUNT returns an integer value
        assert!(matches!(result.rows[0].columns[0], Value::Int(_)));
    }

    #[test]
    fn test_collect_subquery_basic() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"MATCH (a:Person {name: "Alice"}), (c:Person {name: "Bob"})
               CREATE (a)-[:KNOWS]->(:Person {name: "Carol"})"#,
        )
        .unwrap();

        // COLLECT subquery returns a list of friend names
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person {name: "Alice"})
               RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert!(matches!(result.rows[0].columns[0], Value::List(_)));
    }

    #[test]
    fn test_collect_subquery_empty() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (:Person {name: "Alice"})"#).unwrap();

        // COLLECT on empty pattern match returns empty list
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person {name: "Alice"})
               RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::List(vec![]));
    }

    #[test]
    fn test_call_subquery_basic() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // CALL subquery with WITH import: get friend count for each person
        // COUNT aggregation always produces a result row (even with 0 matches),
        // so both Alice and Bob appear in results
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person)
               CALL {
                 WITH p
                 MATCH (p)-[:KNOWS]->(f:Person)
                 RETURN COUNT(f) AS friend_count
               }
               RETURN p.name, friend_count"#,
        )
        .unwrap();

        assert_eq!(result.columns.len(), 2);
        // Both Person nodes appear since COUNT always returns a row
        assert_eq!(result.row_count(), 2);
        // Verify friend_count values exist
        for row in &result.rows {
            assert!(matches!(row.columns[1], Value::Int(_)));
        }
    }

    #[test]
    fn test_call_subquery_without_with() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (:Person {name: "Alice"}), (:Person {name: "Bob"})"#,
        )
        .unwrap();

        // CALL subquery without WITH: inner query is independent
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person)
               CALL {
                 MATCH (q:Person)
                 RETURN COUNT(q) AS total
               }
               RETURN p.name, total"#,
        )
        .unwrap();

        // Each outer person row gets combined with inner result
        assert!(result.row_count() >= 1);
    }

    #[test]
    fn test_exists_subquery_with_where() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice", city: "Tokyo"})-[:KNOWS]->(b:Person {name: "Bob", city: "Tokyo"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (c:Person {name: "Charlie", city: "Osaka"})-[:KNOWS]->(d:Person {name: "Dave", city: "Osaka"})"#,
        )
        .unwrap();

        // EXISTS with WHERE inside subquery
        let result = execute(
            &mut graph,
            r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(f:Person) WHERE f.city = "Tokyo" } RETURN p.name"#,
        )
        .unwrap();

        // Only Alice knows someone in Tokyo
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_count_subquery_return_value() {
        let mut graph = Graph::new();
        // Person with no friends
        execute(&mut graph, r#"CREATE (:Person {name: "Alice"})"#).unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (p:Person) RETURN p.name, COUNT { MATCH (p)-[:KNOWS]->() }"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[1], Value::Int(0));
    }

    // ========== FOREACH Tests ==========

    #[test]
    fn test_foreach_create_nodes() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"FOREACH (name IN ['Alice', 'Bob', 'Charlie'] | CREATE (:Person {name: name}))"#,
        )
        .unwrap();

        // Verify 3 nodes were created
        let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(result.row_count(), 3);
    }

    #[test]
    fn test_foreach_set_property() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (:Item {val: 1})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Item {val: 2})"#).unwrap();

        // Match all items and for each, set a flag using MATCH+FOREACH
        let result = execute(
            &mut graph,
            r#"MATCH (n:Item) FOREACH (x IN [1] | SET n.processed = true)"#,
        )
        .unwrap();
        assert_eq!(result.columns[0], "foreach_result");

        // Verify both items have the flag set
        let check = execute(
            &mut graph,
            r#"MATCH (n:Item) WHERE n.processed = true RETURN n"#,
        )
        .unwrap();
        assert_eq!(check.row_count(), 2);
    }

    #[test]
    fn test_foreach_nested_create() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"FOREACH (city IN ['Tokyo', 'Osaka'] |
              FOREACH (name IN ['Alice', 'Bob'] |
                CREATE (:Person {name: name, city: city})
              )
            )"#,
        )
        .unwrap();

        // Expect 2 cities * 2 names = 4 persons
        let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(result.row_count(), 4);
    }

    #[test]
    fn test_foreach_delete_nodes() {
        let mut graph = Graph::new();
        // Create some nodes with no edges so DELETE works
        execute(&mut graph, r#"CREATE (:Temp)"#).unwrap();
        execute(&mut graph, r#"CREATE (:Temp)"#).unwrap();

        let count_before = execute(&mut graph, "MATCH (n:Temp) RETURN n").unwrap();
        assert_eq!(count_before.row_count(), 2);

        // FOREACH DELETE using MATCH+FOREACH
        execute(
            &mut graph,
            r#"MATCH (n:Temp) FOREACH (x IN [1] | DETACH DELETE n)"#,
        )
        .unwrap();

        let count_after = execute(&mut graph, "MATCH (n:Temp) RETURN n").unwrap();
        assert_eq!(count_after.row_count(), 0);
    }

    #[test]
    fn test_match_foreach_set() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (:Node {active: true})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Node {active: true})"#).unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Node) FOREACH (x IN [1] | SET n.visited = true)"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Node) WHERE n.visited = true RETURN n"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_foreach_merge() {
        let mut graph = Graph::new();

        // FOREACH with MERGE - should create if not exists
        execute(
            &mut graph,
            r#"FOREACH (name IN ['Alice', 'Alice', 'Bob'] | MERGE (:Person {name: name}))"#,
        )
        .unwrap();

        // Alice should only exist once (MERGE deduplication)
        let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_foreach_multiple_clauses() {
        let mut graph = Graph::new();

        // FOREACH with multiple update clauses
        execute(
            &mut graph,
            r#"FOREACH (x IN [1] | CREATE (:A) CREATE (:B))"#,
        )
        .unwrap();

        let a = execute(&mut graph, "MATCH (n:A) RETURN n").unwrap();
        let b = execute(&mut graph, "MATCH (n:B) RETURN n").unwrap();
        assert_eq!(a.row_count(), 1);
        assert_eq!(b.row_count(), 1);
    }

    #[test]
    fn test_foreach_empty_list() {
        let mut graph = Graph::new();

        // FOREACH with empty list should create no nodes
        execute(
            &mut graph,
            r#"FOREACH (name IN [] | CREATE (:Person {name: name}))"#,
        )
        .unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(result.row_count(), 0);
    }

    // ---- Task 39: Predicate Functions ----

    #[test]
    fn test_predicate_all() {
        let mut graph = Graph::new();
        // Create a sentinel node to drive MATCH
        execute(&mut graph, "CREATE (:T)").unwrap();

        // all(x IN [1,2,3] WHERE x > 0) → true (all positive)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN all(x IN [1, 2, 3] WHERE x > 0)",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // all(x IN [1,2,3] WHERE x > 1) → false (1 does not satisfy)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN all(x IN [1, 2, 3] WHERE x > 1)",
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        // all() on empty list → true (vacuously true)
        let result = execute(&mut graph, "MATCH (n:T) RETURN all(x IN [] WHERE x > 0)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));
    }

    #[test]
    fn test_predicate_any() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (:T)").unwrap();

        // any(x IN [1,2,3] WHERE x > 2) → true (3 satisfies)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN any(x IN [1, 2, 3] WHERE x > 2)",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // any(x IN [1,2,3] WHERE x > 5) → false (none satisfy)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN any(x IN [1, 2, 3] WHERE x > 5)",
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        // any() on empty list → false
        let result = execute(&mut graph, "MATCH (n:T) RETURN any(x IN [] WHERE x > 0)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));
    }

    #[test]
    fn test_predicate_none() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (:T)").unwrap();

        // none(x IN [1,2,3] WHERE x > 5) → true (no element satisfies)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN none(x IN [1, 2, 3] WHERE x > 5)",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // none(x IN [1,2,3] WHERE x > 0) → false (all satisfy)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN none(x IN [1, 2, 3] WHERE x > 0)",
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        // none() on empty list → true
        let result = execute(&mut graph, "MATCH (n:T) RETURN none(x IN [] WHERE x > 0)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));
    }

    #[test]
    fn test_predicate_single() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (:T)").unwrap();

        // single(x IN [1,2,3] WHERE x = 2) → true (exactly one)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN single(x IN [1, 2, 3] WHERE x = 2)",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // single(x IN [1,2,3] WHERE x > 0) → false (all satisfy, not exactly one)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN single(x IN [1, 2, 3] WHERE x > 0)",
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        // single() on empty list → false
        let result = execute(&mut graph, "MATCH (n:T) RETURN single(x IN [] WHERE x = 1)").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        // single(x IN [1,2,3] WHERE x > 5) → false (zero satisfy)
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN single(x IN [1, 2, 3] WHERE x > 5)",
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));
    }

    #[test]
    fn test_exists_function() {
        let mut graph = Graph::new();
        // Create a node with email property
        execute(
            &mut graph,
            r#"CREATE (:Person {name: "Alice", email: "alice@example.com"})"#,
        )
        .unwrap();
        // Create a node without email property
        execute(&mut graph, r#"CREATE (:Person {name: "Bob"})"#).unwrap();

        // nodes with email should be found by exists(n.email)
        let result = execute(
            &mut graph,
            "MATCH (n:Person) WHERE exists(n.email) RETURN n.name",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );

        // exists() in RETURN context: Alice has email → true
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Alice"}) RETURN exists(n.email)"#,
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // Bob has no email → false
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person {name: "Bob"}) RETURN exists(n.email)"#,
        )
        .unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));
    }

    #[test]
    fn test_is_empty_function() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (:T)").unwrap();

        // isEmpty on empty list → true
        let result = execute(&mut graph, "MATCH (n:T) RETURN isEmpty([])").unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // isEmpty on non-empty list → false
        let result = execute(&mut graph, "MATCH (n:T) RETURN isEmpty([1, 2, 3])").unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));

        // isEmpty on empty string → true
        let result = execute(&mut graph, r#"MATCH (n:T) RETURN isEmpty("")"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(true));

        // isEmpty on non-empty string → false
        let result = execute(&mut graph, r#"MATCH (n:T) RETURN isEmpty("hello")"#).unwrap();
        assert_eq!(result.rows[0].columns[0], Value::Bool(false));
    }

    #[test]
    fn test_predicate_in_where_clause() {
        let mut graph = Graph::new();
        // Use list predicate in a WHERE clause with MATCH
        execute(&mut graph, r#"CREATE (:Item {val: 5})"#).unwrap();
        execute(&mut graph, r#"CREATE (:Item {val: 15})"#).unwrap();

        // Find items where val equals any number in [4, 5, 6]
        let result = execute(
            &mut graph,
            "MATCH (n:Item) WHERE any(x IN [4, 5, 6] WHERE x = n.val) RETURN n.val",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(5));
    }

    // ========== Multiple Labels Tests ==========

    #[test]
    fn test_create_node_with_multiple_labels() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person:Employee {name: "Alice"})"#,
        )
        .unwrap();

        assert_eq!(graph.node_count(), 1);
        let node = graph.nodes().next().unwrap();
        assert!(node.has_label("Person"), "Should have Person label");
        assert!(node.has_label("Employee"), "Should have Employee label");
        assert_eq!(node.labels.len(), 2);
    }

    #[test]
    fn test_match_multiple_labels_and_condition() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person:Employee {name: "Alice"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // Only Alice has both Person AND Employee
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person:Employee) RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_match_single_label_on_multilabel_node() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person:Employee {name: "Alice"})"#,
        )
        .unwrap();

        // Node with multiple labels should match on any single label
        let result1 =
            execute(&mut graph, r#"MATCH (n:Person) RETURN n.name"#).unwrap();
        assert_eq!(result1.row_count(), 1);

        let result2 =
            execute(&mut graph, r#"MATCH (n:Employee) RETURN n.name"#).unwrap();
        assert_eq!(result2.row_count(), 1);
    }

    #[test]
    fn test_set_label_on_multilabel_node() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice"})"#,
        )
        .unwrap();

        // Add two labels sequentially
        execute(&mut graph, r#"MATCH (n:Person) SET n:Employee"#).unwrap();
        execute(&mut graph, r#"MATCH (n:Person) SET n:Manager"#).unwrap();

        let node = graph.nodes().next().unwrap();
        assert!(node.has_label("Person"));
        assert!(node.has_label("Employee"));
        assert!(node.has_label("Manager"));
        assert_eq!(node.labels.len(), 3);
    }

    #[test]
    fn test_remove_label_from_multilabel_node() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person:Employee:Manager {name: "Alice"})"#,
        )
        .unwrap();

        execute(
            &mut graph,
            r#"MATCH (n:Manager) REMOVE n:Manager"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        assert!(node.has_label("Person"), "Should still have Person");
        assert!(node.has_label("Employee"), "Should still have Employee");
        assert!(!node.has_label("Manager"), "Manager should be removed");
        assert_eq!(node.labels.len(), 2);
    }

    #[test]
    fn test_labels_function_multiple_labels() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person:Employee {name: "Alice"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) RETURN labels(n)"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        match &result.rows[0].columns[0] {
            Value::List(labels) => {
                assert!(labels.contains(&Value::String("Person".to_string())));
                assert!(labels.contains(&Value::String("Employee".to_string())));
                assert_eq!(labels.len(), 2);
            }
            other => panic!("Expected list of labels, got {:?}", other),
        }
    }

    #[test]
    fn test_create_match_three_labels() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:A:B:C {val: 42})"#,
        )
        .unwrap();

        let node = graph.nodes().next().unwrap();
        assert!(node.has_label("A"));
        assert!(node.has_label("B"));
        assert!(node.has_label("C"));
        assert_eq!(node.labels.len(), 3);

        // Matching with all three labels should work
        let result = execute(&mut graph, r#"MATCH (n:A:B:C) RETURN n.val"#).unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(42));

        // Matching with only two labels should also work
        let result2 = execute(&mut graph, r#"MATCH (n:A:B) RETURN n.val"#).unwrap();
        assert_eq!(result2.row_count(), 1);

        // Matching with a non-existent combo should return nothing
        let result3 = execute(&mut graph, r#"MATCH (n:A:D) RETURN n.val"#).unwrap();
        assert_eq!(result3.row_count(), 0);
    }

    #[test]
    fn test_pattern_predicate_basic() {
        let mut graph = Graph::new();
        // Alice knows Bob, Charlie knows nobody
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

        // WHERE (n)-[:KNOWS]->() should return only Alice
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE (n)-[:KNOWS]->() RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_pattern_predicate_not() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();
        execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

        // WHERE NOT (n)-[:KNOWS]->() should return Bob and Charlie (no outgoing KNOWS)
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE NOT (n)-[:KNOWS]->() RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_pattern_predicate_with_bound_variable() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
        )
        .unwrap();

        // WHERE (a)-[:KNOWS]->(b) with both a and b bound
        let result = execute(
            &mut graph,
            r#"MATCH (a:Person), (b:Person) WHERE (a)-[:KNOWS]->(b) RETURN a.name, b.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
        assert_eq!(
            result.rows[0].columns[1],
            Value::String("Bob".to_string())
        );
    }

    #[test]
    fn test_pattern_predicate_with_props() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (a:Person {name: "Alice"})-[:WORKS_AT]->(c:Company {name: "ACME"})"#,
        )
        .unwrap();
        execute(
            &mut graph,
            r#"CREATE (b:Person {name: "Bob"})-[:WORKS_AT]->(c2:Company {name: "Other"})"#,
        )
        .unwrap();

        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE (n)-[:WORKS_AT]->(:Company {name: "ACME"}) RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_pattern_predicate_label_only() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (b:Robot {name: "R2D2"})"#).unwrap();

        // WHERE (n:Person) as standalone pattern predicate
        let result = execute(
            &mut graph,
            r#"MATCH (n) WHERE (n:Person) RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(
            result.rows[0].columns[0],
            Value::String("Alice".to_string())
        );
    }

    // ========== Lazy binding (task 59) ==========

    #[test]
    fn test_lazy_binding_limit_early_termination() {
        // Verify that LIMIT N without ORDER BY terminates early:
        // only N rows should be returned even if more exist.
        let mut graph = Graph::new();
        for i in 0..10 {
            let id = graph.create_node("Item");
            graph
                .get_node_mut(id)
                .unwrap()
                .set_property("n", PropertyValue::Int(i));
        }

        let result = execute(&mut graph, "MATCH (n:Item) RETURN n.n LIMIT 3").unwrap();
        assert_eq!(result.row_count(), 3);
    }

    #[test]
    fn test_lazy_binding_multi_pattern_correctness() {
        // Multi-pattern MATCH should return correct results with lazy per-binding expansion.
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        graph
            .get_node_mut(a)
            .unwrap()
            .set_property("v", PropertyValue::Int(1));
        let b = graph.create_node("B");
        graph
            .get_node_mut(b)
            .unwrap()
            .set_property("v", PropertyValue::Int(2));
        graph.create_edge(a, b, "TO").unwrap();

        // Two patterns: node pattern + path pattern
        let result = execute(
            &mut graph,
            "MATCH (a:A), (a)-[:TO]->(b:B) RETURN a.v, b.v",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(1));
        assert_eq!(result.rows[0].columns[1], Value::Int(2));
    }

    #[test]
    fn test_lazy_binding_limit_with_aggregation_unchanged() {
        // Aggregation must still collect all rows (no early cutoff).
        let mut graph = Graph::new();
        for i in 0..5 {
            let id = graph.create_node("N");
            graph
                .get_node_mut(id)
                .unwrap()
                .set_property("v", PropertyValue::Int(i));
        }

        let result = execute(&mut graph, "MATCH (n:N) RETURN count(n) LIMIT 1").unwrap();
        // count must be 5, not 1 (limit does not truncate before aggregation)
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(5));
    }

    // ========== Task 50: Temporal Types ==========

    #[test]
    fn test_date_func_no_arg() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(&mut graph, "MATCH (n:T) RETURN date()").unwrap();
        match &result.rows[0].columns[0] {
            Value::Date(d) => assert!(*d > 0, "date should be after epoch"),
            other => panic!("expected Date, got {:?}", other),
        }
    }

    #[test]
    fn test_date_func_string_arg() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(&mut graph, "MATCH (n:T) RETURN date(\"2024-01-15\")").unwrap();
        match &result.rows[0].columns[0] {
            Value::Date(d) => {
                let (y, m, day) = maharit_core::temporal::days_to_ymd(*d);
                assert_eq!(y, 2024);
                assert_eq!(m, 1);
                assert_eq!(day, 15);
            }
            other => panic!("expected Date, got {:?}", other),
        }
    }

    #[test]
    fn test_datetime_func_no_arg() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(&mut graph, "MATCH (n:T) RETURN datetime()").unwrap();
        match &result.rows[0].columns[0] {
            Value::DateTime(ms) => assert!(*ms > 0),
            other => panic!("expected DateTime, got {:?}", other),
        }
    }

    #[test]
    fn test_datetime_func_string_arg() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN datetime(\"2024-06-15T12:30:00Z\")",
        )
        .unwrap();
        match &result.rows[0].columns[0] {
            Value::DateTime(ms) => {
                let (y, mo, d, h, mi, s, _) = maharit_core::temporal::millis_to_datetime(*ms);
                assert_eq!(y, 2024);
                assert_eq!(mo, 6);
                assert_eq!(d, 15);
                assert_eq!(h, 12);
                assert_eq!(mi, 30);
                assert_eq!(s, 0);
            }
            other => panic!("expected DateTime, got {:?}", other),
        }
    }

    #[test]
    fn test_duration_func_string_arg() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(&mut graph, "MATCH (n:T) RETURN duration(\"P1Y2M3D\")").unwrap();
        match &result.rows[0].columns[0] {
            Value::Duration { months, days, millis } => {
                assert_eq!(*months, 14); // 1*12 + 2
                assert_eq!(*days, 3);
                assert_eq!(*millis, 0);
            }
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    #[test]
    fn test_duration_with_time() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(&mut graph, "MATCH (n:T) RETURN duration(\"PT2H30M\")").unwrap();
        match &result.rows[0].columns[0] {
            Value::Duration { months, days, millis } => {
                assert_eq!(*months, 0);
                assert_eq!(*days, 0);
                assert_eq!(*millis, 2 * 3_600_000 + 30 * 60_000);
            }
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    #[test]
    fn test_date_comparison() {
        let mut graph = Graph::new();
        let nid = graph.create_node("Event");
        graph
            .get_node_mut(nid)
            .unwrap()
            .set_property("date", PropertyValue::Date(
                maharit_core::temporal::ymd_to_days(2024, 6, 15)
            ));
        // Should match: 2024-06-15 >= 2024-01-01
        let result = execute(
            &mut graph,
            "MATCH (e:Event) WHERE e.date >= date(\"2024-01-01\") RETURN e.date",
        )
        .unwrap();
        assert_eq!(result.row_count(), 1);
        // Should not match: 2024-06-15 >= 2025-01-01
        let result2 = execute(
            &mut graph,
            "MATCH (e:Event) WHERE e.date >= date(\"2025-01-01\") RETURN e.date",
        )
        .unwrap();
        assert_eq!(result2.row_count(), 0);
    }

    #[test]
    fn test_date_arithmetic() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        // date("2024-01-01") + duration("P1D") should be 2024-01-02
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN date(\"2024-01-01\") + duration(\"P1D\")",
        )
        .unwrap();
        match &result.rows[0].columns[0] {
            Value::Date(d) => {
                let (y, m, day) = maharit_core::temporal::days_to_ymd(*d);
                assert_eq!(y, 2024);
                assert_eq!(m, 1);
                assert_eq!(day, 2);
            }
            other => panic!("expected Date, got {:?}", other),
        }
    }

    #[test]
    fn test_date_subtraction() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        // date("2024-01-10") - date("2024-01-01") = duration of 9 days
        let result = execute(
            &mut graph,
            "MATCH (n:T) RETURN date(\"2024-01-10\") - date(\"2024-01-01\")",
        )
        .unwrap();
        match &result.rows[0].columns[0] {
            Value::Duration { months: 0, days: 9, millis: 0 } => {}
            other => panic!("expected Duration{{days:9}}, got {:?}", other),
        }
    }

    #[test]
    fn test_temporal_create_and_persist() {
        let mut graph = Graph::new();
        let days = maharit_core::temporal::ymd_to_days(2024, 3, 15);
        let nid = graph.create_node("Event");
        graph
            .get_node_mut(nid)
            .unwrap()
            .set_property("date", PropertyValue::Date(days));
        let node = graph.get_node(nid).unwrap();
        assert_eq!(node.get_property("date"), Some(&PropertyValue::Date(days)));
    }

    #[test]
    fn test_date_field_accessors() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:T) WITH date("2024-06-15") AS d RETURN d.year, d.month, d.day"#,
        ).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(2024));
        assert_eq!(result.rows[0].columns[1], Value::Int(6));
        assert_eq!(result.rows[0].columns[2], Value::Int(15));
    }

    #[test]
    fn test_datetime_field_accessors() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:T) WITH datetime("2024-06-15T10:30:45Z") AS dt RETURN dt.year, dt.hour, dt.minute, dt.second"#,
        ).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(2024));
        assert_eq!(result.rows[0].columns[1], Value::Int(10));
        assert_eq!(result.rows[0].columns[2], Value::Int(30));
        assert_eq!(result.rows[0].columns[3], Value::Int(45));
    }

    #[test]
    fn test_duration_field_accessors() {
        // P1Y2M3DT4H5M6S = 1 year, 2 months, 3 days, 4 hours, 5 minutes, 6 seconds
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:T) WITH duration("P1Y2M3DT4H5M6S") AS dur RETURN dur.years, dur.months, dur.days, dur.hours, dur.minutes, dur.seconds"#,
        ).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(1));  // years
        assert_eq!(result.rows[0].columns[1], Value::Int(2));  // months (within year)
        assert_eq!(result.rows[0].columns[2], Value::Int(3));  // days
        assert_eq!(result.rows[0].columns[3], Value::Int(4));  // hours
        assert_eq!(result.rows[0].columns[4], Value::Int(5));  // minutes
        assert_eq!(result.rows[0].columns[5], Value::Int(6));  // seconds
    }

    #[test]
    fn test_date_from_map() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            "MATCH (n:T) WITH date({year: 2024, month: 3, day: 7}) AS d RETURN d.year, d.month, d.day",
        ).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(2024));
        assert_eq!(result.rows[0].columns[1], Value::Int(3));
        assert_eq!(result.rows[0].columns[2], Value::Int(7));
    }

    #[test]
    fn test_duration_from_map() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:T)").unwrap();
        let result = execute(
            &mut graph,
            "MATCH (n:T) WITH duration({years: 1, months: 2, days: 3, hours: 4, minutes: 5, seconds: 6}) AS dur RETURN dur.years, dur.months, dur.days, dur.hours",
        ).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(1));
        assert_eq!(result.rows[0].columns[1], Value::Int(2));
        assert_eq!(result.rows[0].columns[2], Value::Int(3));
        assert_eq!(result.rows[0].columns[3], Value::Int(4));
    }

    #[test]
    fn test_date_field_in_where_clause() {
        let mut graph = Graph::new();
        let nid = graph.create_node_with_labels(vec!["Event".to_string()]);
        let days = maharit_core::temporal::ymd_to_days(2024, 6, 15);
        graph.get_node_mut(nid).unwrap().set_property("date", PropertyValue::Date(days));
        // MATCH event, carry date through WITH, filter on year, return month
        let result = execute(
            &mut graph,
            r#"MATCH (e:Event) WITH e.date AS d WHERE d.year = 2024 RETURN d.month"#,
        ).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(6));
    }

    // ========== PROPERTY INDEX TESTS ==========

    #[test]
    fn test_create_index_and_lookup() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        // Create index
        let stmt = Parser::new("CREATE INDEX ON :Person(name)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();

        // Create some nodes
        let stmt = Parser::new("CREATE (n:Person {name: 'Alice', age: 30})").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();
        let stmt = Parser::new("CREATE (n:Person {name: 'Bob', age: 25})").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();
        let stmt = Parser::new("CREATE (n:Person {name: 'Alice', age: 35})").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();

        // Re-create index to pick up existing nodes
        let stmt = Parser::new("DROP INDEX ON :Person(name)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();
        let stmt = Parser::new("CREATE INDEX ON :Person(name)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();

        // Query using indexed property
        let stmt = Parser::new("MATCH (n:Person {name: 'Alice'}) RETURN n.name").unwrap().parse().unwrap();
        let rs = executor.execute(stmt).unwrap();
        assert_eq!(rs.row_count(), 2);
    }

    #[test]
    fn test_show_indexes() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        let stmt = Parser::new("CREATE INDEX ON :Person(name)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();
        let stmt = Parser::new("CREATE INDEX ON :Employee(email)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();

        let stmt = Parser::new("SHOW INDEXES").unwrap().parse().unwrap();
        let result = executor.execute(stmt).unwrap();
        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_drop_index() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        let stmt = Parser::new("CREATE INDEX ON :Person(name)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();
        let stmt = Parser::new("DROP INDEX ON :Person(name)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();

        let stmt = Parser::new("SHOW INDEXES").unwrap().parse().unwrap();
        let result = executor.execute(stmt).unwrap();
        assert_eq!(result.row_count(), 0);
    }

    #[test]
    fn test_index_auto_update_on_create() {
        let mut graph = Graph::new();
        let mut executor = Executor::new(&mut graph);

        // Create index first
        let stmt = Parser::new("CREATE INDEX ON :Person(name)").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();

        // Create nodes - they should be automatically indexed
        let stmt = Parser::new("CREATE (n:Person {name: 'Alice'})").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();
        let stmt = Parser::new("CREATE (n:Person {name: 'Bob'})").unwrap().parse().unwrap();
        executor.execute(stmt).unwrap();

        // Verify index has the entries
        let alices = executor.property_index().find_by_property(
            "name",
            &PropertyValue::String("Alice".to_string()),
        );
        assert_eq!(alices.len(), 1);
    }

    #[test]
    fn test_count_node_optimization() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:Person {name: 'Alice'})").unwrap();
        execute(&mut graph, "CREATE (n:Person {name: 'Bob'})").unwrap();
        execute(&mut graph, "CREATE (n:Person {name: 'Carol'})").unwrap();
        let result = execute(&mut graph, "MATCH (n:Person) RETURN count(n)").unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(3));
    }

    #[test]
    fn test_group_by_count() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:Person {city: 'Tokyo'})").unwrap();
        execute(&mut graph, "CREATE (n:Person {city: 'Tokyo'})").unwrap();
        execute(&mut graph, "CREATE (n:Person {city: 'Osaka'})").unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN n.city, count(n)").unwrap();
        assert_eq!(result.row_count(), 2); // Two distinct cities
        assert_eq!(result.columns, vec!["n.city", "count(n)"]);
    }

    #[test]
    fn test_group_by_avg() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:Person {dept: 'Eng', salary: 100})").unwrap();
        execute(&mut graph, "CREATE (n:Person {dept: 'Eng', salary: 200})").unwrap();
        execute(&mut graph, "CREATE (n:Person {dept: 'Sales', salary: 150})").unwrap();

        let result = execute(&mut graph, "MATCH (n:Person) RETURN n.dept, avg(n.salary)").unwrap();
        assert_eq!(result.row_count(), 2);

        // Find the Eng row and verify avg = 150.0
        let eng_row = result
            .rows
            .iter()
            .find(|r| r.columns[0] == Value::String("Eng".to_string()));
        assert!(eng_row.is_some());
        if let Some(row) = eng_row {
            assert_eq!(row.columns[1], Value::Float(150.0));
        }
    }

    #[test]
    fn test_simple_count_star() {
        let mut graph = Graph::new();
        execute(&mut graph, "CREATE (n:A)").unwrap();
        execute(&mut graph, "CREATE (n:A)").unwrap();
        let result = execute(&mut graph, "MATCH (n:A) RETURN count(*)").unwrap();
        assert_eq!(result.row_count(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(2));
    }

    // ========== UNWIND + CREATE batch write tests (task #70) ==========

    #[test]
    fn test_unwind_create_from_map_params() {
        // UNWIND $nodes AS n CREATE (:Person {id: n.id, name: n.name}) でノードを一括作成
        let mut graph = Graph::new();
        let stmt = Parser::new("UNWIND $nodes AS n CREATE (:Person {id: n.id, name: n.name})")
            .unwrap()
            .parse()
            .unwrap();

        let nodes_list = Value::List(vec![
            Value::Map({
                let mut m = HashMap::new();
                m.insert("id".to_string(), Value::Int(1));
                m.insert("name".to_string(), Value::String("Alice".to_string()));
                m
            }),
            Value::Map({
                let mut m = HashMap::new();
                m.insert("id".to_string(), Value::Int(2));
                m.insert("name".to_string(), Value::String("Bob".to_string()));
                m
            }),
            Value::Map({
                let mut m = HashMap::new();
                m.insert("id".to_string(), Value::Int(3));
                m.insert("name".to_string(), Value::String("Carol".to_string()));
                m
            }),
        ]);

        let mut params = HashMap::new();
        params.insert("nodes".to_string(), nodes_list);

        Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(graph.node_count(), 3);

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.id, n.name ORDER BY n.id",
        )
        .unwrap();
        assert_eq!(result.row_count(), 3);
        assert_eq!(result.rows[0].columns[0], Value::Int(1));
        assert_eq!(result.rows[0].columns[1], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::Int(2));
        assert_eq!(result.rows[2].columns[0], Value::Int(3));
    }

    #[test]
    fn test_unwind_create_bulk_1000_nodes() {
        // 1000件のノードを UNWIND+CREATE で一括作成できることを確認
        let mut graph = Graph::new();

        let items: Vec<Value> = (1..=1000)
            .map(|i| {
                Value::Map({
                    let mut m = HashMap::new();
                    m.insert("id".to_string(), Value::Int(i));
                    m
                })
            })
            .collect();

        let stmt = Parser::new("UNWIND $items AS item CREATE (:Bulk {id: item.id})")
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert("items".to_string(), Value::List(items));

        Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(graph.node_count(), 1000);
    }

    #[test]
    fn test_unwind_create_scalar_list() {
        // スカラーリストの UNWIND+CREATE（n.prop アクセスなし）
        let mut graph = Graph::new();
        let stmt = Parser::new("UNWIND $vals AS v CREATE (:Tag {value: v})")
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert(
            "vals".to_string(),
            Value::List(vec![
                Value::String("alpha".to_string()),
                Value::String("beta".to_string()),
            ]),
        );

        Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn test_unwind_map_property_access_null_for_missing_key() {
        // マップに存在しないキーへのアクセスは Null を返す（エラーにならない）
        let mut graph = Graph::new();
        let stmt = Parser::new("UNWIND $nodes AS n CREATE (:X {a: n.a, b: n.b})")
            .unwrap()
            .parse()
            .unwrap();

        let mut params = HashMap::new();
        params.insert(
            "nodes".to_string(),
            Value::List(vec![Value::Map({
                let mut m = HashMap::new();
                m.insert("a".to_string(), Value::Int(1));
                // "b" key is absent
                m
            })]),
        );

        Executor::new(&mut graph)
            .execute_with_params(stmt, params)
            .unwrap();

        assert_eq!(graph.node_count(), 1);
        let node = graph.nodes().next().unwrap();
        assert_eq!(node.get_property("a"), Some(&PropertyValue::Int(1)));
        // missing key evaluates to Null; Null properties may be stored as PropertyValue::Null
        let b = node.get_property("b");
        assert!(b.is_none() || b == Some(&PropertyValue::Null));
    }

    #[test]
    fn test_unwind_inline_json_map_create() {
        // benchmark.py の bench_unwind_batch_create が生成するクエリ形式 (Task 88)
        // json.dumps で {"key": value} 形式になるインラインマップリテラル + CREATE
        let mut graph = Graph::new();
        let result = execute(
            &mut graph,
            r#"UNWIND [{"id": 0, "name": "Alice0", "city": "Tokyo"}, {"id": 1, "name": "Bob1", "city": "Osaka"}] AS item CREATE (:UnwindBench {id: item.id, name: item.name, city: item.city})"#,
        );
        assert!(result.is_ok(), "UNWIND with inline JSON map literal failed: {:?}", result);
        assert_eq!(graph.node_count(), 2);

        let result = execute(
            &mut graph,
            "MATCH (n:UnwindBench) RETURN n.id, n.name, n.city ORDER BY n.id",
        ).unwrap();
        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::Int(0));
        assert_eq!(result.rows[0].columns[1], Value::String("Alice0".to_string()));
        assert_eq!(result.rows[0].columns[2], Value::String("Tokyo".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::Int(1));
        assert_eq!(result.rows[1].columns[1], Value::String("Bob1".to_string()));
        assert_eq!(result.rows[1].columns[2], Value::String("Osaka".to_string()));
    }

    // ========== Standalone RETURN tests (Task 89) ==========

    #[test]
    fn test_execute_standalone_return_literal() {
        let mut graph = Graph::new();
        let stmt = Parser::new("RETURN 1 + 1 AS result")
            .unwrap()
            .parse()
            .unwrap();
        let result = Executor::new(&mut graph).execute(stmt).unwrap();
        assert_eq!(result.columns, vec!["result"]);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(2));
    }

    #[test]
    fn test_execute_standalone_return_string() {
        let mut graph = Graph::new();
        let stmt = Parser::new(r#"RETURN 'hello' AS greeting"#)
            .unwrap()
            .parse()
            .unwrap();
        let result = Executor::new(&mut graph).execute(stmt).unwrap();
        assert_eq!(result.columns, vec!["greeting"]);
        assert_eq!(result.rows[0].columns[0], Value::String("hello".to_string()));
    }

    #[test]
    fn test_execute_standalone_return_multiple() {
        let mut graph = Graph::new();
        let stmt = Parser::new("RETURN 1 AS a, 2 AS b")
            .unwrap()
            .parse()
            .unwrap();
        let result = Executor::new(&mut graph).execute(stmt).unwrap();
        assert_eq!(result.columns, vec!["a", "b"]);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::Int(1));
        assert_eq!(result.rows[0].columns[1], Value::Int(2));
    }
}
