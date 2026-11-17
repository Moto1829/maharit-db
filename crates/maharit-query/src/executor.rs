use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use maharit_core::{
    ConcurrentGraph, Constraint, ConstraintError, ConstraintManager, ConstraintType, Edge,
    FulltextError, FulltextManager, Graph, GraphBackend, IndexDefinition, NodeId, PropertyIndex,
    PropertyType, PropertyValue, traversal,
};
use rayon::prelude::*;
use regex::Regex;

use crate::ast::*;

mod algo;
mod bindings;
mod error;
mod value;

use crate::cache::AstCache;
pub use bindings::BindingValue;
use bindings::Bindings;
pub use error::ExecuteError;
pub use value::{ResultSet, Row, Value};

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
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// `range()` が一度に生成できる最大要素数。
///
/// 上限がないと `range(0, i64::MAX)` のようなクエリでメモリを枯渇させたり、
/// 加算のオーバーフローで無限ループに陥らせる DoS が可能になるため、
/// リスト確保前にこの値を超える範囲を拒否する。
const MAX_RANGE_LEN: usize = 10_000_000;

/// Prefix of hidden variables given to anonymous nodes while matching a path.
/// It contains a NUL character, so no query can refer to such a variable.
const ANON_VAR_PREFIX: &str = "\u{0}anon";

/// 共有マネージャ（制約・全文検索・プロパティ索引）の保持形態。
///
/// サーバーはクエリ毎にマネージャを deep clone せず、読み取りクエリには共有参照、
/// 書き込みクエリには排他参照を渡す。`Shared` に対する可変アクセスは
/// copy-on-write で `Owned` に昇格するため、万一読み取り経路で更新が起きても
/// 共有状態は壊れない（その更新は当該 Executor 内に閉じる）。
enum ManagerSlot<'a, T: Clone> {
    Owned(T),
    Shared(&'a T),
    Exclusive(&'a mut T),
}

impl<T: Clone> ManagerSlot<'_, T> {
    fn into_owned(self) -> T {
        match self {
            ManagerSlot::Owned(t) => t,
            ManagerSlot::Shared(t) => t.clone(),
            ManagerSlot::Exclusive(t) => t.clone(),
        }
    }
}

impl<T: Clone> std::ops::Deref for ManagerSlot<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            ManagerSlot::Owned(t) => t,
            ManagerSlot::Shared(t) => t,
            ManagerSlot::Exclusive(t) => t,
        }
    }
}

impl<T: Clone> std::ops::DerefMut for ManagerSlot<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        if let ManagerSlot::Shared(t) = self {
            *self = ManagerSlot::Owned((*t).clone());
        }
        match self {
            ManagerSlot::Owned(t) => t,
            ManagerSlot::Exclusive(t) => t,
            ManagerSlot::Shared(_) => unreachable!(),
        }
    }
}

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
    graph: *mut (dyn GraphBackend + 'a),
    readonly: bool,
    _marker: std::marker::PhantomData<&'a ()>,
    constraints: ManagerSlot<'a, ConstraintManager>,
    fulltext: ManagerSlot<'a, FulltextManager>,
    property_index: ManagerSlot<'a, PropertyIndex>,
    params: HashMap<String, Value>,
    /// 現在処理中セグメントの WHERE から抽出した範囲述語のヒント
    /// `(変数名, プロパティ名, 演算子, 比較値)`。索引がある数値プロパティの
    /// スキャン候補を範囲索引で絞るために使う。正当性は WHERE の retain が保証し、
    /// ここでは候補のスーパーセットを得るだけなので誤りは生じない。
    range_hints: Vec<(String, String, BinaryOp, PropertyValue)>,
    /// ACL を適用するときのグラフのラッパー。`graph` はこれを指す（server/173）
    acl: Option<Box<crate::acl::AclBackend<'a>>>,
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
            constraints: ManagerSlot::Owned(ConstraintManager::new()),
            fulltext: ManagerSlot::Owned(FulltextManager::new()),
            property_index: ManagerSlot::Owned(PropertyIndex::new()),
            params: HashMap::new(),
            range_hints: Vec::new(),
            acl: None,
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
            constraints: ManagerSlot::Owned(ConstraintManager::new()),
            fulltext: ManagerSlot::Owned(FulltextManager::new()),
            property_index: ManagerSlot::Owned(PropertyIndex::new()),
            params: HashMap::new(),
            range_hints: Vec::new(),
            acl: None,
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
            constraints: ManagerSlot::Owned(ConstraintManager::new()),
            fulltext: ManagerSlot::Owned(FulltextManager::new()),
            property_index: ManagerSlot::Owned(PropertyIndex::new()),
            params: HashMap::new(),
            range_hints: Vec::new(),
            acl: None,
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
        property_index: PropertyIndex,
    ) -> Self {
        let g: &dyn GraphBackend = graph;
        Self {
            graph: (g as *const dyn GraphBackend) as *mut dyn GraphBackend,
            readonly: false,
            _marker: std::marker::PhantomData,
            constraints: ManagerSlot::Owned(constraints),
            fulltext: ManagerSlot::Owned(fulltext),
            property_index: ManagerSlot::Owned(property_index),
            params: HashMap::new(),
            range_hints: Vec::new(),
            acl: None,
        }
    }

    /// Create an `Executor` for a [`ConcurrentGraph`] that borrows the shared
    /// managers instead of owning a copy.
    ///
    /// Intended for read-only statements: the managers are only read, so the
    /// caller can hold read locks and run many such executors concurrently
    /// without cloning indexes per query. (An unexpected mutation is applied to
    /// a private copy and never reaches the shared managers.)
    ///
    /// # Safety
    ///
    /// Same safety requirements as [`new_concurrent`].
    pub unsafe fn new_concurrent_shared(
        graph: &'a ConcurrentGraph,
        constraints: &'a ConstraintManager,
        fulltext: &'a FulltextManager,
        property_index: &'a PropertyIndex,
    ) -> Self {
        let g: &dyn GraphBackend = graph;
        Self {
            graph: (g as *const dyn GraphBackend) as *mut dyn GraphBackend,
            readonly: false,
            _marker: std::marker::PhantomData,
            constraints: ManagerSlot::Shared(constraints),
            fulltext: ManagerSlot::Shared(fulltext),
            property_index: ManagerSlot::Shared(property_index),
            params: HashMap::new(),
            range_hints: Vec::new(),
            acl: None,
        }
    }

    /// Create an `Executor` over any writable [`GraphBackend`] that updates the
    /// shared managers in place (no clone-in / write-back).
    ///
    /// Intended for write statements while the caller holds exclusive locks on
    /// the managers. Index updates stay in step with graph mutations even when
    /// the statement fails midway (the graph is not rolled back either).
    /// The backend may be a wrapper around a [`ConcurrentGraph`] (e.g. one that
    /// records mutations for replication).
    pub fn new_with_backend_exclusive(
        graph: &'a mut (dyn GraphBackend + 'a),
        constraints: &'a mut ConstraintManager,
        fulltext: &'a mut FulltextManager,
        property_index: &'a mut PropertyIndex,
    ) -> Self {
        Self {
            graph: graph as *mut (dyn GraphBackend + 'a),
            readonly: false,
            _marker: std::marker::PhantomData,
            constraints: ManagerSlot::Exclusive(constraints),
            fulltext: ManagerSlot::Exclusive(fulltext),
            property_index: ManagerSlot::Exclusive(property_index),
            params: HashMap::new(),
            range_hints: Vec::new(),
            acl: None,
        }
    }

    /// Consume the executor and return the constraint, fulltext, and property
    /// index managers.
    ///
    /// Use this after `execute()` to persist updated manager state across
    /// requests (the property index in particular must survive so that
    /// `CREATE INDEX` and subsequent index-accelerated lookups work over the
    /// network path, not just within a single Executor instance).
    pub fn into_managers(self) -> (ConstraintManager, FulltextManager, PropertyIndex) {
        (
            self.constraints.into_owned(),
            self.fulltext.into_owned(),
            self.property_index.into_owned(),
        )
    }

    /// ACL を適用して実行する（`None` や空の規則なら何もしない）。
    ///
    /// 見えないノード・プロパティは無いものとして扱い、拒否された書き込みは
    /// [`execute_with_params`](Self::execute_with_params) が
    /// [`ExecuteError::AccessDenied`] を返す。書き込みを取り消すのは呼び出し側の役目。
    pub fn with_access_policy(mut self, policy: Option<crate::acl::AccessPolicy>) -> Self {
        if let Some(policy) = policy.filter(|p| !p.is_empty()) {
            let mut wrapper = Box::new(crate::acl::AclBackend::new(self.graph, policy));
            let g: &mut (dyn GraphBackend + 'a) = &mut *wrapper;
            self.graph = g as *mut (dyn GraphBackend + 'a);
            self.acl = Some(wrapper);
        }
        self
    }

    /// ACL を通さないグラフ。制約の検査と索引の維持に使う
    /// （見えないノードを含めて一意性を守り、索引から値を落とさないため）。
    fn raw_graph(&self) -> &dyn GraphBackend {
        match &self.acl {
            Some(acl) => acl.raw(),
            None => self.graph_ref(),
        }
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
    /// ノードに接続しているエッジの本数（自己ループは 1 本と数える）。
    fn incident_edge_count(&self, node_id: NodeId) -> i64 {
        let graph = self.graph_ref();
        let mut ids: HashSet<maharit_core::EdgeId> =
            graph.outgoing_edges(node_id).iter().map(|e| e.id).collect();
        ids.extend(graph.incoming_edges(node_id).iter().map(|e| e.id));
        ids.len() as i64
    }

    fn graph_mut(&mut self) -> &mut dyn GraphBackend {
        debug_assert!(
            !self.readonly,
            "write operation called on read-only executor"
        );
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
        match self.acl.as_ref().and_then(|acl| acl.take_violation()) {
            Some(msg) if result.is_ok() => Err(ExecuteError::AccessDenied(msg)),
            _ => result,
        }
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

    /// SET/REMOVE で単一プロパティが変化した際に、共有プロパティ索引を追随させる。
    ///
    /// `(プライマリラベル, property)` に索引がある場合のみ旧値を除去し新値を登録する。
    /// これがないと、永続化された索引が SET 後に陳腐化し、索引ベースの候補絞り込みが
    /// 偽陰性（本来一致するノードを取りこぼす）を起こして誤結果になる。
    /// リレーションシップのプロパティ（無ければ null）。
    fn edge_property(&self, edge_id: u64, key: &str) -> Value {
        self.graph_ref()
            .get_edge(edge_id)
            .and_then(|e| e.properties.get(key).map(Value::from))
            .unwrap_or(Value::Null)
    }

    /// ノードの全文検索インデックスを今のプロパティで作り直す。
    fn refresh_fulltext(&mut self, node_id: NodeId) {
        self.fulltext.remove_node(node_id);
        if let Some(node) = self.raw_graph().get_node(node_id) {
            let label = node.primary_label().to_string();
            self.fulltext.index_node(node_id, &label, &node.properties);
        }
    }

    fn reindex_node_property(
        &mut self,
        node_id: NodeId,
        property: &str,
        old: Option<PropertyValue>,
        new: Option<&PropertyValue>,
    ) {
        self.refresh_fulltext(node_id);
        // 索引定義は (label, property) 単位。プライマリラベルで判定する。
        let label = match self.raw_graph().get_node(node_id) {
            Some(n) => n.primary_label().to_string(),
            None => return,
        };
        if !self.property_index.has_index(&label, property) {
            return;
        }
        if let Some(o) = old {
            self.property_index.unindex_property(node_id, property, &o);
        }
        if let Some(n) = new {
            self.property_index.index_property(node_id, property, n);
        }
    }

    /// 文を実行
    pub fn execute(&mut self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
        match stmt {
            Statement::Create(create) => self.execute_create(create),
            Statement::CreateReturn(create, return_clause) => {
                let (_, _, bindings) =
                    self.execute_create_with_bindings(&create, &Bindings::new())?;
                self.build_result_set(&return_clause, &[bindings])
            }
            Statement::Match(m) => self.execute_match(m),
            Statement::Pipeline(p) => self.execute_pipeline(&p),
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
            Statement::Acl(_) | Statement::ShowAcl => Err(ExecuteError::TypeError(
                "GRANT / DENY / REVOKE / SHOW ACL are only available on the server".to_string(),
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
        let (created_nodes, created_edges, _) =
            self.execute_create_with_bindings(&create, &Bindings::new())?;
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
        let evaluated_props: Vec<(String, PropertyValue)> = self
            .pattern_property_exprs(&pattern.properties, &pattern.properties_param)?
            .into_iter()
            .map(|(k, expr)| {
                let val = self.evaluate_expression(&expr, bindings as &Bindings)?;
                let prop_val = self.value_to_property(&val)?;
                Ok((k, prop_val))
            })
            .collect::<Result<Vec<_>, ExecuteError>>()?
            .into_iter()
            // null のプロパティは作らない（Cypher の意味。bug/174）
            .filter(|(_, v)| !matches!(v, PropertyValue::Null))
            .collect();

        let props: HashMap<String, PropertyValue> = evaluated_props.iter().cloned().collect();

        // Validate constraints before creating (using primary label)
        self.constraints
            .validate_node_create(self.raw_graph(), &primary_label, &props, None)?;

        let node_id = self.graph_mut().create_node_with_labels(labels.clone());

        // Set properties
        for (key, prop_val) in &evaluated_props {
            self.graph_mut()
                .set_node_property(node_id, key, prop_val.clone());
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
            bindings.insert(Arc::from(var.as_str()), BindingValue::Node(node_id));
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                // delete_node は隣接エッジも消すので、その本数を先に数えておく
                let incident = self.incident_edge_count(node_id);
                if self.graph_mut().delete_node(node_id).is_some() {
                    self.property_index.remove_node(node_id);
                    self.fulltext.remove_node(node_id);
                    deleted_nodes += 1;
                    deleted_edges += incident;
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
                    self.property_index.remove_node(node_id);
                    self.fulltext.remove_node(node_id);
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
        let mut result_bindings = Vec::with_capacity(all_bindings.len());

        for bindings in &all_bindings {
            let (cn, ce, b) = self.execute_create_with_bindings(&mc.create_clause, bindings)?;
            created_nodes += cn;
            created_edges += ce;
            result_bindings.push(b);
        }

        if let Some(return_clause) = &mc.return_clause {
            return self.build_result_set(return_clause, &result_bindings);
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
    ) -> Result<(i64, i64, Bindings), ExecuteError> {
        let mut bindings = existing_bindings.clone();
        let mut created_nodes = 0i64;
        let mut created_edges = 0i64;
        // 途中まで作ってから失敗しないよう、先に型指定を検査する
        for pattern in &create.patterns {
            check_single_edge_types(pattern)?;
        }

        for pattern in &create.patterns {
            match pattern {
                Pattern::Node(node_pattern) => {
                    // If variable is already bound, skip creation
                    if let Some(var) = &node_pattern.variable
                        && bindings.contains_key(var.as_str())
                    {
                        continue;
                    }
                    self.create_node(node_pattern, &mut bindings)?;
                    created_nodes += 1;
                }
                Pattern::Path(path_pattern) => {
                    // Check if start node is already bound
                    let start_id = if let Some(var) = &path_pattern.start.variable {
                        if let Some(bound) = bindings.get(var.as_str()) {
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
                            if let Some(bound) = bindings.get(var.as_str()) {
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

                        let edge_label = single_edge_type(&segment.edge)?;

                        // Validate endpoint label constraints before creating edge
                        self.constraints.validate_edge_create(
                            self.raw_graph(),
                            &edge_label,
                            from,
                            to,
                        )?;

                        let edge_id = self.graph_mut().create_edge(from, to, edge_label)?;

                        // Evaluate and set edge properties
                        let edge_props: Vec<(String, PropertyValue)> = self
                            .pattern_property_exprs(
                                &segment.edge.properties,
                                &segment.edge.properties_param,
                            )?
                            .into_iter()
                            .map(|(k, expr)| {
                                let val = self.evaluate_expression(&expr, &bindings)?;
                                let prop_val = self.value_to_property(&val)?;
                                Ok((k, prop_val))
                            })
                            .collect::<Result<_, ExecuteError>>()?;

                        for (key, prop_val) in edge_props
                            .into_iter()
                            .filter(|(_, v)| !matches!(v, PropertyValue::Null))
                        {
                            self.graph_mut().set_edge_property(edge_id, &key, prop_val);
                        }
                        if let Some(var) = &segment.edge.variable {
                            bindings.insert(Arc::from(var.as_str()), BindingValue::Edge(edge_id));
                        }

                        created_edges += 1;
                        current_id = end_id;
                    }
                }
            }
        }

        Ok((created_nodes, created_edges, bindings))
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

        // Apply SET clause, then any following SET / REMOVE clauses in order
        self.apply_set_clause(&ms.set_clause, &all_bindings)?;
        self.apply_update_clauses(&ms.more_updates, &all_bindings)?;

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

    /// マップを返す式を評価し、プロパティの (キー, 値) 列にする。
    fn evaluate_property_map(
        &self,
        expr: &Expression,
        bindings: &Bindings,
        clause: &str,
    ) -> Result<Vec<(String, PropertyValue)>, ExecuteError> {
        match self.evaluate_expression(expr, bindings)? {
            Value::Map(map) => map
                .into_iter()
                .map(|(k, v)| Ok((k, self.value_to_property(&v)?)))
                .collect(),
            other => Err(ExecuteError::TypeError(format!(
                "{clause} requires a map, got {other:?}"
            ))),
        }
    }

    /// `SET n += map`（`replace = false`）/ `SET n = map`（`replace = true`）を適用する。
    /// 置き換えでは、マップに無い既存のプロパティを削除する。
    /// ノードまたはリレーションシップのプロパティを 1 つ消す（無ければ何もしない）。
    fn remove_property(
        &mut self,
        binding_value: &BindingValue,
        property: &str,
    ) -> Result<(), ExecuteError> {
        match binding_value {
            BindingValue::Node(node_id) => {
                if let Some(node) = self.graph_ref().get_node(*node_id) {
                    self.constraints.validate_property_remove(&node, property)?;
                }
                if let Some(old) = self.graph_mut().remove_node_property(*node_id, property) {
                    self.reindex_node_property(*node_id, property, Some(old), None);
                }
                Ok(())
            }
            BindingValue::Edge(edge_id) => {
                self.graph_mut().remove_edge_property(*edge_id, property);
                Ok(())
            }
            _ => Err(ExecuteError::TypeError(
                "SET requires node or edge binding".to_string(),
            )),
        }
    }

    fn set_properties_from_map(
        &mut self,
        bindings: &Bindings,
        variable: &str,
        evaluated: Vec<(String, PropertyValue)>,
        replace: bool,
    ) -> Result<(), ExecuteError> {
        let binding_value = bindings
            .get(variable)
            .ok_or_else(|| ExecuteError::UndefinedVariable(variable.to_string()))?
            .clone();
        // null の値はプロパティの削除（bug/174）。`SET n = {…}` では残さないキーと同じ扱い
        let (nulls, evaluated): (Vec<_>, Vec<_>) = evaluated
            .into_iter()
            .partition(|(_, v)| matches!(v, PropertyValue::Null));
        for (key, _) in &nulls {
            self.remove_property(&binding_value, key)?;
        }
        match &binding_value {
            BindingValue::Node(node_id) => {
                let node_id = *node_id;
                // Validate constraints for each new property
                for (key, prop_val) in &evaluated {
                    if let Some(node) = self.graph_ref().get_node(node_id) {
                        self.constraints.validate_property_set(
                            self.raw_graph(),
                            &node,
                            key,
                            prop_val,
                        )?;
                    }
                }
                if replace && let Some(node) = self.graph_ref().get_node(node_id) {
                    for key in node.properties.keys() {
                        if evaluated.iter().any(|(k, _)| k == key) {
                            continue;
                        }
                        let old = self.graph_mut().remove_node_property(node_id, key);
                        self.reindex_node_property(node_id, key, old, None);
                    }
                }
                for (key, prop_val) in evaluated {
                    let old = self.graph_ref().get_node_property(node_id, &key);
                    self.graph_mut()
                        .set_node_property(node_id, &key, prop_val.clone());
                    self.reindex_node_property(node_id, &key, old, Some(&prop_val));
                }
            }
            BindingValue::Edge(edge_id) => {
                let edge_id = *edge_id;
                if replace && let Some(edge) = self.graph_ref().get_edge(edge_id) {
                    for key in edge.properties.keys() {
                        if !evaluated.iter().any(|(k, _)| k == key) {
                            self.graph_mut().remove_edge_property(edge_id, key);
                        }
                    }
                }
                for (key, prop_val) in evaluated {
                    self.graph_mut().set_edge_property(edge_id, &key, prop_val);
                }
            }
            _ => {
                return Err(ExecuteError::TypeError(
                    "SET requires node or edge binding".to_string(),
                ));
            }
        }
        Ok(())
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
                            .get(variable.as_str())
                            .ok_or_else(|| ExecuteError::UndefinedVariable(variable.clone()))?;

                        let value = self.evaluate_expression(value_expr, bindings)?;
                        let prop_value = self.value_to_property(&value)?;

                        // `SET n.p = null` はプロパティの削除（bug/174）
                        if matches!(prop_value, PropertyValue::Null) {
                            let binding_value = binding_value.clone();
                            self.remove_property(&binding_value, property)?;
                            continue;
                        }

                        match binding_value {
                            BindingValue::Node(node_id) => {
                                // Validate constraint before setting
                                if let Some(node) = self.graph_ref().get_node(*node_id) {
                                    self.constraints.validate_property_set(
                                        self.raw_graph(),
                                        &node,
                                        property,
                                        &prop_value,
                                    )?;
                                }
                                let old = self.graph_ref().get_node_property(*node_id, property);
                                self.graph_mut().set_node_property(
                                    *node_id,
                                    property,
                                    prop_value.clone(),
                                );
                                self.reindex_node_property(
                                    *node_id,
                                    property,
                                    old,
                                    Some(&prop_value),
                                );
                            }
                            BindingValue::Edge(edge_id) => {
                                self.graph_mut()
                                    .set_edge_property(*edge_id, property, prop_value);
                            }
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "SET requires node or edge binding".to_string(),
                                ));
                            }
                        }
                    }
                    SetItem::MergeProperties(variable, props_map) => {
                        // Evaluate all property expressions first
                        let evaluated: Vec<(String, PropertyValue)> = props_map
                            .iter()
                            .map(|(k, expr)| {
                                let val = self.evaluate_expression(expr, bindings)?;
                                let prop_val = self.value_to_property(&val)?;
                                Ok((k.clone(), prop_val))
                            })
                            .collect::<Result<_, ExecuteError>>()?;
                        self.set_properties_from_map(bindings, variable, evaluated, false)?;
                    }
                    SetItem::MergeMap(variable, expr) => {
                        let evaluated = self.evaluate_property_map(expr, bindings, "SET +=")?;
                        self.set_properties_from_map(bindings, variable, evaluated, false)?;
                    }
                    SetItem::ReplaceProperties(variable, expr) => {
                        let evaluated = self.evaluate_property_map(expr, bindings, "SET =")?;
                        self.set_properties_from_map(bindings, variable, evaluated, true)?;
                    }
                    SetItem::AddLabel(variable, new_label) => {
                        let binding_value = bindings
                            .get(variable.as_str())
                            .ok_or_else(|| ExecuteError::UndefinedVariable(variable.clone()))?;

                        match binding_value {
                            BindingValue::Node(node_id) => {
                                let node_id = *node_id;
                                self.graph_mut().add_node_label(node_id, new_label.clone());
                                self.refresh_fulltext(node_id);
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

        let result_bindings = self.apply_merge(&merge, &all_bindings)?;

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

    /// 行ごとに MERGE のパターンを照合し、無ければ作る。照合・作成後の行を返す。
    fn apply_merge(
        &mut self,
        merge: &MergeStatement,
        all_bindings: &[Bindings],
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result_bindings = Vec::new();

        for bindings in all_bindings {
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
        Ok(result_bindings)
    }

    // ========== 句の列（task171） ==========

    /// 句を書かれた順に、行（バインディング）の列へ適用する。
    fn execute_pipeline(&mut self, p: &PipelineStatement) -> Result<ResultSet, ExecuteError> {
        let mut rows: Vec<Bindings> = vec![Bindings::new()];
        let (mut created_nodes, mut created_edges) = (0i64, 0i64);
        let (mut deleted_nodes, mut deleted_edges) = (0i64, 0i64);
        let mut properties_set = 0i64;
        let counts = |this: &Self| {
            (
                this.graph_ref().node_count() as i64,
                this.graph_ref().edge_count() as i64,
            )
        };

        for clause in &p.clauses {
            match clause {
                PipelineClause::Read(segment) => {
                    rows = self.execute_query_segment(segment, rows)?;
                }
                PipelineClause::Create(create) => {
                    let mut next = Vec::with_capacity(rows.len());
                    for b in &rows {
                        let (n, e, b) = self.execute_create_with_bindings(create, b)?;
                        created_nodes += n;
                        created_edges += e;
                        next.push(b);
                    }
                    rows = next;
                }
                PipelineClause::Merge(merge) => {
                    let before = counts(self);
                    rows = self.apply_merge(merge, &rows)?;
                    let after = counts(self);
                    created_nodes += after.0 - before.0;
                    created_edges += after.1 - before.1;
                }
                PipelineClause::Update(update) => {
                    if let UpdateClause::Set(set) = update {
                        properties_set += set.items.len() as i64 * rows.len() as i64;
                    }
                    self.apply_update_clauses(std::slice::from_ref(update), &rows)?;
                }
                PipelineClause::Delete(delete) => {
                    let before = counts(self);
                    for b in &rows {
                        self.apply_delete_clause(delete, b)?;
                    }
                    let after = counts(self);
                    deleted_nodes += before.0 - after.0;
                    deleted_edges += before.1 - after.1;
                }
                PipelineClause::Foreach(f) => {
                    let before = counts(self);
                    for b in &rows {
                        self.execute_foreach_stmt_ref(f, b)?;
                    }
                    let after = counts(self);
                    created_nodes += (after.0 - before.0).max(0);
                    created_edges += (after.1 - before.1).max(0);
                    deleted_nodes += (before.0 - after.0).max(0);
                    deleted_edges += (before.1 - after.1).max(0);
                }
            }
        }

        if let Some(return_clause) = &p.return_clause {
            return self.build_result_set(return_clause, &rows);
        }
        Ok(ResultSet::new(
            vec![
                "created_nodes".to_string(),
                "created_edges".to_string(),
                "deleted_nodes".to_string(),
                "deleted_edges".to_string(),
                "properties_set".to_string(),
            ],
            vec![Row {
                columns: vec![
                    Value::Int(created_nodes),
                    Value::Int(created_edges),
                    Value::Int(deleted_nodes),
                    Value::Int(deleted_edges),
                    Value::Int(properties_set),
                ],
            }],
        ))
    }

    fn create_pattern(
        &mut self,
        pattern: &Pattern,
        bindings: &mut Bindings,
    ) -> Result<(), ExecuteError> {
        check_single_edge_types(pattern)?;
        match pattern {
            Pattern::Node(node_pattern) => {
                if let Some(var) = &node_pattern.variable
                    && bindings.contains_key(var.as_str())
                {
                    return Ok(());
                }
                self.create_node(node_pattern, bindings)?;
                Ok(())
            }
            Pattern::Path(path_pattern) => {
                let start_id = if let Some(var) = &path_pattern.start.variable {
                    if let Some(bound) = bindings.get(var.as_str()) {
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
                        if let Some(bound) = bindings.get(var.as_str()) {
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

                    let edge_label = single_edge_type(&segment.edge)?;

                    // Validate endpoint label constraints before creating edge
                    self.constraints.validate_edge_create(
                        self.raw_graph(),
                        &edge_label,
                        from,
                        to,
                    )?;

                    let edge_id = self.graph_mut().create_edge(from, to, edge_label)?;

                    // Evaluate and set edge properties
                    let edge_props: Vec<(String, PropertyValue)> = self
                        .pattern_property_exprs(
                            &segment.edge.properties,
                            &segment.edge.properties_param,
                        )?
                        .into_iter()
                        .map(|(k, expr)| {
                            let val = self.evaluate_expression(&expr, bindings as &Bindings)?;
                            let prop_val = self.value_to_property(&val)?;
                            Ok((k, prop_val))
                        })
                        .collect::<Result<_, ExecuteError>>()?;

                    for (key, prop_val) in edge_props
                        .into_iter()
                        .filter(|(_, v)| !matches!(v, PropertyValue::Null))
                    {
                        self.graph_mut().set_edge_property(edge_id, &key, prop_val);
                    }
                    // MERGE の ON CREATE SET / RETURN から `r` を参照できるように束縛する
                    if let Some(var) = &segment.edge.variable {
                        bindings.insert(Arc::from(var.as_str()), BindingValue::Edge(edge_id));
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

        // Apply REMOVE clause, then any following SET / REMOVE clauses in order
        for bindings in &all_bindings {
            self.apply_remove_clause(&mr.remove_clause, bindings)?;
        }
        self.apply_update_clauses(&mr.more_updates, &all_bindings)?;

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
            bindings.insert(
                Arc::from(uw.variable.as_str()),
                BindingValue::Scalar(item.clone()),
            );
            all_bindings.push(bindings);
        }

        // Execute CREATE if present
        if let Some(create_clause) = &uw.create_clause {
            let mut created_nodes = 0i64;
            let mut created_edges = 0i64;

            // Continue with the post-CREATE bindings so SET / RETURN can refer
            // to the variables introduced by CREATE.
            let mut created_bindings = Vec::with_capacity(all_bindings.len());
            for bindings in &all_bindings {
                let (cn, ce, b) = self.execute_create_with_bindings(create_clause, bindings)?;
                created_nodes += cn;
                created_edges += ce;
                created_bindings.push(b);
            }
            let all_bindings = created_bindings;

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
            // Bind nodes as nodes so `SET n.prop = …` can update them.
            let value = match item {
                Value::Node(id) | Value::NodeData { id, .. } => BindingValue::Node(*id),
                other => BindingValue::Scalar(other.clone()),
            };
            bindings.insert(Arc::from(stmt.variable.as_str()), value);

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

    fn apply_update_clauses(
        &mut self,
        updates: &[UpdateClause],
        all_bindings: &[Bindings],
    ) -> Result<(), ExecuteError> {
        for update in updates {
            match update {
                UpdateClause::Set(set) => self.apply_set_clause(set, all_bindings)?,
                UpdateClause::Remove(remove) => {
                    for bindings in all_bindings {
                        self.apply_remove_clause(remove, bindings)?;
                    }
                }
            }
        }
        Ok(())
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
                        .get(var.as_str())
                        .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;

                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                self.constraints.validate_property_remove(&node, prop)?;
                            }
                            let removed = self.graph_mut().remove_node_property(*node_id, prop);
                            if let Some(old) = removed {
                                self.reindex_node_property(*node_id, prop, Some(old), None);
                            }
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
                    if let Some(node_id) = bindings.get(var.as_str()).and_then(|v| v.as_node()) {
                        self.graph_mut().remove_node_label(node_id, label);
                        self.refresh_fulltext(node_id);
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
            if let Some(binding_value) = bindings.get(var.as_str()) {
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
            if !delete.detach && self.graph_ref().has_incident_edges(node_id) {
                continue;
            }
            if self.graph_mut().delete_node(node_id).is_some() {
                self.property_index.remove_node(node_id);
                self.fulltext.remove_node(node_id);
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

    fn execute_create_index(
        &mut self,
        ci: CreateIndexStatement,
    ) -> Result<ResultSet, ExecuteError> {
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
                self.property_index
                    .index_property(node_id, &ci.property, val);
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

    /// Execute a top-level procedure call:
    /// `CALL proc.name(args) [YIELD col [AS alias], ...] [WHERE / MATCH / WITH ...] [FOREACH | RETURN]`
    ///
    /// The procedure produces rows keyed by its output columns. YIELD picks
    /// (and optionally renames) columns, after which the rows flow through the
    /// trailing segments exactly like MATCH bindings.
    ///
    /// Currently supports:
    ///   - `db.index.fulltext.search(indexName, query)` — (node, score)
    ///   - `db.labels()` — (label)
    fn execute_procedure_call(
        &mut self,
        pc: ProcedureCallStatement,
    ) -> Result<ResultSet, ExecuteError> {
        let (columns, rows) = match pc.procedure.as_str() {
            "db.index.fulltext.search" => self.fulltext_search_procedure(&pc.arguments)?,
            "db.labels" => self.labels_procedure(&pc.arguments)?,
            name if algo::ALGORITHM_PROCEDURES.iter().any(|(n, _)| *n == name) => {
                self.algorithm_procedure(name, &pc.arguments)?
            }
            other => {
                return Err(ExecuteError::TypeError(format!(
                    "unknown procedure: {}",
                    other
                )));
            }
        };

        // YIELD: keep only the named columns, under their aliases.
        let (out_columns, mut bindings): (Vec<String>, Vec<Bindings>) = if pc.yield_items.is_empty()
        {
            (columns.iter().map(|c| c.to_string()).collect(), rows)
        } else {
            for item in &pc.yield_items {
                if !columns.contains(&item.column.as_str()) {
                    return Err(ExecuteError::TypeError(format!(
                        "{} does not yield '{}' (available: {})",
                        pc.procedure,
                        item.column,
                        columns.join(", ")
                    )));
                }
            }
            let renamed = rows
                .into_iter()
                .map(|row| {
                    pc.yield_items
                        .iter()
                        .filter_map(|item| {
                            row.get(item.column.as_str())
                                .map(|v| (Arc::from(item.name()), v.clone()))
                        })
                        .collect()
                })
                .collect();
            (
                pc.yield_items
                    .iter()
                    .map(|i| i.name().to_string())
                    .collect(),
                renamed,
            )
        };

        for segment in &pc.segments {
            bindings = self.execute_query_segment(segment, bindings)?;
        }

        self.apply_update_clauses(&pc.updates, &bindings)?;
        if !pc.updates.is_empty() && pc.foreach_clause.is_none() && pc.return_clause.is_none() {
            let set_items: usize = pc
                .updates
                .iter()
                .map(|u| match u {
                    UpdateClause::Set(s) => s.items.len(),
                    UpdateClause::Remove(_) => 0,
                })
                .sum();
            return Ok(ResultSet::new(
                vec!["properties_set".to_string()],
                vec![Row {
                    columns: vec![Value::Int((set_items * bindings.len()) as i64)],
                }],
            ));
        }

        if let Some(foreach) = &pc.foreach_clause {
            for b in &bindings {
                self.execute_foreach_stmt_ref(foreach, b)?;
            }
            return Ok(ResultSet::new(
                vec!["foreach_result".to_string()],
                vec![Row {
                    columns: vec![Value::String("ok".to_string())],
                }],
            ));
        }

        if let Some(return_clause) = &pc.return_clause {
            return self.build_result_set(return_clause, &bindings);
        }

        // No RETURN: project the yielded columns directly.
        let rows = bindings
            .iter()
            .map(|b| Row {
                columns: out_columns
                    .iter()
                    .map(|c| match b.get(c.as_str()) {
                        Some(BindingValue::Node(id)) => Value::Node(*id),
                        Some(BindingValue::Scalar(v)) => v.clone(),
                        Some(BindingValue::Edge(id)) => Value::Edge(*id),
                        Some(BindingValue::Path { nodes, edges }) => Value::Path {
                            nodes: nodes.clone(),
                            edges: edges.clone(),
                        },
                        None => Value::Null,
                    })
                    .collect(),
            })
            .collect();
        Ok(ResultSet::new(out_columns, rows))
    }

    /// `db.index.fulltext.search(indexName, query)` — rows of (node, score).
    fn fulltext_search_procedure(
        &self,
        arguments: &[Expression],
    ) -> Result<(&'static [&'static str], Vec<Bindings>), ExecuteError> {
        if arguments.len() != 2 {
            return Err(ExecuteError::TypeError(
                "db.index.fulltext.search requires exactly 2 arguments: (indexName, query)"
                    .to_string(),
            ));
        }

        // Evaluate arguments — they must resolve to strings
        let empty_bindings = Bindings::new();
        let index_name = match self.evaluate_expression(&arguments[0], &empty_bindings)? {
            Value::String(s) => s,
            _ => {
                return Err(ExecuteError::TypeError(
                    "db.index.fulltext.search: first argument must be a string (index name)"
                        .to_string(),
                ));
            }
        };
        let query = match self.evaluate_expression(&arguments[1], &empty_bindings)? {
            Value::String(s) => s,
            _ => {
                return Err(ExecuteError::TypeError(
                    "db.index.fulltext.search: second argument must be a string (query)"
                        .to_string(),
                ));
            }
        };

        // ACL: 見えないノードと、索引の対象に読めないプロパティを含むノードは返さない
        // （スコアから読めない値が推測できてしまうため）
        let indexed: Vec<String> = self
            .fulltext
            .get_index(&index_name)
            .map(|idx| idx.properties().to_vec())
            .unwrap_or_default();
        let rows = self
            .fulltext
            .search(&index_name, &query)?
            .iter()
            .filter(|result| match &self.acl {
                None => true,
                Some(acl) => match acl.raw().get_node(result.node_id) {
                    Some(n) => {
                        let read = &acl.policy().read;
                        !read.node_denied(&n.labels)
                            && !indexed.iter().any(|p| read.property_denied(&n.labels, p))
                    }
                    None => false,
                },
            })
            .map(|result| {
                let mut bindings = Bindings::new();
                bindings.insert(Arc::from("node"), BindingValue::Node(result.node_id));
                bindings.insert(
                    Arc::from("score"),
                    BindingValue::Scalar(Value::Float(result.score)),
                );
                bindings
            })
            .collect();
        Ok((&["node", "score"], rows))
    }

    /// `db.labels()` — one row per distinct node label, in sorted order.
    fn labels_procedure(
        &self,
        arguments: &[Expression],
    ) -> Result<(&'static [&'static str], Vec<Bindings>), ExecuteError> {
        if !arguments.is_empty() {
            return Err(ExecuteError::TypeError(
                "db.labels takes no arguments".to_string(),
            ));
        }
        let labels: std::collections::BTreeSet<String> = self
            .graph_ref()
            .all_nodes()
            .into_iter()
            .flat_map(|n| n.labels.into_iter())
            .collect();
        let rows = labels
            .into_iter()
            .map(|label| {
                let mut bindings = Bindings::new();
                bindings.insert(
                    Arc::from("label"),
                    BindingValue::Scalar(Value::String(label)),
                );
                bindings
            })
            .collect();
        Ok((&["label"], rows))
    }

    // ========== EXPLAIN / PROFILE ==========

    /// Graph statistics for EXPLAIN / PROFILE: counts per label and the
    /// property indexes that exist, so the plan shows index use (bug/167).
    fn plan_stats(&self) -> crate::planner::GraphStats {
        let graph = self.graph_ref();
        let mut label_counts: HashMap<String, u64> = HashMap::new();
        for node in graph.all_nodes() {
            for label in &node.labels {
                *label_counts.entry(label.clone()).or_insert(0) += 1;
            }
        }
        crate::planner::GraphStats {
            node_count: graph.node_count() as u64,
            edge_count: graph.edge_count() as u64,
            label_counts,
            indexed_properties: self
                .property_index
                .list_indexes()
                .into_iter()
                .map(|d| (d.label.clone(), d.property.clone()))
                .collect(),
        }
    }

    fn execute_explain(&self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
        let plan = crate::planner::build_plan_with_stats(&stmt, &self.plan_stats());
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
        let mut plan = crate::planner::build_plan_with_stats(&stmt, &self.plan_stats());

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
            Value::Duration {
                months,
                days,
                millis,
            } => Ok(PropertyValue::Duration {
                months: *months,
                days: *days,
                millis: *millis,
            }),
            // リストはプリミティブの要素だけ（Neo4j と同じくリストのリストやマップは保存できない）
            Value::List(items) => items
                .iter()
                .map(|item| match item {
                    Value::List(_) => Err(ExecuteError::TypeError(
                        "a list stored as a property cannot contain lists".to_string(),
                    )),
                    other => self.value_to_property(other),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(PropertyValue::List),
            _ => Err(ExecuteError::TypeError(
                "cannot convert to property value".to_string(),
            )),
        }
    }

    // ========== MATCH ==========

    fn execute_match(&mut self, m: MatchStatement) -> Result<ResultSet, ExecuteError> {
        // Fast path: `MATCH (n:L) RETURN count(*)` / `count(n)` with no filters
        // answers from the label/node count without materializing any bindings.
        if let Some(rs) = self.try_simple_count(&m) {
            return Ok(rs);
        }

        // Detect an early-termination limit: applicable only when there is no ORDER BY
        // and no aggregation, since both require collecting all rows before outputting.
        let has_aggregation = m.return_clause.items.iter().any(Self::is_aggregate);

        let early_limit: Option<usize> = if !has_aggregation && m.return_clause.order_by.is_none() {
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

    /// Fast path for a filter-free node count: `MATCH (n:L) RETURN count(*)`
    /// or `count(n)`. Returns `Some(result)` when the query matches this shape,
    /// answering from `nodes_by_label`/`node_count` without building bindings.
    /// Returns `None` (fall back to the general path) for anything more complex.
    fn try_simple_count(&self, m: &MatchStatement) -> Option<ResultSet> {
        if m.call_clause.is_some() || m.segments.len() != 1 {
            return None;
        }
        let seg = &m.segments[0];
        if seg.where_clause.is_some() || seg.with_clause.is_some() || !seg.unwinds.is_empty() {
            return None;
        }
        if seg.match_clauses.len() != 1 {
            return None;
        }
        let clause = &seg.match_clauses[0];
        if clause.optional || clause.patterns.len() != 1 {
            return None;
        }
        let np = match &clause.patterns[0] {
            Pattern::Node(np) => np,
            _ => return None,
        };
        // Inline properties or multiple labels would filter the set, so the
        // simple count identity no longer holds.
        if !np.properties.is_empty() || np.properties_param.is_some() || np.labels.len() > 1 {
            return None;
        }

        let rc = &m.return_clause;
        if rc.order_by.is_some() || rc.skip.is_some() || rc.limit.is_some() || rc.items.is_empty() {
            return None;
        }

        // Every return item must be a plain count(*) or count(<the node var>).
        let node_var = np.variable.as_deref();
        if !rc.items.iter().all(|it| Self::is_plain_count(it, node_var)) {
            return None;
        }

        let cnt = match np.labels.first() {
            Some(label) => self.graph_ref().nodes_by_label(label).len(),
            None => self.graph_ref().node_count(),
        } as i64;

        let columns: Vec<String> = rc
            .items
            .iter()
            .map(|it| self.return_item_to_column_name(it))
            .collect();
        let row_values = vec![Value::Int(cnt); rc.items.len()];
        Some(ResultSet::new(
            columns,
            vec![Row {
                columns: row_values,
            }],
        ))
    }

    /// True if `item` is `count(*)` or `count(v)` where `v` is `node_var`
    /// (unwrapping any `AS` alias). Both count exactly the matched node set.
    fn is_plain_count(item: &ReturnItem, node_var: Option<&str>) -> bool {
        match item {
            ReturnItem::Alias(inner, _) => Self::is_plain_count(inner, node_var),
            ReturnItem::Aggregate(AggregateFunction::Count(inner)) => match inner {
                None => true, // count(*)
                Some(boxed) => match boxed.as_ref() {
                    ReturnItem::Variable(v) => node_var == Some(v.as_str()),
                    _ => false,
                },
            },
            _ => false,
        }
    }

    fn execute_call_subquery(
        &self,
        call: &CallSubquery,
        outer_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        for outer in outer_bindings {
            // Run every UNION branch and concatenate their rows.
            let (col_names, mut rows) = self.call_subquery_branch_rows(call, &outer)?;
            let mut dedup = false;
            let mut branch = call;
            while let Some((union_type, next)) = &branch.union {
                let (next_cols, next_rows) = self.call_subquery_branch_rows(next, &outer)?;
                if next_cols != col_names {
                    return Err(ExecuteError::TypeError(format!(
                        "all UNION branches in CALL {{}} must return the same columns: [{}] vs [{}]",
                        col_names.join(", "),
                        next_cols.join(", ")
                    )));
                }
                rows.extend(next_rows);
                dedup |= *union_type == UnionType::Union;
                branch = next;
            }
            if dedup {
                let mut seen = HashSet::new();
                rows.retain(|row| {
                    let key: Vec<String> = row.columns.iter().map(|v| format!("{}", v)).collect();
                    seen.insert(key.join("\x00"))
                });
            }

            // No inner results - skip this outer row (inner join semantics).
            // Otherwise combine the outer binding with each inner row's columns as scalars.
            for inner_row in &rows {
                let mut merged = outer.clone();
                for (col_name, col_val) in col_names.iter().zip(inner_row.columns.iter()) {
                    merged.insert(
                        Arc::from(col_name.as_str()),
                        BindingValue::Scalar(col_val.clone()),
                    );
                }
                result.push(merged);
            }
        }

        Ok(result)
    }

    /// Evaluate one CALL subquery branch (ignoring its UNION continuation) for
    /// a single outer row. Returns the output column names and rows.
    fn call_subquery_branch_rows(
        &self,
        call: &CallSubquery,
        outer: &Bindings,
    ) -> Result<(Vec<String>, Vec<Row>), ExecuteError> {
        // Build inner starting bindings from WITH imports
        let inner_start = if let Some(ref imports) = call.with_import {
            let mut inner = Bindings::new();
            for var in imports {
                if let Some(val) = outer.get(var.as_str()) {
                    inner.insert(Arc::from(var.as_str()), val.clone());
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

        Ok((col_names, inner_result.rows))
    }

    fn execute_query_segment(
        &mut self,
        segment: &QuerySegment,
        mut bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Filter pushdown: pull simple `var.prop = <const>` equalities out of the
        // WHERE clause and inject them into the matching node patterns. This lets
        // `match_node_pattern` use the property index (or at least filter during
        // the scan) instead of materializing every label match and filtering
        // afterwards. The original WHERE retain below still runs, so semantics are
        // unchanged — we only prune candidates earlier.
        let pushable = match &segment.where_clause {
            Some(w) => {
                let mut v = Vec::new();
                Self::collect_pushable_equalities(w, &mut v);
                v
            }
            None => Vec::new(),
        };

        // Range pushdown: collect `var.prop <op> <numeric>` predicates and stash
        // them so `match_node_pattern` can narrow candidates via the range index
        // (only an optimization; the WHERE retain below guarantees correctness).
        //
        // Skip entirely when no property index is defined: range narrowing needs
        // an index to fire, so without one this collection is pure overhead on
        // every WHERE query.
        self.range_hints = match &segment.where_clause {
            Some(w) if self.property_index.has_any_index() => {
                let mut v = Vec::new();
                Self::collect_range_predicates(w, &mut v);
                v
            }
            _ => Vec::new(),
        };

        // UNWIND: 行ごとにリストを展開する（null は 0 行、リスト以外は 1 行）
        for unwind in &segment.unwinds {
            let mut expanded = Vec::new();
            for b in bindings {
                let items = match self.evaluate_expression(&unwind.expression, &b)? {
                    Value::List(items) => items,
                    Value::Null => Vec::new(),
                    other => vec![other],
                };
                for item in items {
                    let value = match item {
                        Value::Node(id) | Value::NodeData { id, .. } => BindingValue::Node(id),
                        Value::Edge(id) => BindingValue::Edge(id),
                        other => BindingValue::Scalar(other),
                    };
                    let mut row = b.clone();
                    row.insert(Arc::from(unwind.variable.as_str()), value);
                    expanded.push(row);
                }
            }
            bindings = expanded;
        }

        // Execute MATCH clauses (augmented with pushed-down predicates where safe).
        for match_clause in &segment.match_clauses {
            if pushable.is_empty() || match_clause.optional {
                // OPTIONAL MATCH is skipped: pushing a predicate into it could turn
                // a NULL-preserving row into a filtered one before the WHERE runs.
                bindings = self.execute_match_clause(match_clause, bindings)?;
            } else {
                let augmented = MatchClause {
                    patterns: Self::augment_patterns(&match_clause.patterns, &pushable),
                    optional: match_clause.optional,
                };
                bindings = self.execute_match_clause(&augmented, bindings)?;
            }
        }

        // Confine range hints to this segment's node scan so they cannot affect
        // later WHERE-clause pattern predicates, WITH, or subquery evaluation.
        self.range_hints.clear();

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

    // ── WHERE 述語のパターン押し下げ（フィルタプッシュダウン）補助 ──────────────

    /// 押し下げ可能な定数（リテラル or パラメータ）かどうか。
    /// 他変数を参照する式は、マッチ時点で束縛されているとは限らないため対象外。
    fn is_pushable_const(expr: &Expression) -> bool {
        matches!(expr, Expression::Literal(_) | Expression::Parameter(_))
    }

    /// WHERE 式からトップレベルで AND 連結された `var.prop = <const>` の等価述語を
    /// 収集する。OR / NOT などの下には降りない（誤って絞り込まないため）。
    fn collect_pushable_equalities(expr: &Expression, out: &mut Vec<(String, String, Expression)>) {
        match expr {
            Expression::BinaryOp(l, BinaryOp::And, r) => {
                Self::collect_pushable_equalities(l, out);
                Self::collect_pushable_equalities(r, out);
            }
            Expression::BinaryOp(l, BinaryOp::Eq, r) => match (l.as_ref(), r.as_ref()) {
                (Expression::Property(v, p), rhs) if Self::is_pushable_const(rhs) => {
                    out.push((v.clone(), p.clone(), rhs.clone()));
                }
                (lhs, Expression::Property(v, p)) if Self::is_pushable_const(lhs) => {
                    out.push((v.clone(), p.clone(), lhs.clone()));
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// WHERE 式からトップレベル AND で連結された数値の範囲述語
    /// `var.prop <op> <numeric literal>` を収集する（op ∈ Lt/Gt/Lte/Gte）。
    /// プロパティが左右どちらの辺でも、演算子を向きに合わせて正規化する。
    /// 収集結果は範囲索引での候補絞り込みに使う（正当性は WHERE retain が保証）。
    fn collect_range_predicates(
        expr: &Expression,
        out: &mut Vec<(String, String, BinaryOp, PropertyValue)>,
    ) {
        // リテラルを数値の PropertyValue に変換（それ以外は None）。
        fn numeric(expr: &Expression) -> Option<PropertyValue> {
            match expr {
                Expression::Literal(Literal::Int(n)) => Some(PropertyValue::Int(*n)),
                Expression::Literal(Literal::Float(f)) => Some(PropertyValue::Float(*f)),
                _ => None,
            }
        }
        // プロパティが右辺にある場合に演算子を反転する（`40 < n.age` → `n.age > 40`）。
        fn flip(op: BinaryOp) -> BinaryOp {
            match op {
                BinaryOp::Lt => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Lt,
                BinaryOp::Lte => BinaryOp::Gte,
                BinaryOp::Gte => BinaryOp::Lte,
                other => other,
            }
        }
        match expr {
            Expression::BinaryOp(l, BinaryOp::And, r) => {
                Self::collect_range_predicates(l, out);
                Self::collect_range_predicates(r, out);
            }
            Expression::BinaryOp(l, op, r)
                if matches!(
                    op,
                    BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Lte | BinaryOp::Gte
                ) =>
            {
                match (l.as_ref(), r.as_ref()) {
                    (Expression::Property(v, p), rhs) => {
                        if let Some(val) = numeric(rhs) {
                            out.push((v.clone(), p.clone(), *op, val));
                        }
                    }
                    (lhs, Expression::Property(v, p)) => {
                        if let Some(val) = numeric(lhs) {
                            out.push((v.clone(), p.clone(), flip(*op), val));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// パターン列の各ノードパターンに、対応する変数の等価述語を注入したコピーを返す。
    fn augment_patterns(
        patterns: &[Pattern],
        eqs: &[(String, String, Expression)],
    ) -> Vec<Pattern> {
        patterns
            .iter()
            .map(|p| match p {
                Pattern::Node(np) => Pattern::Node(Self::augment_node_pattern(np, eqs)),
                Pattern::Path(pp) => Pattern::Path(PathPattern {
                    variable: pp.variable.clone(),
                    start: Self::augment_node_pattern(&pp.start, eqs),
                    segments: pp
                        .segments
                        .iter()
                        .map(|seg| PathSegment {
                            edge: seg.edge.clone(),
                            node: Self::augment_node_pattern(&seg.node, eqs),
                        })
                        .collect(),
                    shortest: pp.shortest,
                }),
            })
            .collect()
    }

    /// ノードパターンの変数に一致する等価述語をプロパティ制約として追加する。
    /// 既存のインラインプロパティは上書きしない（両方を AND として扱う）。
    fn augment_node_pattern(np: &NodePattern, eqs: &[(String, String, Expression)]) -> NodePattern {
        let var = match &np.variable {
            Some(v) => v,
            None => return np.clone(),
        };
        let mut properties = np.properties.clone();
        for (v, prop, val) in eqs {
            if v == var {
                properties
                    .entry(prop.clone())
                    .or_insert_with(|| val.clone());
            }
        }
        NodePattern {
            variable: np.variable.clone(),
            labels: np.labels.clone(),
            properties,
            properties_param: np.properties_param.clone(),
        }
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
                    for (col, value) in col_names.iter().zip(row.columns) {
                        match value {
                            Value::Node(id) | Value::NodeData { id, .. } => {
                                new_binding.insert(Arc::from(col.as_str()), BindingValue::Node(id));
                            }
                            Value::Path { nodes, edges } => {
                                new_binding.insert(
                                    Arc::from(col.as_str()),
                                    BindingValue::Path { nodes, edges },
                                );
                            }
                            other => {
                                new_binding
                                    .insert(Arc::from(col.as_str()), BindingValue::Scalar(other));
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
        let mut keys: Vec<Vec<Value>> = Vec::new();
        let mut order_slots: Option<Vec<Option<usize>>> = None;

        for binding in &bindings {
            let mut new_binding = Bindings::new();
            let mut projected = Vec::with_capacity(with_clause.items.len());

            for (item, var_name) in with_clause.items.iter().zip(col_names.iter()) {
                let value = self.evaluate_return_item(&item.expression, binding)?;
                if with_clause.order_by.is_some() {
                    projected.push(value.clone());
                }

                // `WITH x` / `WITH x AS y` passes the binding through unchanged, so a
                // relationship stays a relationship (a `Value` cannot represent one).
                if let ReturnItem::Variable(v) = &item.expression
                    && let Some(bound) = binding.get(v.as_str())
                {
                    new_binding.insert(Arc::from(var_name.as_str()), bound.clone());
                    continue;
                }

                // Convert Value back to BindingValue for the new binding
                match value {
                    Value::Node(id) | Value::NodeData { id, .. } => {
                        new_binding.insert(Arc::from(var_name.as_str()), BindingValue::Node(id));
                    }
                    Value::Path { nodes, edges } => {
                        new_binding.insert(
                            Arc::from(var_name.as_str()),
                            BindingValue::Path { nodes, edges },
                        );
                    }
                    other => {
                        new_binding
                            .insert(Arc::from(var_name.as_str()), BindingValue::Scalar(other));
                    }
                }
            }

            if let Some(ref order_by) = with_clause.order_by {
                let row = Row { columns: projected };
                // ORDER BY may use both the incoming variables and the WITH
                // aliases (e.g. `WITH r AS rel ORDER BY rel.since`).
                let mut scope = binding.clone();
                scope.extend(new_binding.iter().map(|(k, v)| (k.clone(), v.clone())));
                let slots =
                    order_slots.get_or_insert_with(|| self.order_key_slots(order_by, &col_names));
                keys.push(self.order_keys(order_by, slots, &row, Some(&scope))?);
            }
            result.push(new_binding);
        }

        // Apply DISTINCT if needed (keeping ORDER BY keys aligned)
        if with_clause.distinct {
            let mut seen = std::collections::HashSet::new();
            let has_keys = !keys.is_empty();
            let mut kept = Vec::new();
            let mut kept_keys = Vec::new();
            let mut keys_iter = keys.into_iter();
            for binding in result {
                let key = has_keys.then(|| keys_iter.next().unwrap_or_default());
                if seen.insert(Self::bindings_key(&binding)) {
                    kept.push(binding);
                    kept_keys.extend(key);
                }
            }
            result = kept;
            keys = kept_keys;
        }

        // Apply ORDER BY (was previously ignored for non-aggregating WITH)
        if let Some(ref order_by) = with_clause.order_by {
            let mut pairs: Vec<(Vec<Value>, Bindings)> = keys.into_iter().zip(result).collect();
            pairs.sort_by(|a, b| self.compare_keys(&a.0, &b.0, order_by));
            result = pairs.into_iter().map(|(_, b)| b).collect();
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

    /// DISTINCT 判定用のバインディングのキー。`HashMap` の走査順はインスタンスごとに
    /// 異なるため（RandomState）、Debug 表示をそのまま使うと同じ内容でも別キーになる。
    /// 変数名でソートしてから文字列化する。
    fn bindings_key(binding: &Bindings) -> String {
        let mut entries: Vec<_> = binding.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        format!("{:?}", entries)
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
            ScalarFunction::Math(f, _) => f.name().to_string(),
            ScalarFunction::Math2(f, ..) => f.name().to_string(),
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
                    // No matches found - keep the original bindings and bind the
                    // clause's new variables to null.
                    let mut bindings = bindings;
                    for var in pattern_variables(&clause.patterns) {
                        if !bindings.contains_key(var.as_str()) {
                            bindings
                                .insert(Arc::from(var.as_str()), BindingValue::Scalar(Value::Null));
                        }
                    }
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

    /// スキャン候補を範囲索引で絞り込む（該当する範囲ヒントと索引がある場合）。
    ///
    /// 戻り値は「WHERE を満たすノードを必ず含むスーパーセット」。最終的な正当性は
    /// `node_matches_pattern` と WHERE の retain が保証するため、多少広くても問題ない。
    /// int/float の両範囲索引を union するので、プロパティの数値型が混在していても
    /// 取りこぼさない。
    fn range_index_candidates(&self, pattern: &NodePattern) -> Option<Vec<NodeId>> {
        // 数値 f を i64 に安全に丸める（範囲外はクランプ）。
        fn floor_i64(f: f64) -> i64 {
            if f >= i64::MAX as f64 {
                i64::MAX
            } else if f <= i64::MIN as f64 {
                i64::MIN
            } else {
                f.floor() as i64
            }
        }
        fn ceil_i64(f: f64) -> i64 {
            if f >= i64::MAX as f64 {
                i64::MAX
            } else if f <= i64::MIN as f64 {
                i64::MIN
            } else {
                f.ceil() as i64
            }
        }

        let var = pattern.variable.as_deref()?;
        let label = pattern.labels.first()?;

        for (v, prop, op, val) in &self.range_hints {
            if v != var || !self.property_index.has_index(label, prop) {
                continue;
            }
            let f = match val {
                PropertyValue::Int(n) => *n as f64,
                PropertyValue::Float(x) => *x,
                _ => continue,
            };
            let lower_open = matches!(op, BinaryOp::Gt | BinaryOp::Gte);
            let upper_open = matches!(op, BinaryOp::Lt | BinaryOp::Lte);

            let mut ids: Vec<NodeId> = Vec::new();
            if lower_open {
                ids.extend(
                    self.property_index
                        .find_by_int_range(prop, floor_i64(f), i64::MAX),
                );
                ids.extend(self.property_index.find_by_float_range(prop, f, f64::MAX));
            } else if upper_open {
                ids.extend(
                    self.property_index
                        .find_by_int_range(prop, i64::MIN, ceil_i64(f)),
                );
                ids.extend(self.property_index.find_by_float_range(prop, f64::MIN, f));
            } else {
                continue;
            }

            ids.sort_unstable();
            ids.dedup();
            return Some(ids);
        }
        None
    }

    fn match_node_pattern(
        &self,
        pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        // Candidate node set for the scan path, computed on first use and reused
        // across input bindings. It is built lazily because bound variables and
        // property-index lookups never scan: listing every node of the label
        // would then dominate the cost of a point lookup.
        //
        // Priority:
        // 1. A WHERE range predicate on an indexed numeric property → use the
        //    range index for a candidate superset (narrows the scan a lot).
        // 2. Otherwise, if the pattern has a label → enumerate only that label's
        //    nodes via the label index.
        // 3. Otherwise → all node ids.
        //
        // `node_matches_pattern` re-verifies labels/properties and the WHERE
        // retain re-checks the exact predicate, so a candidate *superset* is
        // always safe.
        let scan_candidates = std::cell::OnceCell::new();
        let scan_candidates = || -> &Vec<NodeId> {
            scan_candidates.get_or_init(|| {
                self.range_index_candidates(pattern).unwrap_or_else(|| {
                    match pattern.labels.first() {
                        Some(label) => self.graph_ref().nodes_by_label(label),
                        None => self.graph_ref().node_ids(),
                    }
                })
            })
        };

        for bindings in current_bindings {
            // Check if variable is already bound
            if let Some(var) = &pattern.variable
                && let Some(bound_value) = bindings.get(var.as_str())
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
            //
            // The index holds every node, including ones an ACL session cannot
            // read. That is fine: index hits are only candidates, and
            // `node_matches_pattern` re-checks labels and property values
            // through `graph_ref()`, which is the ACL view when one is active.
            let mut used_index = false;
            if let Some(label) = pattern.labels.first() {
                'index_loop: for (prop_key, prop_expr) in &pattern.properties {
                    if self.property_index.has_index(label, prop_key)
                        && let Expression::Literal(lit) = prop_expr
                    {
                        let prop_val = PropertyValue::from(lit.clone());
                        let candidate_ids =
                            self.property_index.find_by_property(prop_key, &prop_val);
                        for node_id in candidate_ids {
                            if self.node_matches_pattern(node_id, pattern, &bindings)? {
                                let mut new_bindings = bindings.clone();
                                if let Some(var) = &pattern.variable {
                                    new_bindings.insert(
                                        Arc::from(var.as_str()),
                                        BindingValue::Node(node_id),
                                    );
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
            let all_node_ids = scan_candidates();
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
                        new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(node_id));
                    }
                    result.push(new_bindings);
                }
            } else {
                // Sequential path for small graphs.
                for &node_id in all_node_ids {
                    if self.node_matches_pattern(node_id, pattern, &bindings)? {
                        let mut new_bindings = bindings.clone();
                        if let Some(var) = &pattern.variable {
                            new_bindings
                                .insert(Arc::from(var.as_str()), BindingValue::Node(node_id));
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
        // Each hop continues from the node matched by the previous hop, so every
        // node on the path needs a binding. Anonymous nodes (`()`, `(:Label)`)
        // get a hidden variable name that no query can spell, which is removed
        // again once the whole path has matched.
        // A path variable additionally needs every relationship bound.
        let has_anonymous = pattern.start.variable.is_none()
            || pattern.segments.iter().any(|s| s.node.variable.is_none())
            || (pattern.variable.is_some()
                && pattern.segments.iter().any(|s| s.edge.variable.is_none()));
        let named;
        let pattern = if has_anonymous {
            let mut p = pattern.clone();
            let mut n = 0;
            let mut fresh = || {
                n += 1;
                Some(format!("{ANON_VAR_PREFIX}{n}"))
            };
            if p.start.variable.is_none() {
                p.start.variable = fresh();
            }
            let bind_edges = p.variable.is_some();
            for segment in &mut p.segments {
                if segment.node.variable.is_none() {
                    segment.node.variable = fresh();
                }
                if bind_edges && segment.edge.variable.is_none() {
                    segment.edge.variable = fresh();
                }
            }
            named = p;
            &named
        } else {
            pattern
        };

        let mut bindings = self.match_node_pattern(&pattern.start, current_bindings)?;
        let mut prev = &pattern.start;
        for segment in &pattern.segments {
            bindings = match pattern.shortest {
                Some(kind) => self.match_shortest_segment(segment, prev, bindings, kind)?,
                None => self.match_segment(segment, prev, bindings)?,
            };
            prev = &segment.node;
        }

        if let Some(path_var) = &pattern.variable {
            for b in &mut bindings {
                let path = Self::assemble_path(pattern, b)?;
                b.insert(Arc::from(path_var.as_str()), path);
            }
        }

        if has_anonymous {
            for b in &mut bindings {
                b.retain(|k, _| !k.starts_with(ANON_VAR_PREFIX));
            }
        }
        Ok(bindings)
    }

    /// Build the value of a path variable from the node / relationship
    /// bindings of a fully matched (and fully named) path pattern.
    fn assemble_path(pattern: &PathPattern, b: &Bindings) -> Result<BindingValue, ExecuteError> {
        let node_of = |var: &Option<String>| {
            var.as_deref()
                .and_then(|v| b.get(v))
                .and_then(|v| v.as_node())
                .ok_or_else(|| ExecuteError::TypeError("path node is not bound".to_string()))
        };
        let mut nodes = vec![node_of(&pattern.start.variable)?];
        let mut edges = Vec::new();
        for segment in &pattern.segments {
            match segment.edge.variable.as_deref().and_then(|v| b.get(v)) {
                Some(BindingValue::Edge(id)) => edges.push(*id),
                // Variable-length hop: its nodes include the hop's start node.
                Some(BindingValue::Path {
                    nodes: hop_nodes,
                    edges: hop_edges,
                }) => {
                    edges.extend(hop_edges.iter().copied());
                    nodes.extend(hop_nodes.iter().skip(1).copied());
                    continue;
                }
                _ => {
                    return Err(ExecuteError::TypeError(
                        "path relationship is not bound".to_string(),
                    ));
                }
            }
            nodes.push(node_of(&segment.node.variable)?);
        }
        Ok(BindingValue::Path { nodes, edges })
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
            .get(prev_var.as_str())
            .and_then(|v| v.as_node())
            .ok_or_else(|| ExecuteError::UndefinedVariable(prev_var.clone()))?;

        // Get edges from previous node
        let edges = self.get_edges_by_direction(prev_id, segment.edge.direction);

        for edge in edges {
            // Check edge type
            if !segment.edge.matches_type(&edge.label) {
                continue;
            }
            if !self.edge_matches_properties(&edge, &segment.edge, bindings)? {
                continue;
            }

            // A relationship variable bound earlier only matches that relationship.
            if let Some(bound) = segment
                .edge
                .variable
                .as_deref()
                .and_then(|v| bindings.get(v))
                && !matches!(bound, BindingValue::Edge(id) if *id == edge.id)
            {
                continue;
            }

            // Get the other node
            let next_id = self.get_next_node(prev_id, &edge, segment.edge.direction);

            // Check if next node matches pattern
            if self.node_matches_pattern(next_id, &segment.node, bindings)? {
                let mut new_bindings = bindings.clone();

                if let Some(var) = &segment.node.variable {
                    new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(next_id));
                }
                if let Some(var) = &segment.edge.variable {
                    new_bindings.insert(Arc::from(var.as_str()), BindingValue::Edge(edge.id));
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
                .get(prev_var.as_str())
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
                    if !segment.edge.matches_type(&edge.label) {
                        continue;
                    }
                    if !self.edge_matches_properties(&edge, &segment.edge, &bindings)? {
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
                    new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(end_id));
                }

                // Bind the path variable (edge list) if specified
                if let Some(var) = &segment.edge.variable {
                    new_bindings.insert(
                        Arc::from(var.as_str()),
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

    /// `shortestPath` / `allShortestPaths` の 1 セグメントを BFS で解く。
    ///
    /// 始点ごとに幅優先で探索し、終点パターンに合うノードそれぞれについて
    /// 最短経路（`All` なら同じ長さの経路すべて）を束縛する。探索は始点からの
    /// 距離で層ごとに進むため、各ノードに最初に届いた層がその最短距離になる。
    fn match_shortest_segment(
        &self,
        segment: &PathSegment,
        prev_pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
        kind: ShortestPathKind,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let (min, max) = match &segment.edge.length_range {
            Some(range) => (range.min, range.max),
            None => (1, Some(1)),
        };
        if min > 1 {
            return Err(ExecuteError::TypeError(
                "shortestPath() supports only a minimum length of 0 or 1".to_string(),
            ));
        }
        let prev_var = prev_pattern
            .variable
            .as_ref()
            .ok_or_else(|| ExecuteError::TypeError("path pattern requires variable".to_string()))?;

        let mut result = Vec::new();
        for bindings in current_bindings {
            let start_id = bindings
                .get(prev_var.as_str())
                .and_then(|v| v.as_node())
                .ok_or_else(|| ExecuteError::UndefinedVariable(prev_var.clone()))?;
            // When the end node is already bound only its layer needs to be reached.
            let bound_end = segment
                .node
                .variable
                .as_deref()
                .and_then(|v| bindings.get(v))
                .and_then(|v| v.as_node());

            // dist[n]: hop count from start; preds[n]: (previous node, edge) on a shortest path.
            let mut dist: HashMap<NodeId, u32> = HashMap::from([(start_id, 0)]);
            let mut preds: HashMap<NodeId, Vec<(NodeId, u64)>> = HashMap::new();
            let mut targets = Vec::new();
            let mut frontier = vec![start_id];
            let mut depth = 0u32;
            while !frontier.is_empty() {
                for &node in &frontier {
                    if depth >= min && self.node_matches_pattern(node, &segment.node, &bindings)? {
                        targets.push(node);
                    }
                }
                if bound_end.is_some_and(|end| dist.contains_key(&end))
                    || max.is_some_and(|m| depth >= m)
                {
                    break;
                }
                let mut next = Vec::new();
                for &node in &frontier {
                    for edge in self.get_edges_by_direction(node, segment.edge.direction) {
                        if !segment.edge.matches_type(&edge.label) {
                            continue;
                        }
                        if !self.edge_matches_properties(&edge, &segment.edge, &bindings)? {
                            continue;
                        }
                        let to = self.get_next_node(node, &edge, segment.edge.direction);
                        match dist.get(&to) {
                            None => {
                                dist.insert(to, depth + 1);
                                preds.insert(to, vec![(node, edge.id)]);
                                next.push(to);
                            }
                            Some(&d) if d == depth + 1 && kind == ShortestPathKind::All => {
                                preds.entry(to).or_default().push((node, edge.id));
                            }
                            Some(_) => {}
                        }
                    }
                }
                frontier = next;
                depth += 1;
            }

            for end_id in targets {
                for (nodes, edges) in Self::shortest_paths_to(&preds, start_id, end_id, kind) {
                    let mut new_bindings = bindings.clone();
                    if let Some(var) = &segment.node.variable {
                        new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(end_id));
                    }
                    if let Some(var) = &segment.edge.variable {
                        new_bindings
                            .insert(Arc::from(var.as_str()), BindingValue::Path { nodes, edges });
                    }
                    result.push(new_bindings);
                }
            }
        }
        Ok(result)
    }

    /// BFS の先行リストから `start` → `end` の最短経路を復元する
    /// （`Single` なら 1 本、`All` ならすべて）。戻り値は (ノード列, エッジ列)。
    fn shortest_paths_to(
        preds: &HashMap<NodeId, Vec<(NodeId, u64)>>,
        start: NodeId,
        end: NodeId,
        kind: ShortestPathKind,
    ) -> Vec<(Vec<NodeId>, Vec<u64>)> {
        // Partial paths are built backwards from `end`.
        let mut done = Vec::new();
        let mut stack = vec![(vec![end], Vec::new())];
        while let Some((mut nodes, mut edges)) = stack.pop() {
            let head = *nodes.last().unwrap();
            if head == start {
                nodes.reverse();
                edges.reverse();
                done.push((nodes, edges));
                if kind == ShortestPathKind::Single {
                    break;
                }
                continue;
            }
            let Some(ps) = preds.get(&head) else { continue };
            for &(prev, edge) in ps {
                let mut n = nodes.clone();
                let mut e = edges.clone();
                n.push(prev);
                e.push(edge);
                stack.push((n, e));
            }
        }
        done
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
        // A variable bound earlier (previous pattern, WITH, subquery import)
        // only matches that same node.
        if let Some(bound) = pattern.variable.as_deref().and_then(|v| bindings.get(v))
            && bound.as_node() != Some(node_id)
        {
            return Ok(false);
        }

        let backend = self.graph_ref();

        // Check labels without cloning the whole node. A node must have ALL
        // labels specified in the pattern (AND). When the pattern has labels,
        // this also confirms the node exists.
        if !pattern.labels.is_empty() {
            if !backend.node_has_all_labels(node_id, &pattern.labels) {
                return Ok(false);
            }
        } else if pattern.properties.is_empty()
            && pattern.properties_param.is_none()
            && !backend.contains_node(node_id)
        {
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
        for (key, expected_expr) in self.param_property_exprs(&pattern.properties_param)? {
            match backend.get_node_property(node_id, &key) {
                Some(actual) => {
                    let expected_val = self.evaluate_expression(&expected_expr, bindings)?;
                    if !self.property_value_matches(&actual, &expected_val) {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }

        Ok(true)
    }

    /// パターンの `$props` を、マップの各キーを参照する式（`$props['key']`）の
    /// 並びに展開する。パラメータが無ければ空。マップ以外ならエラー。
    fn param_property_exprs(
        &self,
        param: &Option<String>,
    ) -> Result<Vec<(String, Expression)>, ExecuteError> {
        let Some(name) = param else {
            return Ok(Vec::new());
        };
        match self.params.get(name) {
            Some(Value::Map(map)) => Ok(map
                .keys()
                .map(|k| {
                    (
                        k.clone(),
                        Expression::IndexAccess(
                            Box::new(Expression::Parameter(name.clone())),
                            Box::new(Expression::Literal(Literal::String(k.clone()))),
                        ),
                    )
                })
                .collect()),
            Some(other) => Err(ExecuteError::TypeError(format!(
                "parameter ${name} used as pattern properties must be a map, got {other:?}"
            ))),
            None => Err(ExecuteError::UndefinedVariable(format!("${name}"))),
        }
    }

    /// パターンのインラインプロパティと `$props` の展開をまとめて返す。
    fn pattern_property_exprs(
        &self,
        properties: &HashMap<String, Expression>,
        param: &Option<String>,
    ) -> Result<Vec<(String, Expression)>, ExecuteError> {
        let mut out: Vec<(String, Expression)> = properties
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.extend(self.param_property_exprs(param)?);
        Ok(out)
    }

    /// リレーションシップがパターンのインラインプロパティ（`-[r {w: 1}]-`）を満たすか。
    fn edge_matches_properties(
        &self,
        edge: &Edge,
        pattern: &EdgePattern,
        bindings: &Bindings,
    ) -> Result<bool, ExecuteError> {
        if pattern.properties.is_empty() && pattern.properties_param.is_none() {
            return Ok(true);
        }
        for (key, expected_expr) in
            self.pattern_property_exprs(&pattern.properties, &pattern.properties_param)?
        {
            let Some(actual) = edge.properties.get(&key) else {
                return Ok(false);
            };
            let expected = self.evaluate_expression(&expected_expr, bindings)?;
            if !self.property_value_matches(actual, &expected) {
                return Ok(false);
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
            // リスト・日付など: 値に直して比べる
            (PropertyValue::List(_), Value::List(_)) => {
                self.values_equal(&Value::from(actual), expected)
            }
            _ => false,
        }
    }

    fn build_result_set(
        &self,
        return_clause: &ReturnClause,
        bindings_list: &[Bindings],
    ) -> Result<ResultSet, ExecuteError> {
        // Check if any aggregation is present
        let has_aggregation = return_clause.items.iter().any(Self::is_aggregate);

        if has_aggregation {
            return self.build_aggregated_result_set(return_clause, bindings_list);
        }

        // Build column names
        let columns: Vec<String> = return_clause
            .items
            .iter()
            .map(|item| self.return_item_to_column_name(item))
            .collect();

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
        // With ORDER BY and LIMIT, only the first `needed` rows are kept.
        let needed = match (resolved_skip, resolved_limit) {
            (Some(skip), Some(limit)) => Some(skip.saturating_add(limit) as usize),
            (None, Some(limit)) => Some(limit as usize),
            _ => None,
        };
        let order_slots = return_clause
            .order_by
            .as_ref()
            .map(|order_by| self.order_key_slots(order_by, &columns));

        // ORDER BY ... LIMIT without DISTINCT: sort on the keys alone and
        // evaluate the RETURN items only for the rows that remain. (DISTINCT
        // needs every row's values before it can drop duplicates.)
        if let (Some(order_by), Some(slots), Some(needed)) =
            (&return_clause.order_by, &order_slots, needed)
            && !return_clause.distinct
            && needed < bindings_list.len()
        {
            let mut rows =
                self.build_top_rows(return_clause, order_by, slots, bindings_list, needed)?;
            if let Some(skip) = resolved_skip {
                rows.drain(..(skip as usize).min(rows.len()));
            }
            return Ok(ResultSet::new(columns, rows));
        }

        // Build rows (and ORDER BY keys, which may refer to values that are
        // not returned, e.g. `RETURN n.name ORDER BY n.age`).
        let mut rows = Vec::new();
        let mut keys = Vec::new();

        for bindings in bindings_list {
            let mut row_values = Vec::new();

            for item in &return_clause.items {
                row_values.push(self.evaluate_return_item(item, bindings)?);
            }

            let row = Row {
                columns: row_values,
            };
            if let (Some(order_by), Some(slots)) = (&return_clause.order_by, &order_slots) {
                keys.push(self.order_keys(order_by, slots, &row, Some(bindings))?);
            }
            rows.push(row);
        }

        // Apply DISTINCT (keeping each surviving row's ORDER BY keys aligned)
        if return_clause.distinct {
            let mut seen = std::collections::HashSet::new();
            let has_keys = !keys.is_empty();
            let mut kept_rows = Vec::new();
            let mut kept_keys = Vec::new();
            let mut keys_iter = keys.into_iter();
            for row in rows {
                let key = has_keys.then(|| keys_iter.next().unwrap_or_default());
                if seen.insert(self.row_to_key(&row)) {
                    kept_rows.push(row);
                    kept_keys.extend(key);
                }
            }
            rows = kept_rows;
            keys = kept_keys;
        }

        // Apply ORDER BY with optional LIMIT optimization
        if let Some(ref order_by) = return_clause.order_by {
            rows = self.sort_rows_by_keys(rows, keys, order_by, needed);
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

    fn row_to_key(&self, row: &Row) -> String {
        row.columns
            .iter()
            .map(|v| format!("{:?}", v))
            .collect::<Vec<_>>()
            .join("|")
    }

    /// ORDER BY の各キーの値を求める。
    ///
    /// キーが返却列（列名が一致）ならその値を使う。そうでなければ射影前の
    /// バインディング `source` から評価する（`RETURN n.name ORDER BY n.age`）。
    /// `source` が無い（集計後）場合に返却列でないキーはエラーにする。以前は
    /// 列が見つからないキーを黙って無視し、並べ替えずに結果を返していた。
    ///
    /// `slots` は [`order_key_slots`](Self::order_key_slots) の結果（行ごとに
    /// 列名を作って探さないよう、呼び出し側が 1 回だけ求める）。
    fn order_keys(
        &self,
        order_by: &OrderByClause,
        slots: &[Option<usize>],
        row: &Row,
        source: Option<&Bindings>,
    ) -> Result<Vec<Value>, ExecuteError> {
        order_by
            .items
            .iter()
            .zip(slots)
            .map(|(item, slot)| {
                if let Some(idx) = slot {
                    return Ok(row.columns[*idx].clone());
                }
                match source {
                    Some(bindings) => self.evaluate_return_item(&item.expression, bindings),
                    None => Err(ExecuteError::TypeError(format!(
                        "ORDER BY {}: with aggregation, ORDER BY can only refer to returned columns (add it to RETURN or use an alias)",
                        self.return_item_to_column_name(&item.expression)
                    ))),
                }
            })
            .collect()
    }

    /// ORDER BY の各キーが返却列のどれに当たるか（当たらなければ `None`）。
    fn order_key_slots(&self, order_by: &OrderByClause, columns: &[String]) -> Vec<Option<usize>> {
        order_by
            .items
            .iter()
            .map(|item| {
                let name = self.return_item_to_column_name(&item.expression);
                columns.iter().position(|c| *c == name)
            })
            .collect()
    }

    /// `ORDER BY … LIMIT` で残る行だけを組み立てる。
    ///
    /// 全行について並べ替えキーだけを求め、上位 `needed` 件を選んでから、その行の
    /// RETURN の式を評価する。残らない行の RETURN の式は評価しない。
    fn build_top_rows(
        &self,
        return_clause: &ReturnClause,
        order_by: &OrderByClause,
        slots: &[Option<usize>],
        bindings_list: &[Bindings],
        needed: usize,
    ) -> Result<Vec<Row>, ExecuteError> {
        if needed == 0 {
            return Ok(Vec::new());
        }
        let mut keyed: Vec<(Vec<Value>, usize)> = Vec::with_capacity(bindings_list.len());
        for (i, bindings) in bindings_list.iter().enumerate() {
            let key = order_by
                .items
                .iter()
                .zip(slots)
                .map(|(item, slot)| match slot {
                    Some(idx) => self.evaluate_return_item(&return_clause.items[*idx], bindings),
                    None => self.evaluate_return_item(&item.expression, bindings),
                })
                .collect::<Result<Vec<_>, _>>()?;
            keyed.push((key, i));
        }
        // 同じキーの行は入力の順に並べる（全件を安定ソートしたときと同じ結果にする）
        let cmp = |a: &(Vec<Value>, usize), b: &(Vec<Value>, usize)| {
            self.compare_keys(&a.0, &b.0, order_by).then(a.1.cmp(&b.1))
        };
        if needed < keyed.len() {
            keyed.select_nth_unstable_by(needed, cmp);
            keyed.truncate(needed);
        }
        keyed.sort_unstable_by(cmp);
        keyed
            .into_iter()
            .map(|(_, i)| {
                let columns = return_clause
                    .items
                    .iter()
                    .map(|item| self.evaluate_return_item(item, &bindings_list[i]))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Row { columns })
            })
            .collect()
    }

    fn compare_keys(
        &self,
        a: &[Value],
        b: &[Value],
        order_by: &OrderByClause,
    ) -> std::cmp::Ordering {
        for (i, item) in order_by.items.iter().enumerate() {
            let cmp = self.compare_values_for_sort(&a[i], &b[i], item.direction, item.nulls_order);
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    }

    /// `keys[i]` を `rows[i]` の並べ替えキーとしてソートする。`needed` があり行数より
    /// 少なければ上位 `needed` 件だけを部分選択してからソートする（TopN 最適化）。
    fn sort_rows_by_keys(
        &self,
        rows: Vec<Row>,
        keys: Vec<Vec<Value>>,
        order_by: &OrderByClause,
        needed: Option<usize>,
    ) -> Vec<Row> {
        let mut pairs: Vec<(Vec<Value>, Row)> = keys.into_iter().zip(rows).collect();
        let cmp =
            |a: &(Vec<Value>, Row), b: &(Vec<Value>, Row)| self.compare_keys(&a.0, &b.0, order_by);
        if let Some(n) = needed
            && n < pairs.len()
        {
            if n == 0 {
                return Vec::new();
            }
            pairs.select_nth_unstable_by(n, cmp);
            pairs.truncate(n);
        }
        pairs.sort_by(cmp);
        pairs.into_iter().map(|(_, row)| row).collect()
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

    /// RETURN 項目の結果列名（エイリアス、または式の表示形）。シャーディングの
    /// コーディネーターがシャードの結果を列名で突き合わせるために使う。
    pub fn column_name(item: &ReturnItem) -> String {
        match item {
            ReturnItem::Alias(_, name) => name.clone(),
            ReturnItem::Variable(v) => v.clone(),
            ReturnItem::Property(v, p) => format!("{}.{}", v, p),
            ReturnItem::All => "*".to_string(),
            ReturnItem::Expr(e) => Self::expression_to_display(e),
            ReturnItem::Aggregate(agg) => match agg {
                AggregateFunction::Count(inner) => match inner {
                    None => "COUNT(*)".to_string(),
                    Some(inner) => format!("count({})", Self::column_name(inner)),
                },
                AggregateFunction::Sum(inner) => {
                    format!("SUM({})", Self::column_name(inner))
                }
                AggregateFunction::Avg(inner) => {
                    format!("AVG({})", Self::column_name(inner))
                }
                AggregateFunction::Min(inner) => {
                    format!("MIN({})", Self::column_name(inner))
                }
                AggregateFunction::Max(inner) => {
                    format!("MAX({})", Self::column_name(inner))
                }
                AggregateFunction::Collect(inner) => {
                    format!("COLLECT({})", Self::column_name(inner))
                }
                AggregateFunction::PercentileCont(e, p) => format!(
                    "percentileCont({}, {})",
                    Self::column_name(e),
                    Self::column_name(p)
                ),
                AggregateFunction::PercentileDisc(e, p) => format!(
                    "percentileDisc({}, {})",
                    Self::column_name(e),
                    Self::column_name(p)
                ),
                AggregateFunction::StDev(inner) => {
                    format!("stDev({})", Self::column_name(inner))
                }
                AggregateFunction::StDevP(inner) => {
                    format!("stDevP({})", Self::column_name(inner))
                }
                AggregateFunction::CountDistinct(inner) => {
                    format!("COUNT(DISTINCT {})", Self::column_name(inner))
                }
                AggregateFunction::SumDistinct(inner) => {
                    format!("SUM(DISTINCT {})", Self::column_name(inner))
                }
                AggregateFunction::AvgDistinct(inner) => {
                    format!("AVG(DISTINCT {})", Self::column_name(inner))
                }
                AggregateFunction::CollectDistinct(inner) => {
                    format!("COLLECT(DISTINCT {})", Self::column_name(inner))
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
                ScalarFunction::Math(f, e) => {
                    format!("{}({})", f.name(), Self::expression_to_display(e))
                }
                ScalarFunction::Math2(f, a, b) => format!(
                    "{}({}, {})",
                    f.name(),
                    Self::expression_to_display(a),
                    Self::expression_to_display(b)
                ),
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

    fn return_item_to_column_name(&self, item: &ReturnItem) -> String {
        Self::column_name(item)
    }

    fn evaluate_return_item(
        &self,
        item: &ReturnItem,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        match item {
            ReturnItem::Variable(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                            Ok(Value::Edge(*edge_id))
                        }
                        BindingValue::Path { nodes, edges } => Ok(Value::Path {
                            nodes: nodes.clone(),
                            edges: edges.clone(),
                        }),
                        BindingValue::Scalar(value) => Ok(value.clone()),
                    }
                } else {
                    // Not in scope (e.g. dropped by WITH): an error, not null (bug/162)
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ReturnItem::Property(var, prop) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                Ok(node
                                    .get_property(prop)
                                    .map(Value::from)
                                    .unwrap_or(Value::Null))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                Ok(edge
                                    .get_property(prop)
                                    .map(Value::from)
                                    .unwrap_or(Value::Null))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        // Maps, nodes and relationships held as values, temporal fields
                        BindingValue::Scalar(_) => self.evaluate_expression(
                            &Expression::Property(var.clone(), prop.clone()),
                            bindings,
                        ),
                        _ => Ok(Value::Null),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    if let BindingValue::Path { edges, .. } = binding_value {
                        // Return list of edge IDs (or edge data if available)
                        let edge_values: Vec<Value> =
                            edges.iter().map(|&edge_id| Value::Edge(edge_id)).collect();
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                    .get(start.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(start.clone()))?;
                let end_id = bindings
                    .get(end.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(end.clone()))?;

                if let Some(path) = traversal::shortest_path(self.graph_ref(), start_id, end_id) {
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
                    .get(start.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(start.clone()))?;
                let end_id = bindings
                    .get(end.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(end.clone()))?;

                let paths = traversal::all_shortest_paths(self.graph_ref(), start_id, end_id);
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
                    Value::Int(n) => n
                        .checked_abs()
                        .map(Value::Int)
                        .ok_or_else(|| ExecuteError::TypeError("integer overflow".to_string())),
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
            ScalarFunction::Math(f, expr) => match self.evaluate_expression(expr, bindings)? {
                Value::Int(n) => Ok(Value::Float(f.apply(n as f64))),
                Value::Float(n) => Ok(Value::Float(f.apply(n))),
                Value::Null => Ok(Value::Null),
                _ => Err(ExecuteError::TypeError(format!(
                    "{}() requires a numeric value",
                    f.name()
                ))),
            },
            ScalarFunction::Math2(f, a, b) => {
                let as_f64 = |v: Value| match v {
                    Value::Int(n) => Ok(Some(n as f64)),
                    Value::Float(n) => Ok(Some(n)),
                    Value::Null => Ok(None),
                    _ => Err(ExecuteError::TypeError(format!(
                        "{}() requires numeric values",
                        f.name()
                    ))),
                };
                let a = as_f64(self.evaluate_expression(a, bindings)?)?;
                let b = as_f64(self.evaluate_expression(b, bindings)?)?;
                match (a, b) {
                    (Some(a), Some(b)) => Ok(Value::Float(f.apply(a, b))),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::E => Ok(Value::Float(std::f64::consts::E)),
            ScalarFunction::Pi => Ok(Value::Float(std::f64::consts::PI)),
            ScalarFunction::Id(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                        // Compute the element count up-front in i128 (so the
                        // subtraction cannot overflow i64) and reject ranges
                        // that would exhaust memory before allocating.
                        let count: i128 = if step > 0 {
                            if end < start {
                                0
                            } else {
                                (end as i128 - start as i128) / step as i128 + 1
                            }
                        } else if end > start {
                            0
                        } else {
                            (start as i128 - end as i128) / (-(step as i128)) + 1
                        };
                        if count > MAX_RANGE_LEN as i128 {
                            return Err(ExecuteError::TypeError(format!(
                                "range() would produce {} elements, exceeding the limit of {}",
                                count, MAX_RANGE_LEN
                            )));
                        }
                        let mut result = Vec::with_capacity(count as usize);
                        let mut i = start;
                        // `checked_add` guards against i64 overflow near the
                        // bounds, which would otherwise wrap and loop forever.
                        if step > 0 {
                            while i <= end {
                                result.push(Value::Int(i));
                                match i.checked_add(step) {
                                    Some(next) => i = next,
                                    None => break,
                                }
                            }
                        } else {
                            while i >= end {
                                result.push(Value::Int(i));
                                match i.checked_add(step) {
                                    Some(next) => i = next,
                                    None => break,
                                }
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
                    local_bindings.insert(Arc::from(acc_var.as_str()), BindingValue::Scalar(acc));
                    local_bindings.insert(Arc::from(item_var.as_str()), BindingValue::Scalar(item));
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
                            Value::String(s) => match temporal::parse_date(&s) {
                                Some(d) => Ok(Value::Date(d)),
                                None => Err(ExecuteError::TypeError(format!(
                                    "invalid date string: {}",
                                    s
                                ))),
                            },
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
                            _ => Err(ExecuteError::TypeError(
                                "date() requires a string or map argument".to_string(),
                            )),
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
                            Value::String(s) => match temporal::parse_datetime(&s) {
                                Some(ms) => Ok(Value::DateTime(ms)),
                                None => Err(ExecuteError::TypeError(format!(
                                    "invalid datetime string: {}",
                                    s
                                ))),
                            },
                            Value::DateTime(ms) => Ok(Value::DateTime(ms)),
                            _ => Err(ExecuteError::TypeError(
                                "datetime() requires a string argument".to_string(),
                            )),
                        }
                    }
                }
            }
            ScalarFunction::DurationFunc(expr) => {
                use maharit_core::temporal;
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => match temporal::parse_duration(&s) {
                        Some((months, days, millis)) => Ok(Value::Duration {
                            months,
                            days,
                            millis,
                        }),
                        None => Err(ExecuteError::TypeError(format!(
                            "invalid duration string: {}",
                            s
                        ))),
                    },
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
                        let total_millis =
                            hours * 3_600_000 + minutes * 60_000 + seconds * 1_000 + milliseconds;
                        Ok(Value::Duration {
                            months: total_months,
                            days: total_days,
                            millis: total_millis,
                        })
                    }
                    _ => Err(ExecuteError::TypeError(
                        "duration() requires a string or map argument".to_string(),
                    )),
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
            return Ok(ResultSet::new(
                columns,
                vec![Row {
                    columns: row_values,
                }],
            ));
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
            let group_bindings: Vec<Bindings> =
                indices.iter().map(|&i| bindings_list[i].clone()).collect();

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

            rows.push(Row {
                columns: row_values,
            });
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

            // After grouping there is no single source row per result row,
            // so keys must refer to returned columns.
            let slots = self.order_key_slots(order_by, &columns);
            let keys = rows
                .iter()
                .map(|row| self.order_keys(order_by, &slots, row, None))
                .collect::<Result<Vec<_>, _>>()?;
            rows = self.sort_rows_by_keys(rows, keys, order_by, needed);

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
                            .and_then(|b| b.get(var.as_str()))
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
                    let collected: Vec<Value> = bindings_list
                        .iter()
                        .filter_map(|bindings| {
                            self.evaluate_return_item(inner, bindings)
                                .ok()
                                .filter(|v| !matches!(v, Value::Null))
                        })
                        .collect();
                    Ok(Value::List(collected))
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
    fn resolve_skip_limit(&self, expr: &Expression) -> Result<u64, ExecuteError> {
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
            Value::Duration {
                months,
                days,
                millis,
            } => {
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
                    .get(var.as_str())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;
                match binding_value {
                    BindingValue::Node(id) => Ok(Value::Node(*id)),
                    BindingValue::Edge(id) => Ok(Value::Edge(*id)),
                    BindingValue::Path { nodes, edges } => Ok(Value::Path {
                        nodes: nodes.clone(),
                        edges: edges.clone(),
                    }),
                    BindingValue::Scalar(value) => Ok(value.clone()),
                }
            }
            Expression::Property(var, prop) => {
                let binding_value = bindings
                    .get(var.as_str())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;

                match binding_value {
                    BindingValue::Node(node_id) => {
                        let node = self
                            .graph_ref()
                            .get_node(*node_id)
                            .ok_or_else(|| ExecuteError::TypeError("node not found".to_string()))?;
                        Ok(node
                            .get_property(prop)
                            .map(Value::from)
                            .unwrap_or(Value::Null))
                    }
                    BindingValue::Edge(edge_id) => {
                        let edge = self
                            .graph_ref()
                            .get_edge(*edge_id)
                            .ok_or_else(|| ExecuteError::TypeError("edge not found".to_string()))?;
                        Ok(edge
                            .get_property(prop)
                            .map(Value::from)
                            .unwrap_or(Value::Null))
                    }
                    BindingValue::Scalar(scalar_val) => match scalar_val {
                        Value::Map(map) => Ok(map.get(prop).cloned().unwrap_or(Value::Null)),
                        // Nodes held as values, e.g. `[n IN nodes(p) | n.name]`.
                        Value::Node(node_id) => Ok(self
                            .graph_ref()
                            .get_node(*node_id)
                            .and_then(|node| node.get_property(prop).map(Value::from))
                            .unwrap_or(Value::Null)),
                        Value::NodeData { properties, .. } => {
                            Ok(properties.get(prop).map(Value::from).unwrap_or(Value::Null))
                        }
                        // Relationships held as values, e.g. `r IN relationships(p) | r.w`
                        Value::Edge(edge_id) => Ok(self.edge_property(*edge_id, prop)),
                        _ => Self::access_temporal_field(scalar_val, prop),
                    },
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
                // `r[key]`: リレーションシップは値にすると ID になるので、変数のまま扱う
                if let Expression::Variable(var) = list_expr.as_ref()
                    && let Some(BindingValue::Edge(id)) = bindings.get(var.as_str())
                {
                    return match self.evaluate_expression(index_expr, bindings)? {
                        Value::String(key) => Ok(self
                            .graph_ref()
                            .get_edge(*id)
                            .and_then(|e| e.properties.get(&key).map(Value::from))
                            .unwrap_or(Value::Null)),
                        Value::Null => Ok(Value::Null),
                        _ => Err(ExecuteError::TypeError(
                            "a relationship property key must be a string".to_string(),
                        )),
                    };
                }
                let list_val = self.evaluate_expression(list_expr, bindings)?;
                let idx_val = self.evaluate_expression(index_expr, bindings)?;
                match (list_val, idx_val) {
                    // 動的なプロパティ参照: `n["name"]`、`n[$key]`
                    (Value::Node(id), Value::String(key)) => Ok(self
                        .graph_ref()
                        .get_node_property(id, &key)
                        .map(|v| Value::from(&v))
                        .unwrap_or(Value::Null)),
                    (Value::NodeData { properties, .. }, Value::String(key)) => {
                        Ok(properties.get(&key).map(Value::from).unwrap_or(Value::Null))
                    }
                    (Value::Edge(id), Value::String(key)) => Ok(self.edge_property(id, &key)),
                    (Value::List(items), Value::Int(i)) => {
                        let idx = if i < 0 { items.len() as i64 + i } else { i };
                        if idx >= 0 {
                            Ok(items.get(idx as usize).cloned().unwrap_or(Value::Null))
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    // マップのキー参照: `$props['name']` / `$props.name`
                    (Value::Map(map), Value::String(key)) => {
                        Ok(map.get(&key).cloned().unwrap_or(Value::Null))
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
                    // `list[start..]`: 終わりが null なら末尾まで
                    (Value::List(items), Value::Int(s), Value::Null) => {
                        let len = items.len() as i64;
                        let s = if s < 0 { (len + s).max(0) } else { s.min(len) } as usize;
                        Ok(Value::List(items[s..].to_vec()))
                    }
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
                    local_bindings.insert(Arc::from(variable.as_str()), BindingValue::Scalar(item));
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
            Expression::HasLabels(var, labels) => match bindings.get(var.as_str()) {
                Some(BindingValue::Node(id)) => Ok(Value::Bool(
                    self.graph_ref().node_has_all_labels(*id, labels),
                )),
                // `r:KNOWS`: リレーションシップは型が一致するか
                Some(BindingValue::Edge(id)) => Ok(Value::Bool(
                    labels.len() == 1
                        && self
                            .graph_ref()
                            .get_edge(*id)
                            .is_some_and(|e| e.label == labels[0]),
                )),
                Some(BindingValue::Scalar(Value::Null)) => Ok(Value::Null),
                Some(_) => Err(ExecuteError::TypeError(format!(
                    "{var}:{} requires a node or relationship",
                    labels.join(":")
                ))),
                None => Err(ExecuteError::UndefinedVariable(var.clone())),
            },
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
                    local_bindings.insert(
                        Arc::from(variable.as_str()),
                        BindingValue::Scalar(item.clone()),
                    );
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
            BinaryOp::Xor => match (left, right) {
                (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a ^ b)),
                (Value::Null, Value::Bool(_) | Value::Null) | (Value::Bool(_), Value::Null) => {
                    Ok(Value::Null)
                }
                _ => Err(ExecuteError::TypeError("XOR requires booleans".to_string())),
            },
            BinaryOp::Add => {
                if let (Value::List(a), Value::List(b)) = (left, right) {
                    let mut result = a.clone();
                    result.extend(b.iter().cloned());
                    return Ok(Value::List(result));
                }
                // テンポラル演算: Date + Duration, DateTime + Duration
                match (left, right) {
                    (
                        Value::Date(d),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let new_days = temporal::add_duration_to_date(*d, *months, *days, *millis);
                        return Ok(Value::Date(new_days));
                    }
                    (
                        Value::DateTime(ms),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let (y, mo, day, h, mi, s, frac) = temporal::millis_to_datetime(*ms);
                        let base_days = temporal::ymd_to_days(y, mo, day);
                        let new_days = temporal::add_duration_to_date(base_days, *months, *days, 0);
                        let (ny, nmo, nd) = temporal::days_to_ymd(new_days);
                        let new_ms =
                            temporal::datetime_to_millis(ny, nmo, nd, h, mi, s, frac) + millis;
                        return Ok(Value::DateTime(new_ms));
                    }
                    _ => {}
                }
                // String concatenation: 'a' + 'b', 'n=' + 1
                match (left, right) {
                    (Value::String(a), Value::String(b)) => {
                        return Ok(Value::String(format!("{a}{b}")));
                    }
                    (
                        Value::String(a),
                        other @ (Value::Int(_) | Value::Float(_) | Value::Bool(_)),
                    ) => {
                        return Ok(Value::String(format!("{a}{other}")));
                    }
                    (
                        other @ (Value::Int(_) | Value::Float(_) | Value::Bool(_)),
                        Value::String(b),
                    ) => {
                        return Ok(Value::String(format!("{other}{b}")));
                    }
                    _ => {}
                }
                self.arithmetic_op(left, right, i64::checked_add, |a, b| a + b)
            }
            BinaryOp::Sub => {
                // テンポラル演算: Date - Date = Duration, DateTime - DateTime = Duration
                match (left, right) {
                    (Value::Date(a), Value::Date(b)) => {
                        let diff_days = a - b;
                        return Ok(Value::Duration {
                            months: 0,
                            days: diff_days,
                            millis: 0,
                        });
                    }
                    (Value::DateTime(a), Value::DateTime(b)) => {
                        let diff_ms = a - b;
                        return Ok(Value::Duration {
                            months: 0,
                            days: 0,
                            millis: diff_ms,
                        });
                    }
                    (
                        Value::Date(d),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let new_days =
                            temporal::add_duration_to_date(*d, -*months, -*days, -*millis);
                        return Ok(Value::Date(new_days));
                    }
                    (
                        Value::DateTime(ms),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let (y, mo, day, h, mi, s, frac) = temporal::millis_to_datetime(*ms);
                        let base_days = temporal::ymd_to_days(y, mo, day);
                        let new_days =
                            temporal::add_duration_to_date(base_days, -*months, -*days, 0);
                        let (ny, nmo, nd) = temporal::days_to_ymd(new_days);
                        let new_ms =
                            temporal::datetime_to_millis(ny, nmo, nd, h, mi, s, frac) - millis;
                        return Ok(Value::DateTime(new_ms));
                    }
                    _ => {}
                }
                self.arithmetic_op(left, right, i64::checked_sub, |a, b| a - b)
            }
            BinaryOp::Mul => self.arithmetic_op(left, right, i64::checked_mul, |a, b| a * b),
            BinaryOp::Div => {
                if matches!(right, Value::Int(0)) && matches!(left, Value::Int(_)) {
                    return Err(ExecuteError::TypeError("division by zero".to_string()));
                }
                self.arithmetic_op(left, right, i64::checked_div, |a, b| a / b)
            }
            BinaryOp::Mod => {
                if matches!(right, Value::Int(0)) && matches!(left, Value::Int(_)) {
                    return Err(ExecuteError::TypeError("division by zero".to_string()));
                }
                self.arithmetic_op(left, right, i64::checked_rem, |a, b| a % b)
            }
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
            (
                Value::Duration {
                    months: ma,
                    days: da,
                    millis: msa,
                },
                Value::Duration {
                    months: mb,
                    days: db,
                    millis: msb,
                },
            ) => ma == mb && da == db && msa == msb,
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.values_equal(x, y))
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
        F: Fn(i64, i64) -> Option<i64>,
        G: Fn(f64, f64) -> f64,
    {
        match (left, right) {
            // 整数演算は checked: オーバーフローや 0 除算で panic させない
            // （リリースビルドは panic=abort なので、サーバー全体が落ちる）
            (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
                .map(Value::Int)
                .ok_or_else(|| ExecuteError::TypeError("integer overflow".to_string())),
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
                Value::Int(n) => n
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or_else(|| ExecuteError::TypeError("integer overflow".to_string())),
                Value::Float(n) => Ok(Value::Float(-n)),
                _ => Err(ExecuteError::TypeError(
                    "negation requires number".to_string(),
                )),
            },
            UnaryOp::IsNull => Ok(Value::Bool(matches!(val, Value::Null))),
            UnaryOp::IsNotNull => Ok(Value::Bool(!matches!(val, Value::Null))),
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

/// Variables introduced by the patterns (node, relationship and path variables).
fn pattern_variables(patterns: &[Pattern]) -> Vec<String> {
    let mut vars = Vec::new();
    for pattern in patterns {
        match pattern {
            Pattern::Node(np) => vars.extend(np.variable.clone()),
            Pattern::Path(pp) => {
                vars.extend(pp.variable.clone());
                vars.extend(pp.start.variable.clone());
                for seg in &pp.segments {
                    vars.extend(seg.edge.variable.clone());
                    vars.extend(seg.node.variable.clone());
                }
            }
        }
    }
    vars
}

/// パターン中で作るリレーションシップすべてについて [`single_edge_type`] を検査する。
fn check_single_edge_types(pattern: &Pattern) -> Result<(), ExecuteError> {
    if let Pattern::Path(pp) = pattern {
        for seg in &pp.segments {
            single_edge_type(&seg.edge)?;
        }
    }
    Ok(())
}

/// CREATE / MERGE で作るリレーションシップの型。`[:A|B]` のように複数指定されていたらエラー。
fn single_edge_type(edge: &EdgePattern) -> Result<String, ExecuteError> {
    match edge.edge_types.as_slice() {
        [] => Ok(String::new()),
        [t] => Ok(t.clone()),
        _ => Err(ExecuteError::TypeError(format!(
            "a relationship created by CREATE or MERGE must have exactly one type, got {}",
            edge.type_display()
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)] // 3.14 等は PI の近似ではなくリテラルのテストデータ
mod tests;
