use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::ast::*;
use maharit_core::Graph;

/// クエリ実行計画のノード
#[derive(Debug, Clone)]
pub struct PlanNode {
    /// オペレーター名
    pub operator: String,
    /// 推定行数
    pub estimated_rows: u64,
    /// コスト見積もり
    pub estimated_cost: u64,
    /// 詳細情報
    pub details: String,
    /// 実行統計（PROFILE時のみ）
    pub actual_rows: Option<u64>,
    /// 実行時間（マイクロ秒、PROFILE時のみ）
    pub actual_time_us: Option<u64>,
    /// 子ノード
    pub children: Vec<PlanNode>,
}

impl PlanNode {
    pub fn new(operator: &str, estimated_rows: u64, estimated_cost: u64, details: &str) -> Self {
        Self {
            operator: operator.to_string(),
            estimated_rows,
            estimated_cost,
            details: details.to_string(),
            actual_rows: None,
            actual_time_us: None,
            children: Vec::new(),
        }
    }

    pub fn with_child(mut self, child: PlanNode) -> Self {
        self.children.push(child);
        self
    }
}

/// クエリ実行計画
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub nodes: Vec<PlanNode>,
}

impl fmt::Display for QueryPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let has_profile = self.nodes.iter().any(|n| n.actual_rows.is_some());

        if has_profile {
            writeln!(
                f,
                "{:<24} {:>12} {:>8} {:>12} {:>12}",
                "Operator", "Est. Rows", "Cost", "Actual Rows", "Time (us)"
            )?;
            writeln!(f, "{}", "-".repeat(72))?;
        } else {
            writeln!(
                f,
                "{:<24} {:>12} {:>8} {}",
                "Operator", "Est. Rows", "Cost", "Details"
            )?;
            writeln!(f, "{}", "-".repeat(72))?;
        }

        for node in &self.nodes {
            self.fmt_node(f, node, 0, has_profile)?;
        }

        Ok(())
    }
}

impl QueryPlan {
    fn fmt_node(
        &self,
        f: &mut fmt::Formatter<'_>,
        node: &PlanNode,
        depth: usize,
        has_profile: bool,
    ) -> fmt::Result {
        let indent = "  ".repeat(depth);
        let name = format!("{}{}", indent, node.operator);

        if has_profile {
            writeln!(
                f,
                "{:<24} {:>12} {:>8} {:>12} {:>12}",
                name,
                node.estimated_rows,
                node.estimated_cost,
                node.actual_rows
                    .map(|r| r.to_string())
                    .unwrap_or("-".to_string()),
                node.actual_time_us
                    .map(|t| t.to_string())
                    .unwrap_or("-".to_string()),
            )?;
        } else {
            writeln!(
                f,
                "{:<24} {:>12} {:>8} {}",
                name, node.estimated_rows, node.estimated_cost, node.details,
            )?;
        }

        for child in &node.children {
            self.fmt_node(f, child, depth + 1, has_profile)?;
        }

        Ok(())
    }
}

/// Statistics about the graph for query cost estimation.
#[derive(Debug, Clone)]
pub struct GraphStats {
    /// Total number of nodes.
    pub node_count: u64,
    /// Total number of edges.
    pub edge_count: u64,
    /// Number of nodes per label.
    pub label_counts: HashMap<String, u64>,
    /// Set of indexed (label, property) pairs for index selection.
    pub indexed_properties: HashSet<(String, String)>,
}

impl GraphStats {
    /// Collect statistics from a graph.
    pub fn from_graph(graph: &Graph) -> Self {
        let mut label_counts: HashMap<String, u64> = HashMap::new();
        for node in graph.nodes() {
            *label_counts.entry(node.label.clone()).or_insert(0) += 1;
        }

        Self {
            node_count: graph.node_count() as u64,
            edge_count: graph.edge_count() as u64,
            label_counts,
            indexed_properties: HashSet::new(),
        }
    }

    /// Collect statistics from a graph including index information.
    pub fn from_graph_with_indexes(
        graph: &Graph,
        property_index: &maharit_core::PropertyIndex,
    ) -> Self {
        let mut stats = Self::from_graph(graph);
        for idx in property_index.list_indexes() {
            stats
                .indexed_properties
                .insert((idx.label.clone(), idx.property.clone()));
        }
        stats
    }

    /// Create stats with just total counts (no label distribution).
    pub fn simple(node_count: u64, edge_count: u64) -> Self {
        Self {
            node_count,
            edge_count,
            label_counts: HashMap::new(),
            indexed_properties: HashSet::new(),
        }
    }

    /// Estimate the number of nodes with a given label.
    pub fn estimate_label_count(&self, label: &str) -> u64 {
        if let Some(&count) = self.label_counts.get(label) {
            count
        } else if self.label_counts.is_empty() {
            // No label distribution available, use 10% heuristic
            (self.node_count / 10).max(1)
        } else {
            // Label not found in stats, likely 0 but estimate 1
            1
        }
    }

    /// Check if a (label, property) pair has an index.
    pub fn has_index(&self, label: &str, property: &str) -> bool {
        self.indexed_properties
            .contains(&(label.to_string(), property.to_string()))
    }
}

/// Build a query plan from a statement using graph statistics.
pub fn build_plan_with_stats(stmt: &Statement, stats: &GraphStats) -> QueryPlan {
    let nodes = match stmt {
        Statement::Create(create) => plan_create(create),
        Statement::Match(m) => plan_match_with_stats(m, stats),
        Statement::Delete(d) => plan_delete_with_stats(d, stats),
        Statement::Union(u) => plan_union_with_stats(u, stats),
        Statement::MatchCreate(mc) => plan_match_create_with_stats(mc, stats),
        Statement::MatchSet(ms) => plan_match_set_with_stats(ms, stats),
        Statement::Merge(merge) => plan_merge_with_stats(merge, stats),
        Statement::MatchRemove(mr) => plan_match_remove_with_stats(mr, stats),
        Statement::Unwind(uw) => plan_unwind(uw),
        Statement::Foreach(_) => {
            vec![PlanNode::new("Foreach", 1, 1, "iterate list")]
        }
        Statement::MatchForeach(mf) => {
            let seg_count = mf.segments.len() as u64;
            vec![
                PlanNode::new(
                    "NodeByLabelScan",
                    seg_count.max(1),
                    seg_count.max(1) / 10 + 1,
                    "",
                ),
                PlanNode::new("Foreach", 1, 1, "iterate list"),
            ]
        }
        Statement::CreateConstraint(_) => {
            vec![PlanNode::new("CreateConstraint", 1, 1, "")]
        }
        Statement::DropConstraint(_) => {
            vec![PlanNode::new("DropConstraint", 1, 1, "")]
        }
        Statement::ShowConstraints => {
            vec![PlanNode::new("ShowConstraints", 1, 1, "")]
        }
        Statement::CreateFulltextIndex(_) => {
            vec![PlanNode::new("CreateFulltextIndex", 1, 1, "")]
        }
        Statement::DropFulltextIndex(_) => {
            vec![PlanNode::new("DropFulltextIndex", 1, 1, "")]
        }
        Statement::CreateUser(_) => {
            vec![PlanNode::new("CreateUser", 1, 1, "")]
        }
        Statement::DropUser(_) => {
            vec![PlanNode::new("DropUser", 1, 1, "")]
        }
        Statement::AlterUser(_) => {
            vec![PlanNode::new("AlterUser", 1, 1, "")]
        }
        Statement::ShowUsers => {
            vec![PlanNode::new("ShowUsers", 1, 1, "")]
        }
        Statement::Explain(inner) => return build_plan_with_stats(inner, stats),
        Statement::Profile(inner) => return build_plan_with_stats(inner, stats),
        Statement::ProcedureCall(pc) => {
            vec![PlanNode::new("ProcedureCall", 1, 1, &pc.procedure)]
        }
    };

    QueryPlan { nodes }
}

/// Build a query plan from a statement (without executing)
pub fn build_plan(stmt: &Statement, node_count: u64, edge_count: u64) -> QueryPlan {
    let nodes = match stmt {
        Statement::Create(create) => plan_create(create),
        Statement::Match(m) => plan_match(m, node_count, edge_count),
        Statement::Delete(d) => plan_delete(d, node_count),
        Statement::Union(u) => plan_union(u, node_count, edge_count),
        Statement::MatchCreate(mc) => plan_match_create(mc, node_count, edge_count),
        Statement::MatchSet(ms) => plan_match_set(ms, node_count, edge_count),
        Statement::Merge(merge) => plan_merge(merge, node_count, edge_count),
        Statement::MatchRemove(mr) => plan_match_remove(mr, node_count, edge_count),
        Statement::Unwind(uw) => plan_unwind(uw),
        Statement::Foreach(_) => {
            vec![PlanNode::new("Foreach", 1, 1, "iterate list")]
        }
        Statement::MatchForeach(mf) => {
            let seg_count = mf.segments.len() as u64;
            vec![
                PlanNode::new(
                    "NodeByLabelScan",
                    seg_count.max(1),
                    seg_count.max(1) / 10 + 1,
                    "",
                ),
                PlanNode::new("Foreach", 1, 1, "iterate list"),
            ]
        }
        Statement::CreateConstraint(_) => {
            vec![PlanNode::new("CreateConstraint", 1, 1, "")]
        }
        Statement::DropConstraint(_) => {
            vec![PlanNode::new("DropConstraint", 1, 1, "")]
        }
        Statement::ShowConstraints => {
            vec![PlanNode::new("ShowConstraints", 1, 1, "")]
        }
        Statement::CreateFulltextIndex(_) => {
            vec![PlanNode::new("CreateFulltextIndex", 1, 1, "")]
        }
        Statement::DropFulltextIndex(_) => {
            vec![PlanNode::new("DropFulltextIndex", 1, 1, "")]
        }
        Statement::CreateUser(_) => {
            vec![PlanNode::new("CreateUser", 1, 1, "")]
        }
        Statement::DropUser(_) => {
            vec![PlanNode::new("DropUser", 1, 1, "")]
        }
        Statement::AlterUser(_) => {
            vec![PlanNode::new("AlterUser", 1, 1, "")]
        }
        Statement::ShowUsers => {
            vec![PlanNode::new("ShowUsers", 1, 1, "")]
        }
        Statement::Explain(inner) => return build_plan(inner, node_count, edge_count),
        Statement::Profile(inner) => return build_plan(inner, node_count, edge_count),
        Statement::ProcedureCall(pc) => {
            vec![PlanNode::new("ProcedureCall", 1, 1, &pc.procedure)]
        }
    };

    QueryPlan { nodes }
}

fn plan_create(create: &CreateClause) -> Vec<PlanNode> {
    let pattern_count = create.patterns.len() as u64;
    vec![PlanNode::new(
        "CreateNode",
        pattern_count,
        pattern_count,
        &format!("{} pattern(s)", pattern_count),
    )]
}

fn plan_match(m: &MatchStatement, node_count: u64, edge_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();

    for segment in &m.segments {
        for clause in &segment.match_clauses {
            for pattern in &clause.patterns {
                match pattern {
                    Pattern::Node(np) => {
                        let est = estimate_node_scan(np, node_count);
                        let label_info = np
                            .label
                            .as_ref()
                            .map(|l| format!(":{}", l))
                            .unwrap_or_default();
                        nodes.push(PlanNode::new(
                            "NodeByLabelScan",
                            est,
                            est / 10 + 1,
                            &label_info,
                        ));
                    }
                    Pattern::Path(pp) => {
                        let est_start = estimate_node_scan(&pp.start, node_count);
                        nodes.push(PlanNode::new(
                            "NodeByLabelScan",
                            est_start,
                            est_start / 10 + 1,
                            "",
                        ));
                        for seg in &pp.segments {
                            let est_expand = edge_count.max(1);
                            let edge_info = seg
                                .edge
                                .edge_type
                                .as_ref()
                                .map(|t| format!(":{}", t))
                                .unwrap_or_default();
                            nodes.push(PlanNode::new(
                                "Expand",
                                est_expand,
                                est_expand / 5 + 1,
                                &edge_info,
                            ));
                        }
                    }
                }
            }
        }

        if segment.where_clause.is_some() {
            let prev_est = nodes.last().map(|n| n.estimated_rows).unwrap_or(node_count);
            let filtered = prev_est / 2; // assume 50% selectivity
            nodes.push(PlanNode::new("Filter", filtered, filtered / 10 + 1, ""));
        }

        if segment.with_clause.is_some() {
            let prev_est = nodes.last().map(|n| n.estimated_rows).unwrap_or(1);
            nodes.push(PlanNode::new(
                "EagerAggregation",
                prev_est,
                prev_est / 5 + 1,
                "",
            ));
        }
    }

    if m.call_clause.is_some() {
        let prev_est = nodes.last().map(|n| n.estimated_rows).unwrap_or(1);
        nodes.push(PlanNode::new(
            "CallSubquery",
            prev_est,
            prev_est / 5 + 1,
            "",
        ));
    }

    let final_est = nodes.last().map(|n| n.estimated_rows).unwrap_or(1);
    nodes.push(PlanNode::new(
        "Projection",
        final_est,
        final_est / 10 + 1,
        "",
    ));

    if let Some(ref ob) = m.return_clause.order_by {
        nodes.push(PlanNode::new(
            "Sort",
            final_est,
            final_est * 2 + 1,
            &format!("{} key(s)", ob.items.len()),
        ));
    }
    if m.return_clause.limit.is_some() || m.return_clause.skip.is_some() {
        nodes.push(PlanNode::new("Limit", final_est, 1, ""));
    }

    nodes
}

fn plan_delete(d: &DeleteStatement, node_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    let est = node_count;
    nodes.push(PlanNode::new("NodeByLabelScan", est, est / 10 + 1, ""));
    if d.where_clause.is_some() {
        let filtered = est / 2;
        nodes.push(PlanNode::new("Filter", filtered, filtered / 10 + 1, ""));
    }
    let target = d.delete_clause.variables.len() as u64;
    nodes.push(PlanNode::new("Delete", target, target, ""));
    nodes
}

fn plan_union(u: &UnionStatement, node_count: u64, edge_count: u64) -> Vec<PlanNode> {
    let mut children = Vec::new();
    for query in &u.queries {
        let sub = plan_match(query, node_count, edge_count);
        for n in sub {
            children.push(n);
        }
    }
    let total_est: u64 = children.iter().map(|n| n.estimated_rows).sum();
    let mut union_node = PlanNode::new("Union", total_est, total_est / 5 + 1, "");
    union_node.children = children;
    vec![union_node]
}

fn plan_match_create(
    mc: &MatchCreateStatement,
    node_count: u64,
    _edge_count: u64,
) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new(
        "NodeByLabelScan",
        node_count,
        node_count / 10 + 1,
        "",
    ));
    if mc.where_clause.is_some() {
        nodes.push(PlanNode::new(
            "Filter",
            node_count / 2,
            node_count / 20 + 1,
            "",
        ));
    }
    let create_count = mc.create_clause.patterns.len() as u64;
    nodes.push(PlanNode::new("CreateNode", create_count, create_count, ""));
    nodes
}

fn plan_match_set(ms: &MatchSetStatement, node_count: u64, _edge_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new(
        "NodeByLabelScan",
        node_count,
        node_count / 10 + 1,
        "",
    ));
    if ms.where_clause.is_some() {
        nodes.push(PlanNode::new(
            "Filter",
            node_count / 2,
            node_count / 20 + 1,
            "",
        ));
    }
    let set_count = ms.set_clause.items.len() as u64;
    nodes.push(PlanNode::new("SetProperty", set_count, set_count, ""));
    nodes
}

fn plan_merge(merge: &MergeStatement, node_count: u64, edge_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    if !merge.match_clauses.is_empty() {
        nodes.push(PlanNode::new(
            "NodeByLabelScan",
            node_count,
            node_count / 10 + 1,
            "",
        ));
    }
    let pattern_count = merge.patterns.len() as u64;
    nodes.push(PlanNode::new(
        "Merge",
        pattern_count,
        node_count / 5 + edge_count / 5 + 1,
        "match-or-create",
    ));
    nodes
}

fn plan_match_remove(
    mr: &MatchRemoveStatement,
    node_count: u64,
    _edge_count: u64,
) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new(
        "NodeByLabelScan",
        node_count,
        node_count / 10 + 1,
        "",
    ));
    if mr.where_clause.is_some() {
        nodes.push(PlanNode::new(
            "Filter",
            node_count / 2,
            node_count / 20 + 1,
            "",
        ));
    }
    let remove_count = mr.remove_clause.items.len() as u64;
    nodes.push(PlanNode::new(
        "RemoveProperty",
        remove_count,
        remove_count,
        "",
    ));
    nodes
}

fn plan_unwind(uw: &UnwindStatement) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    // Estimate list size
    let est = match &uw.expression {
        Expression::List(items) => items.len() as u64,
        _ => 10, // default estimate
    };
    nodes.push(PlanNode::new("Unwind", est, est, ""));
    if uw.create_clause.is_some() {
        nodes.push(PlanNode::new("CreateNode", est, est, ""));
    }
    if uw.return_clause.is_some() {
        nodes.push(PlanNode::new("Projection", est, est / 10 + 1, ""));
    }
    nodes
}

fn estimate_node_scan(np: &NodePattern, node_count: u64) -> u64 {
    if np.label.is_some() {
        // With label filter, estimate ~10% of nodes
        (node_count / 10).max(1)
    } else {
        node_count.max(1)
    }
}

fn estimate_node_scan_with_stats(np: &NodePattern, stats: &GraphStats) -> u64 {
    if let Some(ref label) = np.label {
        stats.estimate_label_count(label)
    } else {
        stats.node_count.max(1)
    }
}

/// Check if a WHERE clause contains only property comparisons (pushable filters).
fn is_pushable_filter(expr: &Expression) -> bool {
    match expr {
        Expression::BinaryOp(left, op, right) => match op {
            BinaryOp::Eq
            | BinaryOp::Neq
            | BinaryOp::Lt
            | BinaryOp::Gt
            | BinaryOp::Lte
            | BinaryOp::Gte => is_simple_operand(left) && is_simple_operand(right),
            BinaryOp::And => is_pushable_filter(left) && is_pushable_filter(right),
            _ => false,
        },
        _ => false,
    }
}

fn is_simple_operand(expr: &Expression) -> bool {
    matches!(
        expr,
        Expression::Property(_, _) | Expression::Literal(_) | Expression::Variable(_)
    )
}

/// Filter classification for index selection.
enum FilterType {
    /// Equality filter: var.property = value
    IndexSeek { _variable: String, property: String },
    /// Range filter: var.property > value (or <, >=, <=)
    IndexRange { _variable: String, property: String },
    /// Not indexable
    Other,
}

/// Classify a WHERE expression for index selection.
fn classify_filter(expr: &Expression) -> FilterType {
    match expr {
        Expression::BinaryOp(left, op, right) => match op {
            BinaryOp::Eq => {
                if let Some((var, prop)) = extract_property_access(left) {
                    if is_literal_or_value(right) {
                        return FilterType::IndexSeek {
                            _variable: var,
                            property: prop,
                        };
                    }
                }
                if let Some((var, prop)) = extract_property_access(right) {
                    if is_literal_or_value(left) {
                        return FilterType::IndexSeek {
                            _variable: var,
                            property: prop,
                        };
                    }
                }
                FilterType::Other
            }
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Lte | BinaryOp::Gte => {
                if let Some((var, prop)) = extract_property_access(left) {
                    if is_literal_or_value(right) {
                        return FilterType::IndexRange {
                            _variable: var,
                            property: prop,
                        };
                    }
                }
                if let Some((var, prop)) = extract_property_access(right) {
                    if is_literal_or_value(left) {
                        return FilterType::IndexRange {
                            _variable: var,
                            property: prop,
                        };
                    }
                }
                FilterType::Other
            }
            BinaryOp::And => {
                // For AND, check the left side first
                let left_type = classify_filter(left);
                if matches!(left_type, FilterType::IndexSeek { .. }) {
                    return left_type;
                }
                classify_filter(right)
            }
            _ => FilterType::Other,
        },
        _ => FilterType::Other,
    }
}

fn extract_property_access(expr: &Expression) -> Option<(String, String)> {
    if let Expression::Property(var, prop) = expr {
        Some((var.clone(), prop.clone()))
    } else {
        None
    }
}

fn is_literal_or_value(expr: &Expression) -> bool {
    matches!(expr, Expression::Literal(_) | Expression::Variable(_))
}

fn plan_match_with_stats(m: &MatchStatement, stats: &GraphStats) -> Vec<PlanNode> {
    let mut nodes = Vec::new();

    for segment in &m.segments {
        let mut used_index = false;

        for clause in &segment.match_clauses {
            for pattern in &clause.patterns {
                match pattern {
                    Pattern::Node(np) => {
                        let est = estimate_node_scan_with_stats(np, stats);
                        let label_info = np
                            .label
                            .as_ref()
                            .map(|l| format!(":{}", l))
                            .unwrap_or_default();

                        // Index selection: check if WHERE filters on an indexed property
                        if let (Some(label), Some(where_expr)) = (&np.label, &segment.where_clause)
                        {
                            let filter_type = classify_filter(where_expr);
                            match filter_type {
                                FilterType::IndexSeek { ref property, .. }
                                    if stats.has_index(label, property) =>
                                {
                                    // IndexSeek: O(1) lookup, very cheap
                                    let seek_est = 1u64.max(est / 100);
                                    nodes.push(PlanNode::new(
                                        "IndexSeek",
                                        seek_est,
                                        2,
                                        &format!("{}.{}", label_info, property),
                                    ));
                                    used_index = true;
                                    continue;
                                }
                                FilterType::IndexRange { ref property, .. }
                                    if stats.has_index(label, property) =>
                                {
                                    // IndexRangeScan: scan a range in the index
                                    let range_est = (est / 5).max(1);
                                    nodes.push(PlanNode::new(
                                        "IndexRangeScan",
                                        range_est,
                                        range_est / 10 + 1,
                                        &format!("{}.{}", label_info, property),
                                    ));
                                    used_index = true;
                                    continue;
                                }
                                _ => {}
                            }

                            // Filter pushdown: if WHERE is a simple property comparison,
                            // show it as part of the scan node
                            if is_pushable_filter(where_expr) {
                                let filtered = (est / 2).max(1);
                                nodes.push(PlanNode::new(
                                    "NodeByLabelScan+Filter",
                                    filtered,
                                    est / 10 + 1,
                                    &format!("{} (filter pushed down)", label_info),
                                ));
                                continue;
                            }
                        } else if let Some(ref where_expr) = segment.where_clause {
                            // No label but has WHERE - still try filter pushdown
                            if is_pushable_filter(where_expr) {
                                let filtered = (est / 2).max(1);
                                nodes.push(PlanNode::new(
                                    "NodeByLabelScan+Filter",
                                    filtered,
                                    est / 10 + 1,
                                    &format!("{} (filter pushed down)", label_info),
                                ));
                                continue;
                            }
                        }

                        nodes.push(PlanNode::new(
                            "NodeByLabelScan",
                            est,
                            est / 10 + 1,
                            &label_info,
                        ));
                    }
                    Pattern::Path(pp) => {
                        let est_start = estimate_node_scan_with_stats(&pp.start, stats);

                        // Join order optimization: for single-segment paths, check if
                        // starting from the end node would be cheaper (smaller label count)
                        if pp.segments.len() == 1 {
                            let end_node = &pp.segments[0].node;
                            let est_end = estimate_node_scan_with_stats(end_node, stats);

                            let start_label = pp
                                .start
                                .label
                                .as_ref()
                                .map(|l| format!(":{}", l))
                                .unwrap_or_default();
                            let end_label = end_node
                                .label
                                .as_ref()
                                .map(|l| format!(":{}", l))
                                .unwrap_or_default();
                            let edge_info = pp.segments[0]
                                .edge
                                .edge_type
                                .as_ref()
                                .map(|t| format!(":{}", t))
                                .unwrap_or_default();

                            if est_end < est_start && end_node.label.is_some() {
                                // Reverse: scan end node first, expand backward
                                nodes.push(PlanNode::new(
                                    "NodeByLabelScan",
                                    est_end,
                                    est_end / 10 + 1,
                                    &format!("{} (reordered)", end_label),
                                ));
                                let expand_est =
                                    (est_end * stats.edge_count / stats.node_count.max(1)).max(1);
                                nodes.push(PlanNode::new(
                                    "ExpandReverse",
                                    expand_est,
                                    expand_est / 5 + 1,
                                    &format!(
                                        "{} (join reordered: {} < {})",
                                        edge_info, end_label, start_label
                                    ),
                                ));
                            } else {
                                // Normal order
                                nodes.push(PlanNode::new(
                                    "NodeByLabelScan",
                                    est_start,
                                    est_start / 10 + 1,
                                    &start_label,
                                ));
                                let est_expand = stats.edge_count.max(1);
                                nodes.push(PlanNode::new(
                                    "Expand",
                                    est_expand,
                                    est_expand / 5 + 1,
                                    &edge_info,
                                ));
                            }
                        } else {
                            // Multi-segment path: use original ordering
                            let start_label = pp
                                .start
                                .label
                                .as_ref()
                                .map(|l| format!(":{}", l))
                                .unwrap_or_default();
                            nodes.push(PlanNode::new(
                                "NodeByLabelScan",
                                est_start,
                                est_start / 10 + 1,
                                &start_label,
                            ));
                            for seg in &pp.segments {
                                let est_expand = stats.edge_count.max(1);
                                let edge_info = seg
                                    .edge
                                    .edge_type
                                    .as_ref()
                                    .map(|t| format!(":{}", t))
                                    .unwrap_or_default();
                                nodes.push(PlanNode::new(
                                    "Expand",
                                    est_expand,
                                    est_expand / 5 + 1,
                                    &edge_info,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Only add separate Filter if not already handled by index or pushdown
        if segment.where_clause.is_some() && !used_index {
            let already_pushed = nodes
                .last()
                .map(|n| n.operator == "NodeByLabelScan+Filter")
                .unwrap_or(false);

            if !already_pushed {
                let prev_est = nodes
                    .last()
                    .map(|n| n.estimated_rows)
                    .unwrap_or(stats.node_count);
                let filtered = (prev_est / 2).max(1);
                nodes.push(PlanNode::new("Filter", filtered, filtered / 10 + 1, ""));
            }
        }

        if segment.with_clause.is_some() {
            let prev_est = nodes.last().map(|n| n.estimated_rows).unwrap_or(1);
            nodes.push(PlanNode::new(
                "EagerAggregation",
                prev_est,
                prev_est / 5 + 1,
                "",
            ));
        }
    }

    let final_est = nodes.last().map(|n| n.estimated_rows).unwrap_or(1);

    // Column pruning: analyze RETURN clause to determine needed columns
    let projection_details = analyze_projection(&m.return_clause);
    let projection_cost = if projection_details.is_empty() {
        final_est / 10 + 1
    } else {
        // Reduced cost when only fetching specific properties
        final_est / 20 + 1
    };
    nodes.push(PlanNode::new(
        "Projection",
        final_est,
        projection_cost,
        &projection_details,
    ));

    if let Some(ref ob) = m.return_clause.order_by {
        nodes.push(PlanNode::new(
            "Sort",
            final_est,
            final_est * 2 + 1,
            &format!("{} key(s)", ob.items.len()),
        ));
    }
    if m.return_clause.limit.is_some() || m.return_clause.skip.is_some() {
        nodes.push(PlanNode::new("Limit", final_est, 1, ""));
    }

    nodes
}

/// Analyze the RETURN clause to determine which columns are needed.
/// Returns a details string describing the pruning.
fn analyze_projection(return_clause: &ReturnClause) -> String {
    let mut has_all = false;
    let mut has_variable = false;
    let mut properties: Vec<String> = Vec::new();

    for item in &return_clause.items {
        match item {
            ReturnItem::All => {
                has_all = true;
            }
            ReturnItem::Variable(_) => {
                has_variable = true;
            }
            ReturnItem::Property(var, prop) => {
                properties.push(format!("{}.{}", var, prop));
            }
            ReturnItem::Aggregate(_) | ReturnItem::Function(_) | ReturnItem::Expr(_) => {
                // Aggregates need full data, no pruning possible
                has_variable = true;
            }
        }
    }

    if has_all {
        // RETURN * needs everything
        String::new()
    } else if !has_variable && !properties.is_empty() {
        // Only specific properties returned - column pruning applies
        format!("columns: {}", properties.join(", "))
    } else {
        String::new()
    }
}

fn plan_delete_with_stats(d: &DeleteStatement, stats: &GraphStats) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    let est = stats.node_count;
    nodes.push(PlanNode::new("NodeByLabelScan", est, est / 10 + 1, ""));
    if d.where_clause.is_some() {
        let filtered = est / 2;
        nodes.push(PlanNode::new("Filter", filtered, filtered / 10 + 1, ""));
    }
    let target = d.delete_clause.variables.len() as u64;
    nodes.push(PlanNode::new("Delete", target, target, ""));
    nodes
}

fn plan_union_with_stats(u: &UnionStatement, stats: &GraphStats) -> Vec<PlanNode> {
    let mut children = Vec::new();
    for query in &u.queries {
        let sub = plan_match_with_stats(query, stats);
        for n in sub {
            children.push(n);
        }
    }
    let total_est: u64 = children.iter().map(|n| n.estimated_rows).sum();
    let mut union_node = PlanNode::new("Union", total_est, total_est / 5 + 1, "");
    union_node.children = children;
    vec![union_node]
}

fn plan_match_create_with_stats(mc: &MatchCreateStatement, stats: &GraphStats) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new(
        "NodeByLabelScan",
        stats.node_count,
        stats.node_count / 10 + 1,
        "",
    ));
    if mc.where_clause.is_some() {
        nodes.push(PlanNode::new(
            "Filter",
            stats.node_count / 2,
            stats.node_count / 20 + 1,
            "",
        ));
    }
    let create_count = mc.create_clause.patterns.len() as u64;
    nodes.push(PlanNode::new("CreateNode", create_count, create_count, ""));
    nodes
}

fn plan_match_set_with_stats(ms: &MatchSetStatement, stats: &GraphStats) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new(
        "NodeByLabelScan",
        stats.node_count,
        stats.node_count / 10 + 1,
        "",
    ));
    if ms.where_clause.is_some() {
        nodes.push(PlanNode::new(
            "Filter",
            stats.node_count / 2,
            stats.node_count / 20 + 1,
            "",
        ));
    }
    let set_count = ms.set_clause.items.len() as u64;
    nodes.push(PlanNode::new("SetProperty", set_count, set_count, ""));
    nodes
}

fn plan_merge_with_stats(merge: &MergeStatement, stats: &GraphStats) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    if !merge.match_clauses.is_empty() {
        nodes.push(PlanNode::new(
            "NodeByLabelScan",
            stats.node_count,
            stats.node_count / 10 + 1,
            "",
        ));
    }
    let pattern_count = merge.patterns.len() as u64;
    nodes.push(PlanNode::new(
        "Merge",
        pattern_count,
        stats.node_count / 5 + stats.edge_count / 5 + 1,
        "match-or-create",
    ));
    nodes
}

fn plan_match_remove_with_stats(mr: &MatchRemoveStatement, stats: &GraphStats) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new(
        "NodeByLabelScan",
        stats.node_count,
        stats.node_count / 10 + 1,
        "",
    ));
    if mr.where_clause.is_some() {
        nodes.push(PlanNode::new(
            "Filter",
            stats.node_count / 2,
            stats.node_count / 20 + 1,
            "",
        ));
    }
    let remove_count = mr.remove_clause.items.len() as u64;
    nodes.push(PlanNode::new(
        "RemoveProperty",
        remove_count,
        remove_count,
        "",
    ));
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn plan(input: &str) -> QueryPlan {
        let stmt = Parser::new(input).unwrap().parse().unwrap();
        build_plan(&stmt, 1000, 5000)
    }

    #[test]
    fn test_plan_match() {
        let p = plan("MATCH (n:Person) RETURN n");
        assert!(!p.nodes.is_empty());
        assert_eq!(p.nodes[0].operator, "NodeByLabelScan");
    }

    #[test]
    fn test_plan_match_with_filter() {
        let p = plan("MATCH (n:Person) WHERE n.age > 30 RETURN n");
        let ops: Vec<&str> = p.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"NodeByLabelScan"));
        assert!(ops.contains(&"Filter"));
        assert!(ops.contains(&"Projection"));
    }

    #[test]
    fn test_plan_match_path() {
        let p = plan("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b");
        let ops: Vec<&str> = p.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"NodeByLabelScan"));
        assert!(ops.contains(&"Expand"));
    }

    #[test]
    fn test_plan_create() {
        let p = plan("CREATE (n:Person {name: \"Alice\"})");
        assert_eq!(p.nodes[0].operator, "CreateNode");
    }

    #[test]
    fn test_plan_delete() {
        let p = plan("MATCH (n:Person) DELETE n");
        let ops: Vec<&str> = p.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"Delete"));
    }

    #[test]
    fn test_plan_merge() {
        let p = plan("MERGE (n:Person {name: \"Alice\"})");
        let ops: Vec<&str> = p.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"Merge"));
    }

    #[test]
    fn test_plan_unwind() {
        let p = plan("UNWIND [1, 2, 3] AS x RETURN x");
        let ops: Vec<&str> = p.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"Unwind"));
    }

    #[test]
    fn test_plan_with_order_by() {
        let p = plan("MATCH (n:Person) RETURN n ORDER BY n.name");
        let ops: Vec<&str> = p.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"Sort"));
    }

    #[test]
    fn test_plan_with_limit() {
        let p = plan("MATCH (n:Person) RETURN n LIMIT 10");
        let ops: Vec<&str> = p.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"Limit"));
    }

    #[test]
    fn test_plan_display() {
        let p = plan("MATCH (n:Person) WHERE n.age > 30 RETURN n");
        let display = format!("{}", p);
        assert!(display.contains("NodeByLabelScan"));
        assert!(display.contains("Filter"));
        assert!(display.contains("Projection"));
    }

    #[test]
    fn test_explain_is_parsed() {
        let stmt = Parser::new("EXPLAIN MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        assert!(matches!(stmt, Statement::Explain(_)));
    }

    #[test]
    fn test_profile_is_parsed() {
        let stmt = Parser::new("PROFILE MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        assert!(matches!(stmt, Statement::Profile(_)));
    }

    // ===== GraphStats tests =====

    #[test]
    fn test_graph_stats_from_graph() {
        let mut graph = maharit_core::Graph::new();
        graph.create_node("Person");
        graph.create_node("Person");
        graph.create_node("Person");
        graph.create_node("Company");

        let stats = GraphStats::from_graph(&graph);
        assert_eq!(stats.node_count, 4);
        assert_eq!(stats.edge_count, 0);
        assert_eq!(stats.label_counts.get("Person"), Some(&3));
        assert_eq!(stats.label_counts.get("Company"), Some(&1));
    }

    #[test]
    fn test_graph_stats_estimate_label_count() {
        let mut label_counts = HashMap::new();
        label_counts.insert("Person".to_string(), 500);
        label_counts.insert("Company".to_string(), 50);

        let stats = GraphStats {
            node_count: 550,
            edge_count: 1000,
            label_counts,
            indexed_properties: HashSet::new(),
        };

        assert_eq!(stats.estimate_label_count("Person"), 500);
        assert_eq!(stats.estimate_label_count("Company"), 50);
        assert_eq!(stats.estimate_label_count("Unknown"), 1);
    }

    #[test]
    fn test_graph_stats_simple_fallback() {
        let stats = GraphStats::simple(1000, 5000);

        // No label distribution, should use 10% heuristic
        assert_eq!(stats.estimate_label_count("Person"), 100);
    }

    #[test]
    fn test_build_plan_with_stats_label_estimate() {
        let mut label_counts = HashMap::new();
        label_counts.insert("Person".to_string(), 500);
        label_counts.insert("Company".to_string(), 50);

        let stats = GraphStats {
            node_count: 550,
            edge_count: 1000,
            label_counts,
            indexed_properties: HashSet::new(),
        };

        let stmt = Parser::new("MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        // Should use actual label count (500) instead of 10% heuristic (55)
        let scan = &plan.nodes[0];
        assert_eq!(scan.operator, "NodeByLabelScan");
        assert_eq!(scan.estimated_rows, 500);
    }

    #[test]
    fn test_build_plan_with_stats_small_label() {
        let mut label_counts = HashMap::new();
        label_counts.insert("Person".to_string(), 500);
        label_counts.insert("Admin".to_string(), 5);

        let stats = GraphStats {
            node_count: 505,
            edge_count: 100,
            label_counts,
            indexed_properties: HashSet::new(),
        };

        let stmt = Parser::new("MATCH (n:Admin) RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let scan = &plan.nodes[0];
        assert_eq!(scan.estimated_rows, 5);
    }

    // ===== Filter pushdown tests =====

    #[test]
    fn test_filter_pushdown_simple_comparison() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt = Parser::new("MATCH (n:Person) WHERE n.age > 30 RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        // Simple property comparison should be pushed down
        assert!(ops.contains(&"NodeByLabelScan+Filter"));
        assert!(ops.iter().filter(|&&o| o == "Filter").count() == 0);
    }

    #[test]
    fn test_filter_pushdown_equality() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt = Parser::new("MATCH (n:Person) WHERE n.name = 'Alice' RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"NodeByLabelScan+Filter"));
    }

    #[test]
    fn test_filter_pushdown_and_conditions() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt = Parser::new("MATCH (n:Person) WHERE n.age > 20 AND n.age < 60 RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"NodeByLabelScan+Filter"));
    }

    #[test]
    fn test_no_filter_pushdown_for_path() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt = Parser::new("MATCH (a:Person)-[:KNOWS]->(b) WHERE b.age > 30 RETURN a, b")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        // Path patterns should not get pushdown (filter applies after expand)
        assert!(ops.contains(&"Filter"));
        assert!(!ops.contains(&"NodeByLabelScan+Filter"));
    }

    // ===== Index selection tests =====

    fn stats_with_index(label: &str, property: &str) -> GraphStats {
        let mut stats = GraphStats::simple(1000, 5000);
        stats
            .indexed_properties
            .insert((label.to_string(), property.to_string()));
        stats
    }

    #[test]
    fn test_index_seek_on_equality() {
        let stats = stats_with_index("Person", "email");

        let stmt = Parser::new("MATCH (n:Person) WHERE n.email = 'alice@test.com' RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"IndexSeek"));
        assert!(!ops.contains(&"NodeByLabelScan"));
        assert!(!ops.contains(&"Filter"));
    }

    #[test]
    fn test_index_range_scan_on_comparison() {
        let stats = stats_with_index("Person", "age");

        let stmt = Parser::new("MATCH (n:Person) WHERE n.age > 30 RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"IndexRangeScan"));
        assert!(!ops.contains(&"NodeByLabelScan"));
    }

    #[test]
    fn test_no_index_when_not_indexed() {
        // No index on Person.age
        let stats = stats_with_index("Person", "email");

        let stmt = Parser::new("MATCH (n:Person) WHERE n.age > 30 RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        // Should fall back to filter pushdown, not index scan
        assert!(!ops.contains(&"IndexSeek"));
        assert!(!ops.contains(&"IndexRangeScan"));
        assert!(ops.contains(&"NodeByLabelScan+Filter"));
    }

    #[test]
    fn test_index_seek_lower_cost_than_scan() {
        let stats = stats_with_index("Person", "name");

        let stmt = Parser::new("MATCH (n:Person) WHERE n.name = 'Alice' RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        // IndexSeek should have very low cost
        let seek_node = plan
            .nodes
            .iter()
            .find(|n| n.operator == "IndexSeek")
            .unwrap();
        assert!(seek_node.estimated_cost <= 2);
        assert!(seek_node.estimated_rows <= 10);
    }

    #[test]
    fn test_index_seek_details_show_property() {
        let stats = stats_with_index("Person", "email");

        let stmt = Parser::new("MATCH (n:Person) WHERE n.email = 'test' RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let seek_node = plan
            .nodes
            .iter()
            .find(|n| n.operator == "IndexSeek")
            .unwrap();
        assert!(seek_node.details.contains("email"));
    }

    #[test]
    fn test_has_index() {
        let mut stats = GraphStats::simple(100, 50);
        assert!(!stats.has_index("Person", "name"));

        stats
            .indexed_properties
            .insert(("Person".to_string(), "name".to_string()));
        assert!(stats.has_index("Person", "name"));
        assert!(!stats.has_index("Person", "age"));
        assert!(!stats.has_index("Company", "name"));
    }

    // ===== Join order optimization tests =====

    fn stats_with_labels(labels: &[(&str, u64)], edge_count: u64) -> GraphStats {
        let mut label_counts = HashMap::new();
        let mut total = 0u64;
        for &(label, count) in labels {
            label_counts.insert(label.to_string(), count);
            total += count;
        }
        GraphStats {
            node_count: total,
            edge_count,
            label_counts,
            indexed_properties: HashSet::new(),
        }
    }

    #[test]
    fn test_join_reorder_smaller_end_node() {
        // Person has 1000 nodes, Admin has 5 nodes
        // (a:Person)-[:MANAGES]->(b:Admin) should start from Admin
        let stats = stats_with_labels(&[("Person", 1000), ("Admin", 5)], 500);

        let stmt = Parser::new("MATCH (a:Person)-[:MANAGES]->(b:Admin) RETURN a, b")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"ExpandReverse"));

        // First scan should be Admin (smaller)
        let scan = &plan.nodes[0];
        assert_eq!(scan.operator, "NodeByLabelScan");
        assert_eq!(scan.estimated_rows, 5);
        assert!(scan.details.contains("reordered"));
    }

    #[test]
    fn test_join_no_reorder_when_start_is_smaller() {
        // Admin has 5, Person has 1000
        // (a:Admin)-[:REPORTS_TO]->(b:Person) should keep original order
        let stats = stats_with_labels(&[("Admin", 5), ("Person", 1000)], 500);

        let stmt = Parser::new("MATCH (a:Admin)-[:REPORTS_TO]->(b:Person) RETURN a, b")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"Expand"));
        assert!(!ops.contains(&"ExpandReverse"));

        let scan = &plan.nodes[0];
        assert_eq!(scan.estimated_rows, 5);
    }

    #[test]
    fn test_join_reorder_shows_edge_info() {
        let stats = stats_with_labels(&[("Person", 1000), ("Admin", 5)], 500);

        let stmt = Parser::new("MATCH (a:Person)-[:MANAGES]->(b:Admin) RETURN a, b")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let expand = plan
            .nodes
            .iter()
            .find(|n| n.operator == "ExpandReverse")
            .unwrap();
        assert!(expand.details.contains("MANAGES"));
        assert!(expand.details.contains("reordered"));
    }

    #[test]
    fn test_join_equal_labels_no_reorder() {
        // Same count: keep original order
        let stats = stats_with_labels(&[("Person", 100), ("Employee", 100)], 500);

        let stmt = Parser::new("MATCH (a:Person)-[:WORKS_AS]->(b:Employee) RETURN a, b")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let ops: Vec<&str> = plan.nodes.iter().map(|n| n.operator.as_str()).collect();
        assert!(ops.contains(&"Expand"));
        assert!(!ops.contains(&"ExpandReverse"));
    }

    // ===== Column pruning tests =====

    #[test]
    fn test_column_pruning_specific_properties() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt = Parser::new("MATCH (n:Person) RETURN n.name, n.age")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let projection = plan
            .nodes
            .iter()
            .find(|n| n.operator == "Projection")
            .unwrap();
        assert!(projection.details.contains("n.name"));
        assert!(projection.details.contains("n.age"));
        assert!(projection.details.starts_with("columns: "));
    }

    #[test]
    fn test_no_column_pruning_return_variable() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt = Parser::new("MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let projection = plan
            .nodes
            .iter()
            .find(|n| n.operator == "Projection")
            .unwrap();
        // No pruning when returning full variable
        assert!(projection.details.is_empty());
    }

    #[test]
    fn test_no_column_pruning_return_all() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt = Parser::new("MATCH (n:Person) RETURN *")
            .unwrap()
            .parse()
            .unwrap();
        let plan = build_plan_with_stats(&stmt, &stats);

        let projection = plan
            .nodes
            .iter()
            .find(|n| n.operator == "Projection")
            .unwrap();
        assert!(projection.details.is_empty());
    }

    #[test]
    fn test_column_pruning_reduces_cost() {
        let stats = GraphStats::simple(1000, 5000);

        let stmt_full = Parser::new("MATCH (n:Person) RETURN n")
            .unwrap()
            .parse()
            .unwrap();
        let plan_full = build_plan_with_stats(&stmt_full, &stats);

        let stmt_pruned = Parser::new("MATCH (n:Person) RETURN n.name")
            .unwrap()
            .parse()
            .unwrap();
        let plan_pruned = build_plan_with_stats(&stmt_pruned, &stats);

        let cost_full = plan_full
            .nodes
            .iter()
            .find(|n| n.operator == "Projection")
            .unwrap()
            .estimated_cost;
        let cost_pruned = plan_pruned
            .nodes
            .iter()
            .find(|n| n.operator == "Projection")
            .unwrap()
            .estimated_cost;

        assert!(cost_pruned < cost_full);
    }
}
