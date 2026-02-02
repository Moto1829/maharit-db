use std::collections::HashMap;

use maharit_core::{Edge, Graph, NodeId, PropertyValue};
use thiserror::Error;

use crate::ast::*;

/// 実行エラー
#[derive(Debug, Clone, Error, PartialEq)]
pub enum ExecuteError {
    #[error("undefined variable: {0}")]
    UndefinedVariable(String),

    #[error("type error: {0}")]
    TypeError(String),

    #[error("graph error: {0}")]
    GraphError(#[from] maharit_core::GraphError),
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
        label: String,
        properties: HashMap<String, PropertyValue>,
    },
    /// リスト値（可変長パスのエッジリストなど）
    List(Vec<Value>),
    /// パス値（ノードとエッジの交互シーケンス）
    Path {
        nodes: Vec<NodeId>,
        edges: Vec<u64>,
    },
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
            Value::NodeData { id, label, .. } => write!(f, "({}:{})", id, label),
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
    Path {
        nodes: Vec<NodeId>,
        edges: Vec<u64>,
    },
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
    graph: &'a mut Graph,
}

impl<'a> Executor<'a> {
    pub fn new(graph: &'a mut Graph) -> Self {
        Self { graph }
    }

    /// 文を実行
    pub fn execute(&mut self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
        match stmt {
            Statement::Create(create) => self.execute_create(create),
            Statement::Match(m) => self.execute_match(m),
            Statement::Delete(d) => self.execute_delete(d),
        }
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
                        let edge_id = self.graph.create_edge(from, to, edge_label)?;

                        // Set edge properties
                        if let Some(edge) = self.graph.get_edge_mut(edge_id) {
                            for (key, value) in segment.edge.properties {
                                edge.set_property(key, PropertyValue::from(value));
                            }
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
        let label = pattern.label.clone().unwrap_or_default();
        let node_id = self.graph.create_node(label);

        // Set properties
        if let Some(node) = self.graph.get_node_mut(node_id) {
            for (key, value) in &pattern.properties {
                node.set_property(key.clone(), PropertyValue::from(value.clone()));
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
            all_bindings = all_bindings
                .into_iter()
                .filter(|bindings| {
                    self.evaluate_expression(where_expr, bindings)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                })
                .collect();
        }

        // Apply SET clause
        if let Some(set_clause) = &d.set_clause {
            for bindings in &all_bindings {
                for item in &set_clause.items {
                    let node_id = bindings
                        .get(&item.variable)
                        .and_then(|v| v.as_node())
                        .ok_or_else(|| ExecuteError::UndefinedVariable(item.variable.clone()))?;

                    let value = self.evaluate_expression(&item.value, bindings)?;
                    let prop_value = self.value_to_property(&value)?;

                    if let Some(node) = self.graph.get_node_mut(node_id) {
                        node.set_property(&item.property, prop_value);
                    }
                }
            }
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
                        BindingValue::Path { .. } => {
                            // Paths cannot be deleted directly
                        }
                    }
                }
            }
        }

        // Delete edges first
        let mut deleted_edges = 0;
        for edge_id in edges_to_delete {
            if self.graph.delete_edge(edge_id).is_some() {
                deleted_edges += 1;
            }
        }

        // Delete nodes (with DETACH if specified)
        let mut deleted_nodes = 0;
        for node_id in nodes_to_delete {
            if d.delete_clause.detach {
                // delete_node already handles related edges
                if self.graph.delete_node(node_id).is_some() {
                    deleted_nodes += 1;
                }
            } else {
                // Check if node has edges
                let has_edges = !self.graph.get_outgoing_edges(node_id).is_empty()
                    || !self.graph.get_incoming_edges(node_id).is_empty();

                if has_edges {
                    // In a real Cypher implementation, this would be an error
                    // For simplicity, we just skip or we could return an error
                    continue;
                }

                if self.graph.delete_node(node_id).is_some() {
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

    fn value_to_property(&self, value: &Value) -> Result<PropertyValue, ExecuteError> {
        match value {
            Value::Null => Ok(PropertyValue::Null),
            Value::Bool(b) => Ok(PropertyValue::Bool(*b)),
            Value::Int(n) => Ok(PropertyValue::Int(*n)),
            Value::Float(n) => Ok(PropertyValue::Float(*n)),
            Value::String(s) => Ok(PropertyValue::String(s.clone())),
            _ => Err(ExecuteError::TypeError(
                "cannot convert to property value".to_string(),
            )),
        }
    }

    // ========== MATCH ==========

    fn execute_match(&mut self, m: MatchStatement) -> Result<ResultSet, ExecuteError> {
        // Find all matching bindings
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for pattern in &m.patterns {
            all_bindings = self.match_pattern(pattern, all_bindings)?;
        }

        // Apply WHERE filter
        if let Some(where_expr) = &m.where_clause {
            all_bindings = all_bindings
                .into_iter()
                .filter(|bindings| {
                    self.evaluate_expression(where_expr, bindings)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                })
                .collect();
        }

        // Build result set
        self.build_result_set(&m.return_clause, &all_bindings)
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

    fn match_node_pattern(
        &self,
        pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        for bindings in current_bindings {
            // Check if variable is already bound
            if let Some(var) = &pattern.variable {
                if let Some(bound_value) = bindings.get(var) {
                    if let Some(bound_id) = bound_value.as_node() {
                        // Variable already bound, check if it matches
                        if self.node_matches_pattern(bound_id, pattern) {
                            result.push(bindings);
                        }
                    }
                    continue;
                }
            }

            // Find matching nodes
            for node in self.graph.nodes() {
                if self.node_matches_pattern(node.id, pattern) {
                    let mut new_bindings = bindings.clone();
                    if let Some(var) = &pattern.variable {
                        new_bindings.insert(var.clone(), BindingValue::Node(node.id));
                    }
                    result.push(new_bindings);
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
            if let Some(ref edge_type) = segment.edge.edge_type {
                if &edge.label != edge_type {
                    continue;
                }
            }

            // Get the other node
            let next_id = self.get_next_node(prev_id, &edge, segment.edge.direction);

            // Check if next node matches pattern
            if self.node_matches_pattern(next_id, &segment.node) {
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
                if depth >= range.min && self.node_matches_pattern(current_id, &segment.node) {
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
                    if let Some(ref edge_type) = segment.edge.edge_type {
                        if &edge.label != edge_type {
                            continue;
                        }
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

    fn get_edges_by_direction(&self, node_id: NodeId, direction: EdgeDirection) -> Vec<&Edge> {
        match direction {
            EdgeDirection::Outgoing => self.graph.get_outgoing_edges(node_id),
            EdgeDirection::Incoming => self.graph.get_incoming_edges(node_id),
            EdgeDirection::Both => {
                let mut edges = self.graph.get_outgoing_edges(node_id);
                edges.extend(self.graph.get_incoming_edges(node_id));
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

    fn node_matches_pattern(&self, node_id: NodeId, pattern: &NodePattern) -> bool {
        let Some(node) = self.graph.get_node(node_id) else {
            return false;
        };

        // Check label
        if let Some(ref label) = pattern.label {
            if &node.label != label {
                return false;
            }
        }

        // Check properties
        for (key, expected) in &pattern.properties {
            match node.get_property(key) {
                Some(actual) => {
                    if !self.property_matches(actual, expected) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        true
    }

    fn property_matches(&self, actual: &PropertyValue, expected: &Literal) -> bool {
        match (actual, expected) {
            (PropertyValue::Null, Literal::Null) => true,
            (PropertyValue::Bool(a), Literal::Bool(e)) => a == e,
            (PropertyValue::Int(a), Literal::Int(e)) => a == e,
            (PropertyValue::Float(a), Literal::Float(e)) => (a - e).abs() < f64::EPSILON,
            (PropertyValue::String(a), Literal::String(e)) => a == e,
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
            .any(|item| matches!(item, ReturnItem::Aggregate(_)));

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

        // Apply ORDER BY with optional LIMIT optimization
        if let Some(ref order_by) = return_clause.order_by {
            // Calculate how many rows we actually need
            let needed = match (return_clause.skip, return_clause.limit) {
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
        if let Some(skip) = return_clause.skip {
            let skip = skip as usize;
            if skip < rows.len() {
                rows = rows.into_iter().skip(skip).collect();
            } else {
                rows.clear();
            }
        }

        // Apply LIMIT
        if let Some(limit) = return_clause.limit {
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

    fn return_item_to_column_name(&self, item: &ReturnItem) -> String {
        match item {
            ReturnItem::Variable(v) => v.clone(),
            ReturnItem::Property(v, p) => format!("{}.{}", v, p),
            ReturnItem::All => "*".to_string(),
            ReturnItem::Aggregate(agg) => match agg {
                AggregateFunction::Count(_) => "COUNT(*)".to_string(),
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
            },
            ReturnItem::Function(func) => match func {
                ScalarFunction::Nodes(var) => format!("nodes({})", var),
                ScalarFunction::Relationships(var) => format!("relationships({})", var),
                ScalarFunction::Length(var) => format!("length({})", var),
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
                            if let Some(node) = self.graph.get_node(*node_id) {
                                Ok(Value::NodeData {
                                    id: *node_id,
                                    label: node.label.clone(),
                                    properties: node.properties.clone(),
                                })
                            } else {
                                Ok(Value::Node(*node_id))
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            // Return edge as a simple value
                            Ok(Value::Int(*edge_id as i64))
                        }
                        BindingValue::Path { nodes, edges } => {
                            Ok(Value::Path {
                                nodes: nodes.clone(),
                                edges: edges.clone(),
                            })
                        }
                    }
                } else {
                    Ok(Value::Null)
                }
            }
            ReturnItem::Property(var, prop) => {
                if let Some(binding_value) = bindings.get(var) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph.get_node(*node_id) {
                                if let Some(value) = node.get_property(prop) {
                                    Ok(Value::from(value))
                                } else {
                                    Ok(Value::Null)
                                }
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Ok(Value::Null),
                    }
                } else {
                    Ok(Value::Null)
                }
            }
            ReturnItem::All => {
                // For *, we return the first bound node variable
                for (_var, binding_value) in bindings {
                    if let BindingValue::Node(node_id) = binding_value {
                        if let Some(node) = self.graph.get_node(*node_id) {
                            return Ok(Value::NodeData {
                                id: *node_id,
                                label: node.label.clone(),
                                properties: node.properties.clone(),
                            });
                        }
                    }
                }
                Ok(Value::Null)
            }
            ReturnItem::Aggregate(_) => {
                // Aggregates are handled separately
                Ok(Value::Null)
            }
            ReturnItem::Function(func) => {
                self.evaluate_scalar_function(func, bindings)
            }
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
                                if let Some(node) = self.graph.get_node(node_id) {
                                    Value::NodeData {
                                        id: node_id,
                                        label: node.label.clone(),
                                        properties: node.properties.clone(),
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
        }
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

        let mut row_values = Vec::new();

        for item in &return_clause.items {
            let value = self.evaluate_aggregate(item, bindings_list)?;
            row_values.push(value);
        }

        let rows = vec![Row {
            columns: row_values,
        }];
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
                        Ok(Value::Int(bindings_list.len() as i64))
                    } else {
                        // COUNT(expr) - count non-null values
                        let inner = inner.as_ref().unwrap();
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
            },
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
                }
            }
            Expression::Property(var, prop) => {
                let binding_value = bindings
                    .get(var)
                    .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;

                let node_id = binding_value
                    .as_node()
                    .ok_or_else(|| ExecuteError::TypeError("expected node".to_string()))?;

                let node = self
                    .graph
                    .get_node(node_id)
                    .ok_or_else(|| ExecuteError::TypeError("node not found".to_string()))?;

                Ok(node
                    .get_property(prop)
                    .map(Value::from)
                    .unwrap_or(Value::Null))
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
            BinaryOp::Add => self.arithmetic_op(left, right, |a, b| a + b, |a, b| a + b),
            BinaryOp::Sub => self.arithmetic_op(left, right, |a, b| a - b, |a, b| a - b),
            BinaryOp::Mul => self.arithmetic_op(left, right, |a, b| a * b, |a, b| a * b),
            BinaryOp::Div => self.arithmetic_op(left, right, |a, b| a / b, |a, b| a / b),
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
        }
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

    #[test]
    fn test_create_node() {
        let mut graph = Graph::new();
        let result = execute(&mut graph, "CREATE (n:Person)").unwrap();

        assert_eq!(result.row_count(), 1);
        assert_eq!(graph.node_count(), 1);

        let node = graph.nodes().next().unwrap();
        assert_eq!(node.label, "Person");
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
        let alice_id = graph
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

    // ========== ORDER BY tests ==========

    #[test]
    fn test_order_by_asc() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 35})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[2].columns[0], Value::String("Charlie".to_string()));
    }

    #[test]
    fn test_order_by_desc() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 35})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        assert_eq!(result.rows[0].columns[0], Value::String("Charlie".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Alice".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
        assert_eq!(result.rows[2].columns[0], Value::String("Charlie".to_string()));
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
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 35})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age SKIP 1",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Charlie".to_string()));
    }

    #[test]
    fn test_skip_and_limit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 35})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "David", age: 40})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age SKIP 1 LIMIT 2",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Charlie".to_string()));
    }

    // ========== DISTINCT tests ==========

    #[test]
    fn test_distinct() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", city: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", city: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", city: "Osaka"})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN DISTINCT n.city",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_distinct_with_order_by() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", city: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", city: "Tokyo"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", city: "Osaka"})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN DISTINCT n.city ORDER BY n.city",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("Osaka".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Tokyo".to_string()));
    }

    // ========== NULLS FIRST/LAST tests ==========

    #[test]
    fn test_nulls_last_default_asc() {
        let mut graph = Graph::new();
        // Create nodes with and without age property
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 25})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age ASC",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // ASC default: NULLS LAST
        assert_eq!(result.rows[0].columns[0], Value::String("Charlie".to_string())); // age 25
        assert_eq!(result.rows[1].columns[0], Value::String("Alice".to_string()));   // age 30
        assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string()));     // NULL
        assert_eq!(result.rows[2].columns[1], Value::Null);
    }

    #[test]
    fn test_nulls_first_default_desc() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 25})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // DESC default: NULLS FIRST
        assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string()));     // NULL
        assert_eq!(result.rows[1].columns[0], Value::String("Alice".to_string()));   // age 30
        assert_eq!(result.rows[2].columns[0], Value::String("Charlie".to_string())); // age 25
    }

    #[test]
    fn test_nulls_first_explicit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 25})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age ASC NULLS FIRST",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // NULLS FIRST explicitly
        assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string()));     // NULL
        assert_eq!(result.rows[1].columns[0], Value::String("Charlie".to_string())); // age 25
        assert_eq!(result.rows[2].columns[0], Value::String("Alice".to_string()));   // age 30
    }

    #[test]
    fn test_nulls_last_explicit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 25})"#).unwrap();

        let result = execute(
            &mut graph,
            "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC NULLS LAST",
        )
        .unwrap();

        assert_eq!(result.row_count(), 3);
        // NULLS LAST explicitly with DESC
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));   // age 30
        assert_eq!(result.rows[1].columns[0], Value::String("Charlie".to_string())); // age 25
        assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string()));     // NULL
    }

    // ========== TopN optimization tests ==========

    #[test]
    fn test_topn_optimization() {
        let mut graph = Graph::new();
        // Create 10 nodes
        for i in 1..=10 {
            execute(
                &mut graph,
                &format!(r#"CREATE (n:Person {{name: "Person{}", age: {}}})"#, i, i * 10),
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
        assert_eq!(result.rows[1].columns[1], Value::Int(90));  // age 90
        assert_eq!(result.rows[2].columns[1], Value::Int(80));  // age 80
    }

    #[test]
    fn test_topn_with_skip() {
        let mut graph = Graph::new();
        // Create 10 nodes
        for i in 1..=10 {
            execute(
                &mut graph,
                &format!(r#"CREATE (n:Person {{name: "Person{}", age: {}}})"#, i, i * 10),
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
}
