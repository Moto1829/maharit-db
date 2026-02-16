use std::collections::{HashMap, HashSet};

use maharit_core::{
    traversal, Constraint, ConstraintError, ConstraintManager, ConstraintType, Edge,
    FulltextError, FulltextManager, Graph, NodeId, PropertyType, PropertyValue,
};
use regex::Regex;
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

    #[error("constraint error: {0}")]
    ConstraintError(#[from] ConstraintError),

    #[error("fulltext error: {0}")]
    FulltextError(#[from] FulltextError),
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
    graph: &'a mut Graph,
    constraints: ConstraintManager,
    fulltext: FulltextManager,
}

impl<'a> Executor<'a> {
    pub fn new(graph: &'a mut Graph) -> Self {
        Self {
            graph,
            constraints: ConstraintManager::new(),
            fulltext: FulltextManager::new(),
        }
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
            Statement::CreateConstraint(cc) => self.execute_create_constraint(cc),
            Statement::DropConstraint(dc) => self.execute_drop_constraint(dc),
            Statement::ShowConstraints => self.execute_show_constraints(),
            Statement::CreateFulltextIndex(cfi) => self.execute_create_fulltext_index(cfi),
            Statement::DropFulltextIndex(dfi) => self.execute_drop_fulltext_index(dfi),
            Statement::CreateUser(cu) => Ok(ResultSet::new(
                vec!["result".to_string()],
                vec![Row {
                    columns: vec![Value::String(format!("User '{}' created with role '{}'", cu.username, cu.role))],
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
                    columns: vec![Value::String("SHOW USERS requires server context".to_string())],
                }],
            )),
            Statement::Explain(inner) => self.execute_explain(*inner),
            Statement::Profile(inner) => self.execute_profile(*inner),
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

        // Validate constraints before creating
        let props: HashMap<String, PropertyValue> = pattern
            .properties
            .iter()
            .map(|(k, v)| (k.clone(), PropertyValue::from(v.clone())))
            .collect();
        self.constraints
            .validate_node_create(self.graph, &label, &props, None)?;

        let node_id = self.graph.create_node(label.clone());

        // Set properties
        if let Some(node) = self.graph.get_node_mut(node_id) {
            for (key, value) in &pattern.properties {
                node.set_property(key.clone(), PropertyValue::from(value.clone()));
            }
        }

        // Index in fulltext indexes
        self.fulltext.index_node(node_id, &label, &props);

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
            all_bindings = all_bindings
                .into_iter()
                .filter(|b| {
                    self.evaluate_expression(where_expr, b)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                })
                .collect();
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
                    if let Some(var) = &node_pattern.variable {
                        if bindings.contains_key(var) {
                            continue;
                        }
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
                        let edge_id = self.graph.create_edge(from, to, edge_label)?;

                        // Set edge properties
                        if let Some(edge) = self.graph.get_edge_mut(edge_id) {
                            for (key, value) in &segment.edge.properties {
                                edge.set_property(key.clone(), PropertyValue::from(value.clone()));
                            }
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

    fn execute_match_set(
        &mut self,
        ms: MatchSetStatement,
    ) -> Result<ResultSet, ExecuteError> {
        // Execute MATCH segments
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &ms.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;
        }

        // Apply WHERE filter
        if let Some(where_expr) = &ms.where_clause {
            all_bindings = all_bindings
                .into_iter()
                .filter(|b| {
                    self.evaluate_expression(where_expr, b)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                })
                .collect();
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
                let binding_value = bindings
                    .get(&item.variable)
                    .ok_or_else(|| ExecuteError::UndefinedVariable(item.variable.clone()))?;

                let value = self.evaluate_expression(&item.value, bindings)?;
                let prop_value = self.value_to_property(&value)?;

                match binding_value {
                    BindingValue::Node(node_id) => {
                        // Validate constraint before setting
                        if let Some(node) = self.graph.get_node(*node_id) {
                            self.constraints.validate_property_set(
                                self.graph,
                                node,
                                &item.property,
                                &prop_value,
                            )?;
                        }
                        if let Some(node) = self.graph.get_node_mut(*node_id) {
                            node.set_property(&item.property, prop_value);
                        }
                    }
                    BindingValue::Edge(edge_id) => {
                        if let Some(edge) = self.graph.get_edge_mut(*edge_id) {
                            edge.set_property(&item.property, prop_value);
                        }
                    }
                    _ => {
                        return Err(ExecuteError::TypeError(
                            "SET requires node or edge binding".to_string(),
                        ));
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
                all_bindings = all_bindings
                    .into_iter()
                    .filter(|b| {
                        self.evaluate_expression(where_expr, b)
                            .map(|v| matches!(v, Value::Bool(true)))
                            .unwrap_or(false)
                    })
                    .collect();
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
                if let Some(var) = &node_pattern.variable {
                    if bindings.contains_key(var) {
                        return Ok(());
                    }
                }
                self.create_node(node_pattern, bindings)?;
                Ok(())
            }
            Pattern::Path(path_pattern) => {
                let start_id = if let Some(var) = &path_pattern.start.variable {
                    if let Some(bound) = bindings.get(var) {
                        bound.as_node().ok_or_else(|| {
                            ExecuteError::TypeError("expected node".to_string())
                        })?
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
                    let edge_id = self.graph.create_edge(from, to, edge_label)?;

                    if let Some(edge) = self.graph.get_edge_mut(edge_id) {
                        for (key, value) in &segment.edge.properties {
                            edge.set_property(key.clone(), PropertyValue::from(value.clone()));
                        }
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
            all_bindings = all_bindings
                .into_iter()
                .filter(|b| {
                    self.evaluate_expression(where_expr, b)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                })
                .collect();
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
                                if let Some(node) = self.graph.get_node(*node_id) {
                                    self.constraints
                                        .validate_property_remove(node, prop)?;
                                }
                                if let Some(node) = self.graph.get_node_mut(*node_id) {
                                    node.remove_property(prop);
                                }
                            }
                            BindingValue::Edge(edge_id) => {
                                if let Some(edge) = self.graph.get_edge_mut(*edge_id) {
                                    edge.remove_property(prop);
                                }
                            }
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "REMOVE requires node or edge binding".to_string(),
                                ));
                            }
                        }
                    }
                    RemoveItem::Label(var, _label) => {
                        let _binding_value = bindings
                            .get(var)
                            .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;
                        // Label removal: set label to empty string
                        // (Full label management would require multi-label support)
                        if let Some(node_id) = bindings.get(var).and_then(|v| v.as_node()) {
                            if let Some(node) = self.graph.get_node_mut(node_id) {
                                node.label = String::new();
                            }
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
                columns: vec![Value::String(format!(
                    "Constraint '{}' created",
                    cc.name
                ))],
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
                columns: vec![Value::String(format!(
                    "Constraint '{}' dropped",
                    dc.name
                ))],
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
            .graph
            .nodes()
            .filter(|n| n.label == cfi.label)
            .map(|n| n.id)
            .collect();

        for node_id in node_ids {
            if let Some(node) = self.graph.get_node(node_id) {
                let props = node.properties.clone();
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

    // ========== EXPLAIN / PROFILE ==========

    fn execute_explain(&self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
        let node_count = self.graph.node_count() as u64;
        let edge_count = self.graph.edge_count() as u64;
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
        let node_count = self.graph.node_count() as u64;
        let edge_count = self.graph.edge_count() as u64;
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
            _ => Err(ExecuteError::TypeError(
                "cannot convert to property value".to_string(),
            )),
        }
    }

    // ========== MATCH ==========

    fn execute_match(&mut self, m: MatchStatement) -> Result<ResultSet, ExecuteError> {
        // Process each segment
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &m.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;
        }

        // Build result set
        self.build_result_set(&m.return_clause, &all_bindings)
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
            bindings = bindings
                .into_iter()
                .filter(|b| {
                    self.evaluate_expression(where_expr, b)
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                })
                .collect();
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
        // Project bindings through WITH items
        let mut result: Vec<Bindings> = Vec::new();

        for binding in &bindings {
            let mut new_binding = Bindings::new();

            for item in &with_clause.items {
                let value = self.evaluate_return_item(&item.expression, binding)?;

                // Determine the variable name for this item
                let var_name = if let Some(ref alias) = item.alias {
                    alias.clone()
                } else {
                    // Use the original variable name if available
                    match &item.expression {
                        ReturnItem::Variable(v) => v.clone(),
                        ReturnItem::Property(v, p) => format!("{}.{}", v, p),
                        ReturnItem::Aggregate(agg) => self.aggregate_to_name(agg),
                        ReturnItem::Function(func) => self.function_to_name(func),
                        ReturnItem::All => "*".to_string(),
                    }
                };

                // Convert Value back to BindingValue for the new binding
                match value {
                    Value::Node(id) | Value::NodeData { id, .. } => {
                        new_binding.insert(var_name, BindingValue::Node(id));
                    }
                    Value::Path { nodes, edges } => {
                        new_binding.insert(var_name, BindingValue::Path { nodes, edges });
                    }
                    other => {
                        new_binding.insert(var_name, BindingValue::Scalar(other));
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
        if let Some(skip) = with_clause.skip {
            result = result.into_iter().skip(skip as usize).collect();
        }

        // Apply LIMIT
        if let Some(limit) = with_clause.limit {
            result = result.into_iter().take(limit as usize).collect();
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
        }
    }

    fn function_to_name(&self, func: &ScalarFunction) -> String {
        match func {
            ScalarFunction::Nodes(_) => "nodes".to_string(),
            ScalarFunction::Relationships(_) => "relationships".to_string(),
            ScalarFunction::Length(_) => "length".to_string(),
            ScalarFunction::ShortestPath { .. } => "shortestPath".to_string(),
            ScalarFunction::AllShortestPaths { .. } => "allShortestPaths".to_string(),
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
            // Regular MATCH: filter out non-matches
            let mut matches = current_bindings;
            for pattern in &clause.patterns {
                matches = self.match_pattern(pattern, matches)?;
            }
            Ok(matches)
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
                ScalarFunction::ShortestPath { start, end } => {
                    format!("shortestPath({}, {})", start, end)
                }
                ScalarFunction::AllShortestPaths { start, end } => {
                    format!("allShortestPaths({}, {})", start, end)
                }
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
            ScalarFunction::ShortestPath { start, end } => {
                let start_id = bindings
                    .get(start)
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(start.clone()))?;
                let end_id = bindings
                    .get(end)
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(end.clone()))?;

                if let Some(path) = traversal::shortest_path(self.graph, start_id, end_id) {
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

                let paths = traversal::all_shortest_paths(self.graph, start_id, end_id);
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
        }
    }

    /// ノードのシーケンスから連続するノード間のエッジIDを抽出する
    fn extract_edge_ids(&self, nodes: &[NodeId]) -> Vec<u64> {
        let mut edges = Vec::new();
        for window in nodes.windows(2) {
            let from = window[0];
            let to = window[1];
            // Find edge from 'from' to 'to'
            for edge in self.graph.get_outgoing_edges(from) {
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
                    BindingValue::Scalar(value) => Ok(value.clone()),
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
            Expression::Case(case_expr) => self.evaluate_case_expression(case_expr, bindings),
            Expression::List(elements) => {
                let values: Vec<Value> = elements
                    .iter()
                    .map(|e| self.evaluate_expression(e, bindings))
                    .collect::<Result<_, _>>()?;
                Ok(Value::List(values))
            }
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
            BinaryOp::Add => self.arithmetic_op(left, right, |a, b| a + b, |a, b| a + b),
            BinaryOp::Sub => self.arithmetic_op(left, right, |a, b| a - b, |a, b| a - b),
            BinaryOp::Mul => self.arithmetic_op(left, right, |a, b| a * b, |a, b| a * b),
            BinaryOp::Div => self.arithmetic_op(left, right, |a, b| a / b, |a, b| a / b),
            BinaryOp::Regex => match (left, right) {
                (Value::String(s), Value::String(pattern)) => {
                    // Cypher =~ is full-match, so anchor the pattern
                    let anchored = format!("(?s)\\A(?:{})\\z", pattern);
                    let re = Regex::new(&anchored).map_err(|e| {
                        ExecuteError::TypeError(format!("invalid regex: {}", e))
                    })?;
                    Ok(Value::Bool(re.is_match(s)))
                }
                _ => Err(ExecuteError::TypeError(
                    "=~ requires string operands".to_string(),
                )),
            },
            BinaryOp::Contains => match (left, right) {
                (Value::String(haystack), Value::String(needle)) => {
                    let result = haystack.to_lowercase().contains(&needle.to_lowercase());
                    Ok(Value::Bool(result))
                }
                _ => Err(ExecuteError::TypeError(
                    "CONTAINS requires string operands".to_string(),
                )),
            },
            BinaryOp::StartsWith => match (left, right) {
                (Value::String(s), Value::String(prefix)) => {
                    Ok(Value::Bool(s.to_lowercase().starts_with(&prefix.to_lowercase())))
                }
                _ => Err(ExecuteError::TypeError(
                    "STARTS WITH requires string operands".to_string(),
                )),
            },
            BinaryOp::EndsWith => match (left, right) {
                (Value::String(s), Value::String(suffix)) => {
                    Ok(Value::Bool(s.to_lowercase().ends_with(&suffix.to_lowercase())))
                }
                _ => Err(ExecuteError::TypeError(
                    "ENDS WITH requires string operands".to_string(),
                )),
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
        // Bob has no outgoing KNOWS
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
        assert_eq!(result.rows[1].columns[1], Value::Null);
        // Charlie has no outgoing KNOWS
        assert_eq!(result.rows[2].columns[0], Value::String("Charlie".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
    }

    // ========== CASE WHEN tests ==========

    #[test]
    fn test_case_when_searched_in_where() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 15})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 65})"#).unwrap();

        // Use CASE in WHERE to filter: only adults (age >= 18)
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE CASE WHEN n.age >= 18 THEN true ELSE false END RETURN n.name ORDER BY n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Charlie".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
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
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", status: 1})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", status: 2})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", status: 1})"#).unwrap();

        // Simple CASE: CASE n.status WHEN 1 THEN true ELSE false END
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE CASE n.status WHEN 1 THEN true ELSE false END RETURN n.name ORDER BY n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Charlie".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
    }

    #[test]
    fn test_with_limit() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();

        // WITH n LIMIT 2 restricts intermediate results
        let result = execute(
            &mut graph,
            "MATCH (n:Person) WITH n LIMIT 2 RETURN n.name",
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
    }

    #[test]
    fn test_with_where_filter() {
        let mut graph = Graph::new();
        execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
        execute(&mut graph, r#"CREATE (n:Person {name: "Charlie", age: 35})"#).unwrap();

        // WITH + WHERE filters on projected values
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WITH n WHERE n.age > 28 RETURN n.name ORDER BY n.name"#,
        )
        .unwrap();

        assert_eq!(result.row_count(), 2);
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
        assert_eq!(result.rows[1].columns[0], Value::String("Charlie".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
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
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
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
            .find(|n| n.label == "Car")
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

    // ========== MERGE tests ==========

    #[test]
    fn test_merge_create_new() {
        let mut graph = Graph::new();

        execute(
            &mut graph,
            r#"MERGE (n:Person {name: "Alice"})"#,
        )
        .unwrap();

        assert_eq!(graph.node_count(), 1);
        let node = graph.nodes().next().unwrap();
        assert_eq!(node.label, "Person");
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

        execute(
            &mut graph,
            r#"MERGE (n:Person {name: "Alice"})"#,
        )
        .unwrap();

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
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice", age: 30})"#,
        )
        .unwrap();

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
        assert_eq!(node.label, "");
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
        assert_eq!(rs.rows[0].columns[0], Value::String("Constraint 'unique_email' created".to_string()));
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
        assert_eq!(rs.rows[0].columns[0], Value::String("Constraint 'unique_email' dropped".to_string()));
    }

    #[test]
    fn test_drop_nonexistent_constraint() {
        let mut graph = Graph::new();
        let results = execute_with_constraints(
            &mut graph,
            &["DROP CONSTRAINT nonexistent"],
        );
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
        let results = execute_with_constraints(
            &mut graph,
            &["SHOW CONSTRAINTS"],
        );
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
        let stmt = Parser::new("CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE")
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
        let stmt = Parser::new("CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL")
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
        let stmt = Parser::new("CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER")
            .unwrap()
            .parse()
            .unwrap();
        if let Statement::CreateConstraint(cc) = stmt {
            assert_eq!(cc.constraint_type, ConstraintTypeAst::TypeCheck(PropertyTypeAst::Integer));
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
        let stmt = Parser::new("SHOW CONSTRAINTS")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(stmt, Statement::ShowConstraints);
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
        let result = execute(
            &mut graph,
            r#"EXPLAIN CREATE (n:Person {name: "Alice"})"#,
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
        execute(
            &mut graph,
            r#"EXPLAIN CREATE (m:Person {name: "Bob"})"#,
        )
        .unwrap();
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
        let result = execute(
            &mut graph,
            r#"PROFILE CREATE (n:Person {name: "Alice"})"#,
        )
        .unwrap();
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
        let result = execute(
            &mut graph,
            r#"EXPLAIN MERGE (n:Person {name: "Alice"})"#,
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

        let stmt = Parser::new(
            r#"CREATE FULLTEXT INDEX article_search FOR (n:Article) ON (n.title)"#,
        )
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
        let results = executor.fulltext_manager().get_index("article_search")
            .unwrap()
            .search("graph");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_parse_create_fulltext_index() {
        let stmt = Parser::new(
            r#"CREATE FULLTEXT INDEX my_idx FOR (n:Person) ON (n.name, n.bio)"#,
        )
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

    #[test]
    fn test_starts_with_operator() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice"})"#,
        )
        .unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name STARTS WITH "Ali" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
    }

    #[test]
    fn test_ends_with_operator() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice"})"#,
        )
        .unwrap();
        let result = execute(
            &mut graph,
            r#"MATCH (n:Person) WHERE n.name ENDS WITH "ice" RETURN n.name"#,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].columns[0], Value::String("Alice".to_string()));
    }

    #[test]
    fn test_starts_with_case_insensitive() {
        let mut graph = Graph::new();
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice"})"#,
        )
        .unwrap();
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
        execute(
            &mut graph,
            r#"CREATE (n:Person {name: "Alice"})"#,
        )
        .unwrap();
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
        execute(
            &mut graph,
            "CREATE (n:Text {value: \"\u{00e9}\"})",
        )
        .unwrap();
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
        execute(
            &mut graph,
            "CREATE (n:Text {value: \"e\u{0301}\"})",
        )
        .unwrap();
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
        execute(
            &mut graph,
            r#"CREATE (n:Person {age: 30})"#,
        )
        .unwrap();
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
}
