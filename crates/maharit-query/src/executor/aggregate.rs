//! 集約（count / sum / collect / percentile など）を含む RETURN の結果表の組み立て。

use super::*;

impl<'a> Executor<'a> {
    /// ノードのシーケンスから連続するノード間のエッジIDを抽出する
    pub(super) fn extract_edge_ids(&self, nodes: &[NodeId]) -> Vec<u64> {
        let mut edges = Vec::new();
        for window in nodes.windows(2) {
            let from = window[0];
            let to = window[1];
            // Find edge from 'from' to 'to'
            let mut out = Vec::new();
            self.graph_ref().adjacent_edges(from, true, &[], &mut out);
            if let Some(edge) = out.iter().find(|e| e.to == to) {
                edges.push(edge.id);
            }
        }
        edges
    }

    pub(super) fn build_aggregated_result_set(
        &self,
        return_clause: &ReturnClause,
        bindings_list: &[Bindings],
    ) -> Result<ResultSet, ExecuteError> {
        let columns: Vec<String> = return_clause
            .items
            .iter()
            .map(|item| self.return_item_to_column_name(item))
            .collect();

        // Identify group key positions (non-aggregate items)
        let group_key_indices: Vec<usize> = return_clause
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !Self::is_aggregate(item))
            .map(|(i, _)| i)
            .collect();

        if group_key_indices.is_empty() {
            // Simple aggregation: no GROUP BY keys → single row
            let mut row_values = Vec::new();
            for item in &return_clause.items {
                let value = self.evaluate_aggregate(item, bindings_list)?;
                row_values.push(value);
            }
            return Ok(ResultSet::new(
                columns,
                vec![Row {
                    columns: row_values,
                }],
            ));
        }

        // GROUP BY: group binding indices by serialized key values
        // Use insertion-order-preserving Vec + HashMap for deterministic output
        let mut group_order: Vec<Vec<String>> = Vec::new();
        let mut group_map: HashMap<Vec<String>, Vec<usize>> = HashMap::new();

        for (idx, bindings) in bindings_list.iter().enumerate() {
            let key: Vec<String> = group_key_indices
                .iter()
                .map(|&col_idx| {
                    self.evaluate_return_item(&return_clause.items[col_idx], bindings)
                        .map(|v| format!("{}", v))
                        .unwrap_or_default()
                })
                .collect();

            if !group_map.contains_key(&key) {
                group_order.push(key.clone());
            }
            group_map.entry(key).or_default().push(idx);
        }

        // Build one result row per group
        let mut rows = Vec::with_capacity(group_order.len());

        for key_strs in &group_order {
            let indices = &group_map[key_strs];

            // Collect this group's bindings
            let group_bindings: Vec<Bindings> =
                indices.iter().map(|&i| bindings_list[i].clone()).collect();

            let mut row_values = vec![Value::Null; return_clause.items.len()];

            // Fill group key columns using the first binding of the group
            if let Some(&first_idx) = indices.first() {
                for &col_idx in &group_key_indices {
                    row_values[col_idx] = self.evaluate_return_item(
                        &return_clause.items[col_idx],
                        &bindings_list[first_idx],
                    )?;
                }
            }

            // Fill aggregate columns
            for (col_idx, item) in return_clause.items.iter().enumerate() {
                if Self::is_aggregate(item) {
                    row_values[col_idx] = self.evaluate_aggregate(item, &group_bindings)?;
                }
            }

            rows.push(Row {
                columns: row_values,
            });
        }

        // Apply ORDER BY / SKIP / LIMIT on the grouped result
        if let Some(ref order_by) = return_clause.order_by {
            let resolved_skip = return_clause
                .skip
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;
            let resolved_limit = return_clause
                .limit
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;

            let needed = match (resolved_skip, resolved_limit) {
                (Some(skip), Some(limit)) => Some((skip + limit) as usize),
                (None, Some(limit)) => Some(limit as usize),
                _ => None,
            };

            // After grouping there is no single source row per result row,
            // so keys must refer to returned columns.
            let slots = self.order_key_slots(order_by, &columns);
            let keys = rows
                .iter()
                .map(|row| self.order_keys(order_by, &slots, row, None))
                .collect::<Result<Vec<_>, _>>()?;
            rows = self.sort_rows_by_keys(rows, keys, order_by, needed);

            if let Some(skip) = resolved_skip {
                let skip = skip as usize;
                if skip < rows.len() {
                    rows = rows.into_iter().skip(skip).collect();
                } else {
                    rows.clear();
                }
            }
            if let Some(limit) = resolved_limit {
                rows.truncate(limit as usize);
            }
        } else {
            let resolved_skip = return_clause
                .skip
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;
            let resolved_limit = return_clause
                .limit
                .as_ref()
                .map(|e| self.resolve_skip_limit(e))
                .transpose()?;

            if let Some(skip) = resolved_skip {
                let skip = skip as usize;
                if skip < rows.len() {
                    rows = rows.into_iter().skip(skip).collect();
                } else {
                    rows.clear();
                }
            }
            if let Some(limit) = resolved_limit {
                rows.truncate(limit as usize);
            }
        }

        Ok(ResultSet::new(columns, rows))
    }

    pub(super) fn evaluate_aggregate(
        &self,
        item: &ReturnItem,
        bindings_list: &[Bindings],
    ) -> Result<Value, ExecuteError> {
        match item {
            ReturnItem::Aggregate(agg) => match agg {
                AggregateFunction::Count(inner) => {
                    let Some(inner) = inner.as_ref() else {
                        // COUNT(*)
                        return Ok(Value::Int(bindings_list.len() as i64));
                    };
                    // Fast path: bound Node/Edge variables are always non-null in MATCH results
                    if let ReturnItem::Variable(var) = inner.as_ref() {
                        let is_always_bound = bindings_list
                            .first()
                            .and_then(|b| b.get(var.as_str()))
                            .map(|bv| matches!(bv, BindingValue::Node(_) | BindingValue::Edge(_)))
                            .unwrap_or(false);
                        if is_always_bound {
                            return Ok(Value::Int(bindings_list.len() as i64));
                        }
                    }
                    // General path: count non-null values
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
                    let collected: Vec<Value> = bindings_list
                        .iter()
                        .filter_map(|bindings| {
                            self.evaluate_return_item(inner, bindings)
                                .ok()
                                .filter(|v| !matches!(v, Value::Null))
                        })
                        .collect();
                    Ok(Value::List(collected))
                }
                AggregateFunction::PercentileCont(inner, percentile_item) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    if values.is_empty() {
                        return Ok(Value::Null);
                    }
                    let p = match self.evaluate_return_item(
                        percentile_item,
                        bindings_list.first().unwrap_or(&Bindings::new()),
                    )? {
                        Value::Float(f) => f,
                        Value::Int(n) => n as f64,
                        _ => {
                            return Err(ExecuteError::TypeError(
                                "percentileCont() percentile must be a number between 0 and 1"
                                    .to_string(),
                            ));
                        }
                    };
                    if !(0.0..=1.0).contains(&p) {
                        return Err(ExecuteError::TypeError(
                            "percentileCont() percentile must be between 0 and 1".to_string(),
                        ));
                    }
                    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let idx = p * (values.len() - 1) as f64;
                    let lower = idx.floor() as usize;
                    let upper = idx.ceil() as usize;
                    if lower == upper || upper >= values.len() {
                        Ok(Value::Float(values[lower]))
                    } else {
                        let frac = idx - lower as f64;
                        Ok(Value::Float(
                            values[lower] + (values[upper] - values[lower]) * frac,
                        ))
                    }
                }
                AggregateFunction::PercentileDisc(inner, percentile_item) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    if values.is_empty() {
                        return Ok(Value::Null);
                    }
                    let p = match self.evaluate_return_item(
                        percentile_item,
                        bindings_list.first().unwrap_or(&Bindings::new()),
                    )? {
                        Value::Float(f) => f,
                        Value::Int(n) => n as f64,
                        _ => {
                            return Err(ExecuteError::TypeError(
                                "percentileDisc() percentile must be a number between 0 and 1"
                                    .to_string(),
                            ));
                        }
                    };
                    if !(0.0..=1.0).contains(&p) {
                        return Err(ExecuteError::TypeError(
                            "percentileDisc() percentile must be between 0 and 1".to_string(),
                        ));
                    }
                    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let idx = if p == 0.0 {
                        0
                    } else {
                        ((p * values.len() as f64).ceil() as usize)
                            .saturating_sub(1)
                            .min(values.len() - 1)
                    };
                    let val = values[idx];
                    if val.fract() == 0.0 && val.abs() < i64::MAX as f64 {
                        Ok(Value::Int(val as i64))
                    } else {
                        Ok(Value::Float(val))
                    }
                }
                AggregateFunction::StDev(inner) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    let n = values.len();
                    if n < 2 {
                        return Ok(Value::Float(0.0));
                    }
                    let mean = values.iter().sum::<f64>() / n as f64;
                    let variance =
                        values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
                    Ok(Value::Float(variance.sqrt()))
                }
                AggregateFunction::StDevP(inner) => {
                    let mut values: Vec<f64> = Vec::new();
                    for bindings in bindings_list {
                        match self.evaluate_return_item(inner, bindings)? {
                            Value::Int(n) => values.push(n as f64),
                            Value::Float(n) => values.push(n),
                            _ => {}
                        }
                    }
                    let n = values.len();
                    if n == 0 {
                        return Ok(Value::Float(0.0));
                    }
                    let mean = values.iter().sum::<f64>() / n as f64;
                    let variance =
                        values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n as f64;
                    Ok(Value::Float(variance.sqrt()))
                }
                AggregateFunction::CountDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut count = 0i64;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            count += 1;
                        }
                    }
                    Ok(Value::Int(count))
                }
                AggregateFunction::SumDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut sum = 0.0f64;
                    let mut has_float = false;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            match val {
                                Value::Int(n) => sum += n as f64,
                                Value::Float(n) => {
                                    sum += n;
                                    has_float = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    if has_float {
                        Ok(Value::Float(sum))
                    } else {
                        Ok(Value::Int(sum as i64))
                    }
                }
                AggregateFunction::AvgDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut sum = 0.0f64;
                    let mut count = 0usize;
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            match val {
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
                    }
                    if count == 0 {
                        Ok(Value::Null)
                    } else {
                        Ok(Value::Float(sum / count as f64))
                    }
                }
                AggregateFunction::CollectDistinct(inner) => {
                    let mut seen = std::collections::HashSet::<String>::new();
                    let mut result = Vec::new();
                    for bindings in bindings_list {
                        let val = self.evaluate_return_item(inner, bindings)?;
                        if matches!(val, Value::Null) {
                            continue;
                        }
                        let key = format!("{}", val);
                        if seen.insert(key) {
                            result.push(val);
                        }
                    }
                    Ok(Value::List(result))
                }
            },
            // Alias wrapping an aggregate: delegate to the inner aggregate
            ReturnItem::Alias(inner, _) => self.evaluate_aggregate(inner, bindings_list),
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

    /// SKIP / LIMIT の式を評価して非負整数に変換するヘルパー。
    /// 整数リテラルまたはパラメータ参照に対応する。
    pub(super) fn resolve_skip_limit(&self, expr: &Expression) -> Result<u64, ExecuteError> {
        let val = self.evaluate_expression(expr, &Bindings::new())?;
        match val {
            Value::Int(n) if n >= 0 => Ok(n as u64),
            Value::Int(n) => Err(ExecuteError::TypeError(format!(
                "SKIP/LIMIT value must be a non-negative integer, got {}",
                n
            ))),
            other => Err(ExecuteError::TypeError(format!(
                "SKIP/LIMIT value must be an integer, got {:?}",
                other
            ))),
        }
    }
}
