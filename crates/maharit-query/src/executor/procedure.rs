//! `CALL proc.name(...) YIELD ...` のプロシージャ呼び出し（全文検索・db.labels など）。
//! グラフアルゴリズムのプロシージャは algo.rs。

use super::*;

impl<'a> Executor<'a> {
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
    pub(super) fn execute_procedure_call(
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
    pub(super) fn fulltext_search_procedure(
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
    pub(super) fn labels_procedure(
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
}
