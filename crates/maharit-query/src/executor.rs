use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use maharit_core::{
    ConcurrentGraph, Constraint, ConstraintError, ConstraintManager, ConstraintType, EdgeEnds,
    EdgeId, FulltextError, FulltextManager, Graph, GraphBackend, IndexDefinition, NodeId,
    PropertyIndex, PropertyType, PropertyValue, traversal,
};
use rayon::prelude::*;
use regex::Regex;

use crate::ast::*;

mod aggregate;
mod algo;
mod bindings;
mod error;
mod explain;
mod expr;
mod functions;
mod match_;
mod procedure;
mod query;
mod result;
mod schema;
mod value;
mod write;

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

    /// ノードに接続しているエッジの本数（自己ループは 1 本と数える）。
    fn incident_edge_count(&self, node_id: NodeId) -> i64 {
        let graph = self.graph_ref();
        let mut edges = Vec::new();
        graph.adjacent_edges(node_id, true, &[], &mut edges);
        graph.adjacent_edges(node_id, false, &[], &mut edges);
        edges
            .iter()
            .map(|e| e.id)
            .collect::<HashSet<EdgeId>>()
            .len() as i64
    }

    /// Return an exclusive reference to the graph backend.
    ///
    /// Must only be called when `self.readonly == false`.  For `ConcurrentGraph`
    /// this is always safe because writes use DashMap interior mutability.
    #[inline]
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
