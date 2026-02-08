use std::fmt;

use crate::ast::*;

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
                            .map(|l| format!(":{}",l))
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
            nodes.push(PlanNode::new("EagerAggregation", prev_est, prev_est / 5 + 1, ""));
        }
    }

    let final_est = nodes.last().map(|n| n.estimated_rows).unwrap_or(1);
    nodes.push(PlanNode::new("Projection", final_est, final_est / 10 + 1, ""));

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

fn plan_match_create(mc: &MatchCreateStatement, node_count: u64, _edge_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new("NodeByLabelScan", node_count, node_count / 10 + 1, ""));
    if mc.where_clause.is_some() {
        nodes.push(PlanNode::new("Filter", node_count / 2, node_count / 20 + 1, ""));
    }
    let create_count = mc.create_clause.patterns.len() as u64;
    nodes.push(PlanNode::new("CreateNode", create_count, create_count, ""));
    nodes
}

fn plan_match_set(ms: &MatchSetStatement, node_count: u64, _edge_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new("NodeByLabelScan", node_count, node_count / 10 + 1, ""));
    if ms.where_clause.is_some() {
        nodes.push(PlanNode::new("Filter", node_count / 2, node_count / 20 + 1, ""));
    }
    let set_count = ms.set_clause.items.len() as u64;
    nodes.push(PlanNode::new("SetProperty", set_count, set_count, ""));
    nodes
}

fn plan_merge(merge: &MergeStatement, node_count: u64, edge_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    if !merge.match_clauses.is_empty() {
        nodes.push(PlanNode::new("NodeByLabelScan", node_count, node_count / 10 + 1, ""));
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

fn plan_match_remove(mr: &MatchRemoveStatement, node_count: u64, _edge_count: u64) -> Vec<PlanNode> {
    let mut nodes = Vec::new();
    nodes.push(PlanNode::new("NodeByLabelScan", node_count, node_count / 10 + 1, ""));
    if mr.where_clause.is_some() {
        nodes.push(PlanNode::new("Filter", node_count / 2, node_count / 20 + 1, ""));
    }
    let remove_count = mr.remove_clause.items.len() as u64;
    nodes.push(PlanNode::new("RemoveProperty", remove_count, remove_count, ""));
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
}
