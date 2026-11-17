//! CREATE
//!  DELETE
//!  SET
//!  MERGE
//!  REMOVE
//!  UNWIND
//!  FOREACH の実行と、
//! 書き込み句を連ねた文（パイプライン）の処理。

use super::*;

impl<'a> Executor<'a> {
    // ========== CREATE ==========

    pub(super) fn execute_create(
        &mut self,
        create: CreateClause,
    ) -> Result<ResultSet, ExecuteError> {
        let (created_nodes, created_edges, _) =
            self.execute_create_with_bindings(&create, &Bindings::new())?;
        let columns = vec!["created_nodes".to_string(), "created_edges".to_string()];
        let rows = vec![Row {
            columns: vec![Value::Int(created_nodes), Value::Int(created_edges)],
        }];
        Ok(ResultSet::new(columns, rows))
    }

    pub(super) fn create_node(
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

    pub(super) fn execute_delete(&mut self, d: DeleteStatement) -> Result<ResultSet, ExecuteError> {
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

    pub(super) fn execute_match_create(
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

    pub(super) fn execute_create_with_bindings(
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

    pub(super) fn execute_match_set(
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
    pub(super) fn evaluate_property_map(
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
    pub(super) fn remove_property(
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

    pub(super) fn set_properties_from_map(
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

    pub(super) fn apply_set_clause(
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

    pub(super) fn execute_merge(
        &mut self,
        merge: MergeStatement,
    ) -> Result<ResultSet, ExecuteError> {
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
    pub(super) fn apply_merge(
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
    pub(super) fn execute_pipeline(
        &mut self,
        p: &PipelineStatement,
    ) -> Result<ResultSet, ExecuteError> {
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

    pub(super) fn create_pattern(
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

    pub(super) fn execute_match_remove(
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

    pub(super) fn execute_unwind(
        &mut self,
        uw: UnwindStatement,
    ) -> Result<ResultSet, ExecuteError> {
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

    pub(super) fn execute_foreach_stmt_ref(
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

    pub(super) fn execute_foreach_clause(
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

    pub(super) fn apply_update_clauses(
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

    pub(super) fn apply_remove_clause(
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

    pub(super) fn apply_delete_clause(
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

    pub(super) fn execute_match_foreach(
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
}
