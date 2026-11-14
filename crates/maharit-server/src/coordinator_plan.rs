//! シャーディングコーディネーターの読み取りクエリの扱い。
//!
//! コーディネーターは同じクエリを全シャードに送り、返ってきた行をここで 1 つの
//! 結果にまとめる。シャードごとの部分結果から全体の結果を作れるものだけを受け付け、
//! 作れないものはエラーにする（誤った結果を返さないため）。
//!
//! - 書き込みは受け付けない（どのシャードに書くかを決められないため）
//! - 集計 `count` / `sum` / `min` / `max` / `collect` はグループごとに合算する
//! - `ORDER BY` / `SKIP` / `LIMIT` / `DISTINCT` は全体に対して適用する
//! - `avg` などシャードの結果から合算できない集計、`WITH` での集計や件数制限、
//!   `CALL { … }` サブクエリはエラーにする

use std::cmp::Ordering;
use std::collections::HashMap;

use maharit_query::ast::{
    AggregateFunction, Expression, Literal, MatchStatement, NullsOrder, OrderDirection,
    ReturnClause, ReturnItem, Statement, UnionType,
};
use maharit_query::{Executor, Lexer, Parser, TokenKind, is_read_only};
use serde_json::Value as Json;

/// 1 行 = 列名 → 値。
pub type WireRow = HashMap<String, Json>;

/// コーディネーターでのクエリの実行方法。
#[derive(Debug, Clone, PartialEq)]
pub enum CoordPlan {
    /// 全シャードに `shard_query` を送り、`merge` に従ってまとめる。
    FanOut { shard_query: String, merge: Merge },
    /// グラフを読まないクエリ（`RETURN 1` など）や EXPLAIN は 1 シャードだけで実行する。
    SingleShard,
}

/// シャードの結果のまとめ方。
#[derive(Debug, Clone, PartialEq)]
pub enum Merge {
    /// 行をそのまま連結する（`UNION ALL` など）。
    Concat,
    /// 連結して同じ行を 1 行にする（`SHOW CONSTRAINTS` などの各シャード共通の情報、`UNION`）。
    Distinct,
    /// 最終の RETURN に従ってまとめる。
    Return(ReturnMerge),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnMerge {
    /// RETURN の各列とその扱い。
    pub columns: Vec<(String, ColumnKind)>,
    pub distinct: bool,
    /// (列名, 方向, NULL の位置)
    pub order_by: Vec<(String, OrderDirection, NullsOrder)>,
    pub skip: Option<usize>,
    pub limit: Option<usize>,
}

/// RETURN の列の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// グループのキー（集計でない列）。
    Key,
    Count,
    Sum,
    Min,
    Max,
    Collect,
}

impl ReturnMerge {
    fn has_aggregate(&self) -> bool {
        self.columns.iter().any(|(_, k)| *k != ColumnKind::Key)
    }
}

/// クエリを解析して実行方法を決める。パースできないクエリはそのまま全シャードに
/// 送り（シャードがパースエラーを返す）、受け付けないクエリは `Err` を返す。
pub fn plan(query: &str) -> Result<CoordPlan, String> {
    let stmt = match Parser::new(query).and_then(|mut p| p.parse()) {
        Ok(stmt) => stmt,
        Err(_) => {
            return Ok(CoordPlan::FanOut {
                shard_query: query.to_string(),
                merge: Merge::Concat,
            });
        }
    };
    if !is_read_only(&stmt) {
        return Err("the coordinator runs read-only queries only; \
             write to the shard that should hold the data by connecting to it directly"
            .to_string());
    }
    let fan_out = |merge| {
        Ok(CoordPlan::FanOut {
            shard_query: query.to_string(),
            merge,
        })
    };
    match &stmt {
        Statement::Match(m) => {
            if m.segments.iter().all(|s| s.match_clauses.is_empty()) && m.call_clause.is_none() {
                // MATCH の無い `WITH 1 AS x RETURN x` などは全シャードで同じ結果になる
                return Ok(CoordPlan::SingleShard);
            }
            let merge = return_merge(m)?;
            let shard_query = shard_query(query, &merge)?;
            Ok(CoordPlan::FanOut {
                shard_query,
                merge: Merge::Return(merge),
            })
        }
        Statement::Return(_) | Statement::Explain(_) => Ok(CoordPlan::SingleShard),
        Statement::Union(u) => {
            for q in &u.queries {
                let rc = &q.return_clause;
                if rc.items.iter().any(is_aggregate)
                    || rc.skip.is_some()
                    || rc.limit.is_some()
                    || q.call_clause.is_some()
                {
                    return Err(unsupported(
                        "aggregation, SKIP or LIMIT inside UNION branches",
                    ));
                }
                check_with_clauses(q)?;
            }
            match u.union_type {
                UnionType::Union => fan_out(Merge::Distinct),
                UnionType::UnionAll => fan_out(Merge::Concat),
            }
        }
        Statement::Profile(_) => Err(unsupported("PROFILE")),
        // 読み取りだけの句の列（決まった形で読めなかったもの）。合算の仕方を決められない
        Statement::Pipeline(_) => Err(unsupported("this combination of clauses")),
        // SHOW … / プロシージャ: 各シャードの結果を連結し、重複を除く
        _ => fan_out(Merge::Distinct),
    }
}

fn unsupported(what: &str) -> String {
    format!("{what} is not supported by the coordinator; run the query on a shard directly")
}

fn is_aggregate(item: &ReturnItem) -> bool {
    match item {
        ReturnItem::Aggregate(_) => true,
        ReturnItem::Alias(inner, _) => is_aggregate(inner),
        _ => false,
    }
}

/// WITH での集計・DISTINCT・SKIP・LIMIT はシャードごとに効いてしまうため受け付けない。
fn check_with_clauses(m: &MatchStatement) -> Result<(), String> {
    for seg in &m.segments {
        if let Some(w) = &seg.with_clause
            && (w.distinct
                || w.skip.is_some()
                || w.limit.is_some()
                || w.items.iter().any(|i| is_aggregate(&i.expression)))
        {
            return Err(unsupported("aggregation, DISTINCT, SKIP or LIMIT in WITH"));
        }
    }
    Ok(())
}

fn column_kind(item: &ReturnItem) -> Result<ColumnKind, String> {
    let agg = match item {
        ReturnItem::Alias(inner, _) => return column_kind(inner),
        ReturnItem::Aggregate(agg) => agg,
        ReturnItem::All => return Err(unsupported("RETURN *")),
        _ => return Ok(ColumnKind::Key),
    };
    Ok(match agg {
        AggregateFunction::Count(_) => ColumnKind::Count,
        AggregateFunction::Sum(_) => ColumnKind::Sum,
        AggregateFunction::Min(_) => ColumnKind::Min,
        AggregateFunction::Max(_) => ColumnKind::Max,
        AggregateFunction::Collect(_) => ColumnKind::Collect,
        AggregateFunction::Avg(_)
        | AggregateFunction::AvgDistinct(_)
        | AggregateFunction::CountDistinct(_)
        | AggregateFunction::SumDistinct(_)
        | AggregateFunction::CollectDistinct(_)
        | AggregateFunction::PercentileCont(..)
        | AggregateFunction::PercentileDisc(..)
        | AggregateFunction::StDev(_)
        | AggregateFunction::StDevP(_) => {
            return Err(unsupported(&format!(
                "the aggregate {}",
                Executor::column_name(item)
            )));
        }
    })
}

fn literal_count(expr: &Option<Expression>, clause: &str) -> Result<Option<usize>, String> {
    match expr {
        None => Ok(None),
        Some(Expression::Literal(Literal::Int(n))) if *n >= 0 => Ok(Some(*n as usize)),
        Some(_) => Err(unsupported(&format!(
            "{clause} other than an integer literal (e.g. a parameter)"
        ))),
    }
}

fn return_merge(m: &MatchStatement) -> Result<ReturnMerge, String> {
    if m.call_clause.is_some() {
        return Err(unsupported("CALL { … } subqueries"));
    }
    check_with_clauses(m)?;
    let rc: &ReturnClause = &m.return_clause;
    let columns = rc
        .items
        .iter()
        .map(|item| Ok((Executor::column_name(item), column_kind(item)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let order_by = match &rc.order_by {
        None => Vec::new(),
        Some(ob) => ob
            .items
            .iter()
            .map(|item| {
                let name = Executor::column_name(&item.expression);
                if columns.iter().any(|(c, _)| *c == name) {
                    Ok((name, item.direction, item.nulls_order))
                } else {
                    Err(format!(
                        "ORDER BY {name}: on the coordinator, ORDER BY can only refer to \
                         returned columns (add it to RETURN or use its alias)"
                    ))
                }
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    Ok(ReturnMerge {
        columns,
        distinct: rc.distinct,
        order_by,
        skip: literal_count(&rc.skip, "SKIP")?,
        limit: literal_count(&rc.limit, "LIMIT")?,
    })
}

/// シャードに送るクエリ。末尾の `SKIP` / `LIMIT` はコーディネーターで全体に適用するので
/// 外す。集計が無ければ、各シャードは上位 `skip + limit` 行を返せば足りる。
fn shard_query(query: &str, merge: &ReturnMerge) -> Result<String, String> {
    if merge.skip.is_none() && merge.limit.is_none() {
        return Ok(query.to_string());
    }
    let tokens = Lexer::new(query).tokenize().map_err(|e| e.to_string())?;
    let mut tail: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Eof))
        .collect();
    // 末尾は `[SKIP n] [LIMIT n]`（RETURN 句の最後）。
    let mut cut = query.len();
    for _ in 0..2 {
        let n = tail.len();
        if n >= 2 && matches!(tail[n - 2].kind, TokenKind::Skip | TokenKind::Limit) {
            cut = tail[n - 2].span.start;
            tail.truncate(n - 2);
        }
    }
    let mut q = query[..cut].trim_end().to_string();
    if !merge.has_aggregate()
        && let Some(limit) = merge.limit
    {
        q.push_str(&format!(" LIMIT {}", merge.skip.unwrap_or(0) + limit));
    }
    Ok(q)
}

/// シャードの結果を 1 つにまとめる。
pub fn merge_rows(merge: &Merge, batches: Vec<Vec<WireRow>>) -> Vec<WireRow> {
    let rows = batches.into_iter().flatten();
    match merge {
        Merge::Concat => rows.collect(),
        Merge::Distinct => dedup(rows.collect()),
        Merge::Return(rm) => {
            let mut rows: Vec<WireRow> = if rm.has_aggregate() {
                aggregate(rm, rows)
            } else {
                rows.collect()
            };
            if rm.distinct {
                rows = dedup(rows);
            }
            if !rm.order_by.is_empty() {
                rows.sort_by(|a, b| {
                    for (col, dir, nulls) in &rm.order_by {
                        let ord = compare_for_sort(
                            a.get(col).unwrap_or(&Json::Null),
                            b.get(col).unwrap_or(&Json::Null),
                            *dir,
                            *nulls,
                        );
                        if ord != Ordering::Equal {
                            return ord;
                        }
                    }
                    Ordering::Equal
                });
            }
            let skip = rm.skip.unwrap_or(0);
            let limit = rm.limit.unwrap_or(usize::MAX);
            rows.into_iter().skip(skip).take(limit).collect()
        }
    }
}

fn row_key(row: &WireRow, cols: &[&String]) -> String {
    let values: Vec<&Json> = cols
        .iter()
        .map(|c| row.get(*c).unwrap_or(&Json::Null))
        .collect();
    serde_json::to_string(&values).unwrap_or_default()
}

fn dedup(rows: Vec<WireRow>) -> Vec<WireRow> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|row| {
            let mut cols: Vec<&String> = row.keys().collect();
            cols.sort();
            let key = format!("{:?}|{}", cols, row_key(row, &cols));
            seen.insert(key)
        })
        .collect()
}

/// グループのキー列ごとに、集計列の部分結果を合算する。
fn aggregate(rm: &ReturnMerge, rows: impl Iterator<Item = WireRow>) -> Vec<WireRow> {
    let keys: Vec<&String> = rm
        .columns
        .iter()
        .filter(|(_, k)| *k == ColumnKind::Key)
        .map(|(c, _)| c)
        .collect();
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, WireRow> = HashMap::new();
    for row in rows {
        let key = row_key(&row, &keys);
        match groups.get_mut(&key) {
            None => {
                order.push(key.clone());
                groups.insert(key, row);
            }
            Some(acc) => {
                for (col, kind) in &rm.columns {
                    let next = row.get(col).cloned().unwrap_or(Json::Null);
                    let cur = acc.remove(col).unwrap_or(Json::Null);
                    acc.insert(col.clone(), combine(*kind, cur, next));
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .collect()
}

fn combine(kind: ColumnKind, a: Json, b: Json) -> Json {
    match kind {
        ColumnKind::Key => a,
        ColumnKind::Count | ColumnKind::Sum => add(a, b),
        ColumnKind::Min | ColumnKind::Max => {
            if a.is_null() {
                return b;
            }
            if b.is_null() {
                return a;
            }
            let ord = compare_values(&a, &b);
            let take_b = if kind == ColumnKind::Min {
                ord == Ordering::Greater
            } else {
                ord == Ordering::Less
            };
            if take_b { b } else { a }
        }
        ColumnKind::Collect => match (a, b) {
            (Json::Array(mut x), Json::Array(y)) => {
                x.extend(y);
                Json::Array(x)
            }
            (Json::Array(x), _) | (_, Json::Array(x)) => Json::Array(x),
            _ => Json::Array(Vec::new()),
        },
    }
}

fn add(a: Json, b: Json) -> Json {
    match (&a, &b) {
        (Json::Null, _) => b,
        (_, Json::Null) => a,
        _ => match (a.as_i64(), b.as_i64()) {
            (Some(x), Some(y)) if a.is_i64() && b.is_i64() => Json::from(x.saturating_add(y)),
            _ => match (a.as_f64(), b.as_f64()) {
                (Some(x), Some(y)) => Json::from(x + y),
                _ => Json::Null,
            },
        },
    }
}

/// NULL 以外の値の比較（数値は数値として、文字列は辞書順）。
fn compare_values(a: &Json, b: &Json) -> Ordering {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => match (x.as_i64(), y.as_i64()) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => x
                .as_f64()
                .partial_cmp(&y.as_f64())
                .unwrap_or(Ordering::Equal),
        },
        (Json::String(x), Json::String(y)) => x.cmp(y),
        (Json::Bool(x), Json::Bool(y)) => x.cmp(y),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

/// executor の ORDER BY と同じ規則: NULL は既定で ASC なら末尾、DESC なら先頭。
fn compare_for_sort(a: &Json, b: &Json, dir: OrderDirection, nulls: NullsOrder) -> Ordering {
    let nulls_last = match nulls {
        NullsOrder::First => false,
        NullsOrder::Last => true,
        NullsOrder::Default => matches!(dir, OrderDirection::Asc),
    };
    match (a.is_null(), b.is_null()) {
        (true, true) => return Ordering::Equal,
        (true, false) => {
            return if nulls_last {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        (false, true) => {
            return if nulls_last {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        _ => {}
    }
    let ord = compare_values(a, b);
    match dir {
        OrderDirection::Asc => ord,
        OrderDirection::Desc => ord.reverse(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(pairs: &[(&str, Json)]) -> WireRow {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn fan_out(query: &str) -> (String, Merge) {
        match plan(query).unwrap() {
            CoordPlan::FanOut { shard_query, merge } => (shard_query, merge),
            other => panic!("{query}: {other:?}"),
        }
    }

    #[test]
    fn writes_are_rejected() {
        for q in [
            "CREATE (n:P)",
            "MATCH (n) SET n.x = 1",
            "MERGE (n:P {id: 1})",
            "MATCH (n) DETACH DELETE n",
            "CREATE INDEX ON :P(id)",
        ] {
            let err = plan(q).unwrap_err();
            assert!(err.contains("read-only"), "{q}: {err}");
        }
    }

    #[test]
    fn unmergeable_queries_are_rejected() {
        for q in [
            "MATCH (n) RETURN avg(n.x)",
            "MATCH (n) RETURN count(DISTINCT n.x)",
            "MATCH (n) WITH n.k AS k, count(n) AS c RETURN k, c",
            "MATCH (n) WITH n LIMIT 3 RETURN n",
            "MATCH (n) RETURN n.name ORDER BY n.age",
            "MATCH (n) CALL { MATCH (m) RETURN count(m) AS c } RETURN n, c",
            "MATCH (n) RETURN n.x AS x UNION MATCH (m) RETURN count(m) AS x",
        ] {
            assert!(plan(q).is_err(), "{q} should be rejected");
        }
    }

    #[test]
    fn non_graph_queries_run_on_one_shard() {
        assert_eq!(plan("RETURN 1 AS x").unwrap(), CoordPlan::SingleShard);
        assert_eq!(
            plan("EXPLAIN MATCH (n) RETURN n").unwrap(),
            CoordPlan::SingleShard
        );
    }

    #[test]
    fn shard_query_drops_skip_and_widens_limit() {
        let (q, _) = fan_out("MATCH (n:P) RETURN n.name ORDER BY n.name SKIP 2 LIMIT 3");
        assert_eq!(q, "MATCH (n:P) RETURN n.name ORDER BY n.name LIMIT 5");
        let (q, _) = fan_out("MATCH (n:P) RETURN n.name SKIP 2");
        assert_eq!(q, "MATCH (n:P) RETURN n.name");
        // 集計があるときは各シャードの全グループが要る
        let (q, _) = fan_out("MATCH (n:P) RETURN n.k AS k, count(n) AS c ORDER BY c DESC LIMIT 1");
        assert_eq!(
            q,
            "MATCH (n:P) RETURN n.k AS k, count(n) AS c ORDER BY c DESC"
        );
        let (q, _) = fan_out("MATCH (n:P) RETURN n");
        assert_eq!(q, "MATCH (n:P) RETURN n");
    }

    #[test]
    fn aggregates_are_combined_per_group() {
        let (_, merge) = fan_out(
            "MATCH (n:P) RETURN n.k AS k, count(n) AS c, sum(n.v) AS s, min(n.v) AS lo, \
             max(n.v) AS hi, collect(n.v) AS vs ORDER BY k",
        );
        let shard1 = vec![
            row(&[
                ("k", json!("a")),
                ("c", json!(2)),
                ("s", json!(3)),
                ("lo", json!(1)),
                ("hi", json!(2)),
                ("vs", json!([1, 2])),
            ]),
            row(&[
                ("k", json!("b")),
                ("c", json!(1)),
                ("s", json!(1.5)),
                ("lo", json!(1.5)),
                ("hi", json!(1.5)),
                ("vs", json!([1.5])),
            ]),
        ];
        let shard2 = vec![row(&[
            ("k", json!("a")),
            ("c", json!(1)),
            ("s", json!(10)),
            ("lo", json!(10)),
            ("hi", json!(10)),
            ("vs", json!([10])),
        ])];
        let out = merge_rows(&merge, vec![shard1, shard2]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["k"], json!("a"));
        assert_eq!(out[0]["c"], json!(3));
        assert_eq!(out[0]["s"], json!(13));
        assert_eq!(out[0]["lo"], json!(1));
        assert_eq!(out[0]["hi"], json!(10));
        assert_eq!(out[0]["vs"], json!([1, 2, 10]));
        assert_eq!(out[1]["s"], json!(1.5));
    }

    #[test]
    fn global_count_without_keys() {
        let (_, merge) = fan_out("MATCH (n) RETURN count(n)");
        let out = merge_rows(
            &merge,
            vec![
                vec![row(&[("count(n)", json!(1))])],
                vec![row(&[("count(n)", json!(1))])],
            ],
        );
        assert_eq!(out, vec![row(&[("count(n)", json!(2))])]);
    }

    #[test]
    fn rows_are_not_deduplicated_and_order_limit_apply_globally() {
        let (_, merge) =
            fan_out("MATCH (n:P) RETURN n.name AS name ORDER BY name DESC SKIP 1 LIMIT 2");
        let names = |v: &[&str]| {
            v.iter()
                .map(|n| row(&[("name", json!(n))]))
                .collect::<Vec<_>>()
        };
        let out = merge_rows(
            &merge,
            vec![names(&["a", "c", "e"]), names(&["b", "c", "d"])],
        );
        assert_eq!(out, names(&["d", "c"]));

        let (_, merge) = fan_out("MATCH (n:P) RETURN n.name AS name");
        let out = merge_rows(&merge, vec![names(&["a"]), names(&["a"])]);
        assert_eq!(out.len(), 2, "rows from different nodes are kept");

        let (_, merge) = fan_out("MATCH (n:P) RETURN DISTINCT n.name AS name");
        let out = merge_rows(&merge, vec![names(&["a"]), names(&["a"])]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn nulls_sort_like_the_executor() {
        let (_, merge) = fan_out("MATCH (n) RETURN n.x AS x ORDER BY x");
        let xs = |v: &[Json]| {
            v.iter()
                .map(|x| row(&[("x", x.clone())]))
                .collect::<Vec<_>>()
        };
        let out = merge_rows(&merge, vec![xs(&[json!(2), Json::Null]), xs(&[json!(1.5)])]);
        assert_eq!(out, xs(&[json!(1.5), json!(2), Json::Null]));
    }
}
