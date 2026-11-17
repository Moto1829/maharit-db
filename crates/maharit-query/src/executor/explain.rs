//! EXPLAIN / PROFILE の実行。

use super::*;

impl<'a> Executor<'a> {
    // ========== EXPLAIN / PROFILE ==========

    /// Graph statistics for EXPLAIN / PROFILE: counts per label and the
    /// property indexes that exist, so the plan shows index use (bug/167).
    pub(super) fn plan_stats(&self) -> crate::planner::GraphStats {
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

    pub(super) fn execute_explain(&self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
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

    pub(super) fn execute_profile(&mut self, stmt: Statement) -> Result<ResultSet, ExecuteError> {
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

    pub(super) fn value_to_property(&self, value: &Value) -> Result<PropertyValue, ExecuteError> {
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
}
