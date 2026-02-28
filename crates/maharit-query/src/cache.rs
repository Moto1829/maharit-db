//! Query cache for storing parsed ASTs and execution plans.
//!
//! Caches parsed [`Statement`]s keyed by query string to avoid re-parsing
//! identical queries. Uses an LRU eviction policy with configurable capacity.

use std::collections::HashMap;

use crate::ast::Statement;
use crate::parser::{ParseError, Parser};
use crate::planner::{GraphStats, QueryPlan, build_plan_with_stats};

/// A cached query entry with usage tracking.
#[derive(Debug, Clone)]
struct CacheEntry {
    /// The parsed statement.
    statement: Statement,
    /// Number of times this entry has been accessed.
    hit_count: u64,
    /// Logical timestamp of last access (for LRU eviction).
    last_accessed: u64,
}

/// LRU cache for parsed query statements.
///
/// Stores parsed ASTs keyed by their original query string. When the cache
/// reaches its capacity, the least recently used entry is evicted.
#[derive(Debug)]
pub struct QueryCache {
    entries: HashMap<String, CacheEntry>,
    capacity: usize,
    /// Monotonically increasing counter for LRU ordering.
    clock: u64,
    /// Total number of cache hits.
    total_hits: u64,
    /// Total number of cache misses.
    total_misses: u64,
}

/// Statistics about cache usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of entries currently in the cache.
    pub size: usize,
    /// Maximum capacity of the cache.
    pub capacity: usize,
    /// Total number of cache hits.
    pub hits: u64,
    /// Total number of cache misses.
    pub misses: u64,
}

impl CacheStats {
    /// Calculate the cache hit rate as a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

impl QueryCache {
    /// Create a new query cache with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            capacity,
            clock: 0,
            total_hits: 0,
            total_misses: 0,
        }
    }

    /// Get a cached statement for the given query, or parse and cache it.
    ///
    /// Returns a clone of the cached statement on hit, or parses the query,
    /// caches the result, and returns it on miss.
    pub fn get_or_parse(&mut self, query: &str) -> Result<Statement, ParseError> {
        let normalized = normalize_query(query);

        if let Some(entry) = self.entries.get_mut(&normalized) {
            self.clock += 1;
            entry.last_accessed = self.clock;
            entry.hit_count += 1;
            self.total_hits += 1;
            return Ok(entry.statement.clone());
        }

        // Cache miss - parse the query
        self.total_misses += 1;
        let mut parser = Parser::new(query)?;
        let statement = parser.parse()?;

        // Only cache if capacity > 0
        if self.capacity > 0 {
            // Evict if at capacity
            if self.entries.len() >= self.capacity {
                self.evict_lru();
            }

            self.clock += 1;
            self.entries.insert(
                normalized,
                CacheEntry {
                    statement: statement.clone(),
                    hit_count: 0,
                    last_accessed: self.clock,
                },
            );
        }

        Ok(statement)
    }

    /// Check if the cache contains a parsed statement for the given query.
    pub fn contains(&self, query: &str) -> bool {
        let normalized = normalize_query(query);
        self.entries.contains_key(&normalized)
    }

    /// Invalidate (remove) a specific cached query.
    pub fn invalidate(&mut self, query: &str) {
        let normalized = normalize_query(query);
        self.entries.remove(&normalized);
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_hits = 0;
        self.total_misses = 0;
        self.clock = 0;
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            size: self.entries.len(),
            capacity: self.capacity,
            hits: self.total_hits,
            misses: self.total_misses,
        }
    }

    /// Evict the least recently used entry.
    fn evict_lru(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let lru_key = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(key, _)| key.clone());

        if let Some(key) = lru_key {
            self.entries.remove(&key);
        }
    }
}

/// A cached plan entry with the graph stats snapshot it was built with.
#[derive(Debug, Clone)]
struct PlanCacheEntry {
    plan: QueryPlan,
    /// Node count when the plan was built (for staleness detection).
    node_count: u64,
    /// Edge count when the plan was built (for staleness detection).
    edge_count: u64,
    last_accessed: u64,
}

/// Cache for execution plans built from parsed statements.
///
/// Plans are keyed by normalized query string and invalidated when
/// graph statistics change significantly (node/edge count differs).
#[derive(Debug)]
pub struct PlanCache {
    entries: HashMap<String, PlanCacheEntry>,
    capacity: usize,
    clock: u64,
    total_hits: u64,
    total_misses: u64,
}

impl PlanCache {
    /// Create a new plan cache with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            capacity,
            clock: 0,
            total_hits: 0,
            total_misses: 0,
        }
    }

    /// Get a cached plan or build and cache one from the statement and stats.
    pub fn get_or_build(&mut self, query: &str, stmt: &Statement, stats: &GraphStats) -> QueryPlan {
        let normalized = normalize_query(query);

        if let Some(entry) = self.entries.get_mut(&normalized) {
            // Check if the plan is still valid (stats haven't changed much)
            if entry.node_count == stats.node_count && entry.edge_count == stats.edge_count {
                self.clock += 1;
                entry.last_accessed = self.clock;
                self.total_hits += 1;
                return entry.plan.clone();
            }
            // Stats changed - invalidate this entry
            self.entries.remove(&normalized);
        }

        // Build a new plan
        self.total_misses += 1;
        let plan = build_plan_with_stats(stmt, stats);

        if self.capacity > 0 {
            if self.entries.len() >= self.capacity {
                self.evict_lru();
            }

            self.clock += 1;
            self.entries.insert(
                normalized,
                PlanCacheEntry {
                    plan: plan.clone(),
                    node_count: stats.node_count,
                    edge_count: stats.edge_count,
                    last_accessed: self.clock,
                },
            );
        }

        plan
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            size: self.entries.len(),
            capacity: self.capacity,
            hits: self.total_hits,
            misses: self.total_misses,
        }
    }

    /// Clear all cached plans.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_hits = 0;
        self.total_misses = 0;
        self.clock = 0;
    }

    fn evict_lru(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let lru_key = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(key, _)| key.clone());

        if let Some(key) = lru_key {
            self.entries.remove(&key);
        }
    }
}

/// Normalize a query string for cache key comparison.
///
/// Trims whitespace and collapses multiple spaces into single spaces.
fn normalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// パラメータ化クエリの AST キャッシュ。
///
/// クエリ文字列をキーとして、解析済みの [`Statement`] を保持する。
/// パラメータ値が異なっていても同じクエリ文字列であれば AST を再利用できる。
///
/// LRU eviction policy を使用して最大サイズを超えた場合に古いエントリを削除する。
///
/// # Examples
///
/// ```rust
/// use maharit_query::cache::AstCache;
///
/// let mut cache = AstCache::new(100);
/// let stmt = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();
///
/// // 2回目はキャッシュヒット
/// let stmt2 = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();
/// assert_eq!(cache.stats().hits, 1);
/// ```
#[derive(Debug)]
pub struct AstCache {
    cache: HashMap<String, AstCacheEntry>,
    max_size: usize,
    clock: u64,
    total_hits: u64,
    total_misses: u64,
    parse_count: u64,
}

#[derive(Debug, Clone)]
struct AstCacheEntry {
    statement: Statement,
    last_accessed: u64,
}

impl AstCache {
    /// 指定した最大サイズで新しい AstCache を作成する。
    ///
    /// `max_size` が 0 の場合、キャッシュは無効（エントリを保持しない）。
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: HashMap::with_capacity(max_size.min(256)),
            max_size,
            clock: 0,
            total_hits: 0,
            total_misses: 0,
            parse_count: 0,
        }
    }

    /// クエリ文字列に対応する Statement を返す。キャッシュに存在する場合は再利用し、
    /// 存在しない場合はパースしてキャッシュに追加する。
    ///
    /// # Errors
    ///
    /// クエリのパースに失敗した場合は [`ParseError`] を返す。
    /// パースエラーはキャッシュされない。
    pub fn get_or_parse(&mut self, query: &str) -> Result<Statement, ParseError> {
        let key = normalize_query(query);

        if let Some(entry) = self.cache.get_mut(&key) {
            self.clock += 1;
            entry.last_accessed = self.clock;
            self.total_hits += 1;
            return Ok(entry.statement.clone());
        }

        // キャッシュミス - パースを実行
        self.total_misses += 1;
        self.parse_count += 1;
        let mut parser = Parser::new(query)?;
        let statement = parser.parse()?;

        if self.max_size > 0 {
            if self.cache.len() >= self.max_size {
                self.evict_lru();
            }
            self.clock += 1;
            self.cache.insert(
                key,
                AstCacheEntry {
                    statement: statement.clone(),
                    last_accessed: self.clock,
                },
            );
        }

        Ok(statement)
    }

    /// キャッシュに指定クエリのエントリが存在するかどうかを確認する。
    pub fn contains(&self, query: &str) -> bool {
        let key = normalize_query(query);
        self.cache.contains_key(&key)
    }

    /// 指定クエリのキャッシュエントリを無効化（削除）する。
    pub fn invalidate(&mut self, query: &str) {
        let key = normalize_query(query);
        self.cache.remove(&key);
    }

    /// キャッシュ内の全エントリをクリアする。統計もリセットされる。
    pub fn clear(&mut self) {
        self.cache.clear();
        self.total_hits = 0;
        self.total_misses = 0;
        self.parse_count = 0;
        self.clock = 0;
    }

    /// キャッシュの統計情報を返す。
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            size: self.cache.len(),
            capacity: self.max_size,
            hits: self.total_hits,
            misses: self.total_misses,
        }
    }

    /// 実際にパースが実行された回数を返す。
    ///
    /// キャッシュヒット時はパースが実行されないため、この値は
    /// `stats().misses` と等しくなる（clear() を呼んだ後も追跡は継続される）。
    pub fn parse_count(&self) -> u64 {
        self.parse_count
    }

    /// Least Recently Used なエントリを削除する。
    fn evict_lru(&mut self) {
        if self.cache.is_empty() {
            return;
        }

        let lru_key = self
            .cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(key, _)| key.clone());

        if let Some(key) = lru_key {
            self.cache.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit() {
        let mut cache = QueryCache::new(10);

        let stmt1 = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();
        let stmt2 = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();

        assert_eq!(stmt1, stmt2);

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.size, 1);
    }

    #[test]
    fn test_cache_miss() {
        let mut cache = QueryCache::new(10);

        cache
            .get_or_parse("CREATE (n:Person {name: 'Alice'})")
            .unwrap();
        cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.size, 2);
    }

    #[test]
    fn test_whitespace_normalization() {
        let mut cache = QueryCache::new(10);

        let stmt1 = cache.get_or_parse("MATCH  (n:Person)  RETURN  n").unwrap();
        let stmt2 = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();

        assert_eq!(stmt1, stmt2);

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_lru_eviction() {
        let mut cache = QueryCache::new(2);

        cache.get_or_parse("CREATE (a:A)").unwrap();
        cache.get_or_parse("CREATE (b:B)").unwrap();

        assert_eq!(cache.stats().size, 2);

        // Access first entry to make it more recent
        cache.get_or_parse("CREATE (a:A)").unwrap();

        // Add third entry, should evict "CREATE (b:B)" (least recently used)
        cache.get_or_parse("CREATE (c:C)").unwrap();

        assert_eq!(cache.stats().size, 2);
        assert!(cache.contains("CREATE (a:A)"));
        assert!(!cache.contains("CREATE (b:B)"));
        assert!(cache.contains("CREATE (c:C)"));
    }

    #[test]
    fn test_invalidate() {
        let mut cache = QueryCache::new(10);

        cache.get_or_parse("MATCH (n) RETURN n").unwrap();
        assert!(cache.contains("MATCH (n) RETURN n"));

        cache.invalidate("MATCH (n) RETURN n");
        assert!(!cache.contains("MATCH (n) RETURN n"));
    }

    #[test]
    fn test_clear() {
        let mut cache = QueryCache::new(10);

        cache.get_or_parse("CREATE (a:A)").unwrap();
        cache.get_or_parse("CREATE (b:B)").unwrap();
        cache.get_or_parse("CREATE (a:A)").unwrap(); // hit

        assert_eq!(cache.stats().size, 2);
        assert_eq!(cache.stats().hits, 1);

        cache.clear();

        assert_eq!(cache.stats().size, 0);
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn test_hit_rate() {
        let stats = CacheStats {
            size: 5,
            capacity: 10,
            hits: 75,
            misses: 25,
        };
        assert!((stats.hit_rate() - 75.0).abs() < f64::EPSILON);

        let empty_stats = CacheStats {
            size: 0,
            capacity: 10,
            hits: 0,
            misses: 0,
        };
        assert!((empty_stats.hit_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_error_not_cached() {
        let mut cache = QueryCache::new(10);

        let result = cache.get_or_parse("INVALID QUERY ???");
        assert!(result.is_err());

        // Errors should not be cached
        assert_eq!(cache.stats().size, 0);
    }

    #[test]
    fn test_capacity_zero() {
        let mut cache = QueryCache::new(0);

        // Should still parse but immediately evict
        let stmt = cache.get_or_parse("MATCH (n) RETURN n").unwrap();
        assert_eq!(cache.stats().size, 0);

        // But the parse result should still be valid
        assert!(matches!(stmt, Statement::Match(_)));
    }

    #[test]
    fn test_contains() {
        let mut cache = QueryCache::new(10);

        assert!(!cache.contains("MATCH (n) RETURN n"));
        cache.get_or_parse("MATCH (n) RETURN n").unwrap();
        assert!(cache.contains("MATCH (n) RETURN n"));
    }

    // ===== PlanCache tests =====

    #[test]
    fn test_plan_cache_hit() {
        let mut cache = PlanCache::new(10);
        let stats = GraphStats::simple(100, 50);

        let stmt = Parser::new("MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();

        let plan1 = cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats);
        let plan2 = cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats);

        assert_eq!(plan1.nodes.len(), plan2.nodes.len());

        let cache_stats = cache.stats();
        assert_eq!(cache_stats.hits, 1);
        assert_eq!(cache_stats.misses, 1);
    }

    #[test]
    fn test_plan_cache_invalidated_by_stats_change() {
        let mut cache = PlanCache::new(10);
        let stats1 = GraphStats::simple(100, 50);
        let stats2 = GraphStats::simple(200, 100); // Different counts

        let stmt = Parser::new("MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();

        cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 0);

        // Same query but different stats - should miss (plan invalidated)
        cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats2);
        assert_eq!(cache.stats().misses, 2);
        assert_eq!(cache.stats().hits, 0);
    }

    #[test]
    fn test_plan_cache_same_stats_hits() {
        let mut cache = PlanCache::new(10);
        let stats = GraphStats::simple(100, 50);

        let stmt = Parser::new("MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();

        cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats);
        cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats);
        cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats);

        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 2);
    }

    #[test]
    fn test_plan_cache_clear() {
        let mut cache = PlanCache::new(10);
        let stats = GraphStats::simple(100, 50);

        let stmt = Parser::new("MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();

        cache.get_or_build("MATCH (n:Person) RETURN n", &stmt, &stats);
        assert_eq!(cache.stats().size, 1);

        cache.clear();
        assert_eq!(cache.stats().size, 0);
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn test_plan_cache_lru_eviction() {
        let mut cache = PlanCache::new(2);
        let stats = GraphStats::simple(100, 50);

        let stmt_a = Parser::new("CREATE (a:A)").unwrap().parse().unwrap();
        let stmt_b = Parser::new("CREATE (b:B)").unwrap().parse().unwrap();
        let stmt_c = Parser::new("CREATE (c:C)").unwrap().parse().unwrap();

        cache.get_or_build("CREATE (a:A)", &stmt_a, &stats);
        cache.get_or_build("CREATE (b:B)", &stmt_b, &stats);

        // Access a:A to make it more recent
        cache.get_or_build("CREATE (a:A)", &stmt_a, &stats);

        // Add c:C - should evict b:B
        cache.get_or_build("CREATE (c:C)", &stmt_c, &stats);

        assert_eq!(cache.stats().size, 2);
    }

    #[test]
    fn test_various_query_types() {
        let mut cache = QueryCache::new(20);

        // Test various query types are cached correctly
        let queries = vec![
            "CREATE (n:Person {name: 'Alice'})",
            "MATCH (n:Person) RETURN n.name",
            "MATCH (n:Person) WHERE n.age > 30 RETURN n",
            "MATCH (a)-[r:KNOWS]->(b) RETURN a, r, b",
            "MATCH (n) DELETE n",
            "MATCH (n:Person) SET n.age = 31",
        ];

        for query in &queries {
            cache.get_or_parse(query).unwrap();
        }
        assert_eq!(cache.stats().size, queries.len());

        // All should hit on second access
        for query in &queries {
            cache.get_or_parse(query).unwrap();
        }
        assert_eq!(cache.stats().hits, queries.len() as u64);
    }

    // ===== AstCache tests =====

    #[test]
    fn test_ast_cache_hit() {
        let mut cache = AstCache::new(10);

        let stmt1 = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();
        let stmt2 = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();

        // 同じ AST が返ること
        assert_eq!(stmt1, stmt2);

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.size, 1);
    }

    #[test]
    fn test_ast_cache_no_reparse_on_hit() {
        let mut cache = AstCache::new(10);

        // 最初の呼び出しはパース実行
        cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(cache.parse_count(), 1);

        // 2回目以降はパースしない
        cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();
        cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();
        assert_eq!(cache.parse_count(), 1); // パース回数は変わらない
    }

    #[test]
    fn test_ast_cache_miss_on_different_queries() {
        let mut cache = AstCache::new(10);

        cache.get_or_parse("CREATE (n:Person {name: 'Alice'})").unwrap();
        cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.size, 2);
        assert_eq!(cache.parse_count(), 2);
    }

    #[test]
    fn test_ast_cache_clear_causes_miss() {
        let mut cache = AstCache::new(10);

        cache.get_or_parse("MATCH (n) RETURN n").unwrap();
        assert_eq!(cache.stats().size, 1);

        cache.clear();

        assert_eq!(cache.stats().size, 0);
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().misses, 0);
        assert_eq!(cache.parse_count(), 0);

        // クリア後は再パースが必要
        cache.get_or_parse("MATCH (n) RETURN n").unwrap();
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.parse_count(), 1);
    }

    #[test]
    fn test_ast_cache_parse_error_not_cached() {
        let mut cache = AstCache::new(10);

        let result = cache.get_or_parse("INVALID QUERY ???");
        assert!(result.is_err());

        // エラーはキャッシュされない
        assert_eq!(cache.stats().size, 0);
    }

    #[test]
    fn test_ast_cache_invalidate() {
        let mut cache = AstCache::new(10);

        cache.get_or_parse("MATCH (n) RETURN n").unwrap();
        assert!(cache.contains("MATCH (n) RETURN n"));

        cache.invalidate("MATCH (n) RETURN n");
        assert!(!cache.contains("MATCH (n) RETURN n"));
    }

    #[test]
    fn test_ast_cache_lru_eviction() {
        let mut cache = AstCache::new(2);

        cache.get_or_parse("CREATE (a:A)").unwrap();
        cache.get_or_parse("CREATE (b:B)").unwrap();
        assert_eq!(cache.stats().size, 2);

        // a:A を最近アクセスとしてマーク
        cache.get_or_parse("CREATE (a:A)").unwrap();

        // c:C を追加 -> b:B が LRU として削除される
        cache.get_or_parse("CREATE (c:C)").unwrap();

        assert_eq!(cache.stats().size, 2);
        assert!(cache.contains("CREATE (a:A)"));
        assert!(!cache.contains("CREATE (b:B)"));
        assert!(cache.contains("CREATE (c:C)"));
    }

    #[test]
    fn test_ast_cache_whitespace_normalization() {
        let mut cache = AstCache::new(10);

        let stmt1 = cache.get_or_parse("MATCH  (n:Person)  RETURN  n").unwrap();
        let stmt2 = cache.get_or_parse("MATCH (n:Person) RETURN n").unwrap();

        // 正規化後は同一キーとして扱われキャッシュヒットする
        assert_eq!(stmt1, stmt2);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_ast_cache_capacity_zero() {
        let mut cache = AstCache::new(0);

        // パースは成功するがキャッシュには保存されない
        let stmt = cache.get_or_parse("MATCH (n) RETURN n").unwrap();
        assert_eq!(cache.stats().size, 0);
        assert!(matches!(stmt, Statement::Match(_)));
    }

    // ===== execute_cached tests (integration) =====

    #[test]
    fn test_execute_cached_same_query_different_params() {
        use crate::executor::{Executor, Value};
        use maharit_core::Graph;
        use std::collections::HashMap;

        let mut graph = Graph::new();
        let mut cache = AstCache::new(10);
        let mut executor = Executor::new(&mut graph);

        // 最初のパラメータでノード作成
        let mut params1 = HashMap::new();
        params1.insert("name".to_string(), Value::String("Alice".to_string()));
        executor
            .execute_cached(
                "CREATE (n:Person {name: $name})",
                params1,
                &mut cache,
            )
            .unwrap();

        // 同じクエリ文字列・異なるパラメータでノード作成（キャッシュヒット）
        let mut params2 = HashMap::new();
        params2.insert("name".to_string(), Value::String("Bob".to_string()));
        executor
            .execute_cached(
                "CREATE (n:Person {name: $name})",
                params2,
                &mut cache,
            )
            .unwrap();

        // 2回目はキャッシュヒット（パースが1回だけ実行される）
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.parse_count(), 1);
    }

    #[test]
    fn test_execute_cached_invalid_query_returns_error() {
        use crate::executor::{Executor, ExecuteError};
        use maharit_core::Graph;
        use std::collections::HashMap;

        let mut graph = Graph::new();
        let mut cache = AstCache::new(10);
        let mut executor = Executor::new(&mut graph);

        let result = executor.execute_cached("INVALID QUERY ???", HashMap::new(), &mut cache);
        assert!(result.is_err());
        assert!(matches!(result, Err(ExecuteError::ParseError(_))));
    }
}
