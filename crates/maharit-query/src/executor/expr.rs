//! 式の評価（演算子、CASE、比較、CONTAINS のフレーズ・あいまい検索）。

use super::*;

impl<'a> Executor<'a> {
    /// テンポラル値（Date / DateTime / Duration）のフィールドアクセスを行う。
    ///
    /// `d.year`, `d.month`, `d.day`, `d.hour`, `d.minute`, `d.second`,
    /// `dur.years`, `dur.months`, `dur.days`, `dur.hours`, `dur.minutes`, `dur.seconds`
    /// に対応する。一致するフィールドがない場合は `Value::Null` を返す。
    pub(super) fn access_temporal_field(val: &Value, field: &str) -> Result<Value, ExecuteError> {
        use maharit_core::temporal;
        match val {
            Value::Date(days) => {
                let (y, m, d) = temporal::days_to_ymd(*days);
                let v = match field {
                    "year" => Value::Int(y as i64),
                    "month" => Value::Int(m as i64),
                    "day" => Value::Int(d as i64),
                    _ => Value::Null,
                };
                Ok(v)
            }
            Value::DateTime(ms) => {
                let (y, mo, d, h, mi, s, _frac) = temporal::millis_to_datetime(*ms);
                let v = match field {
                    "year" => Value::Int(y as i64),
                    "month" => Value::Int(mo as i64),
                    "day" => Value::Int(d as i64),
                    "hour" => Value::Int(h as i64),
                    "minute" => Value::Int(mi as i64),
                    "second" => Value::Int(s as i64),
                    _ => Value::Null,
                };
                Ok(v)
            }
            Value::Duration {
                months,
                days,
                millis,
            } => {
                let v = match field {
                    "years" => Value::Int((*months / 12) as i64),
                    "months" => Value::Int((*months % 12) as i64),
                    "days" => Value::Int(*days as i64),
                    "hours" => Value::Int(*millis / 3_600_000),
                    "minutes" => Value::Int(*millis % 3_600_000 / 60_000),
                    "seconds" => Value::Int(*millis % 60_000 / 1_000),
                    "milliseconds" => Value::Int(*millis % 1_000),
                    _ => Value::Null,
                };
                Ok(v)
            }
            _ => Ok(Value::Null),
        }
    }

    pub(super) fn evaluate_expression(
        &self,
        expr: &Expression,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        match expr {
            Expression::Literal(lit) => Ok(Value::from(lit.clone())),
            Expression::Variable(var) => {
                let binding_value = bindings
                    .get(var.as_str())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;
                match binding_value {
                    BindingValue::Node(id) => Ok(Value::Node(*id)),
                    BindingValue::Edge(id) => Ok(Value::Edge(*id)),
                    BindingValue::Path { nodes, edges } => Ok(Value::Path {
                        nodes: nodes.clone(),
                        edges: edges.clone(),
                    }),
                    BindingValue::Scalar(value) => Ok(value.clone()),
                }
            }
            Expression::Property(var, prop) => {
                let binding_value = bindings
                    .get(var.as_str())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(var.clone()))?;

                match binding_value {
                    BindingValue::Node(node_id) => {
                        let node = self
                            .graph_ref()
                            .get_node(*node_id)
                            .ok_or_else(|| ExecuteError::TypeError("node not found".to_string()))?;
                        Ok(node
                            .get_property(prop)
                            .map(Value::from)
                            .unwrap_or(Value::Null))
                    }
                    BindingValue::Edge(edge_id) => {
                        let edge = self
                            .graph_ref()
                            .get_edge(*edge_id)
                            .ok_or_else(|| ExecuteError::TypeError("edge not found".to_string()))?;
                        Ok(edge
                            .get_property(prop)
                            .map(Value::from)
                            .unwrap_or(Value::Null))
                    }
                    BindingValue::Scalar(scalar_val) => match scalar_val {
                        Value::Map(map) => Ok(map.get(prop).cloned().unwrap_or(Value::Null)),
                        // Nodes held as values, e.g. `[n IN nodes(p) | n.name]`.
                        Value::Node(node_id) => Ok(self
                            .graph_ref()
                            .get_node(*node_id)
                            .and_then(|node| node.get_property(prop).map(Value::from))
                            .unwrap_or(Value::Null)),
                        Value::NodeData { properties, .. } => {
                            Ok(properties.get(prop).map(Value::from).unwrap_or(Value::Null))
                        }
                        // Relationships held as values, e.g. `r IN relationships(p) | r.w`
                        Value::Edge(edge_id) => Ok(self.edge_property(*edge_id, prop)),
                        _ => Self::access_temporal_field(scalar_val, prop),
                    },
                    _ => Err(ExecuteError::TypeError(format!(
                        "cannot access property '{}' on path value",
                        prop
                    ))),
                }
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
            Expression::IndexAccess(list_expr, index_expr) => {
                // `r[key]`: リレーションシップは値にすると ID になるので、変数のまま扱う
                if let Expression::Variable(var) = list_expr.as_ref()
                    && let Some(BindingValue::Edge(id)) = bindings.get(var.as_str())
                {
                    return match self.evaluate_expression(index_expr, bindings)? {
                        Value::String(key) => Ok(self
                            .graph_ref()
                            .get_edge(*id)
                            .and_then(|e| e.properties.get(&key).map(Value::from))
                            .unwrap_or(Value::Null)),
                        Value::Null => Ok(Value::Null),
                        _ => Err(ExecuteError::TypeError(
                            "a relationship property key must be a string".to_string(),
                        )),
                    };
                }
                let list_val = self.evaluate_expression(list_expr, bindings)?;
                let idx_val = self.evaluate_expression(index_expr, bindings)?;
                match (list_val, idx_val) {
                    // 動的なプロパティ参照: `n["name"]`、`n[$key]`
                    (Value::Node(id), Value::String(key)) => Ok(self
                        .graph_ref()
                        .get_node_property(id, &key)
                        .map(|v| Value::from(&v))
                        .unwrap_or(Value::Null)),
                    (Value::NodeData { properties, .. }, Value::String(key)) => {
                        Ok(properties.get(&key).map(Value::from).unwrap_or(Value::Null))
                    }
                    (Value::Edge(id), Value::String(key)) => Ok(self.edge_property(id, &key)),
                    (Value::List(items), Value::Int(i)) => {
                        let idx = if i < 0 { items.len() as i64 + i } else { i };
                        if idx >= 0 {
                            Ok(items.get(idx as usize).cloned().unwrap_or(Value::Null))
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    // マップのキー参照: `$props['name']` / `$props.name`
                    (Value::Map(map), Value::String(key)) => {
                        Ok(map.get(&key).cloned().unwrap_or(Value::Null))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            Expression::ListSlice(list_expr, start_expr, end_expr) => {
                let list_val = self.evaluate_expression(list_expr, bindings)?;
                let start_val = self.evaluate_expression(start_expr, bindings)?;
                let end_val = self.evaluate_expression(end_expr, bindings)?;
                match (list_val, start_val, end_val) {
                    // `list[start..]`: 終わりが null なら末尾まで
                    (Value::List(items), Value::Int(s), Value::Null) => {
                        let len = items.len() as i64;
                        let s = if s < 0 { (len + s).max(0) } else { s.min(len) } as usize;
                        Ok(Value::List(items[s..].to_vec()))
                    }
                    (Value::List(items), Value::Int(s), Value::Int(e)) => {
                        let len = items.len() as i64;
                        let s = if s < 0 { (len + s).max(0) } else { s.min(len) } as usize;
                        let e = if e < 0 { (len + e).max(0) } else { e.min(len) } as usize;
                        Ok(Value::List(items[s.min(e)..e.max(s)].to_vec()))
                    }
                    (Value::Null, _, _) => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            Expression::ListComprehension {
                variable,
                list,
                predicate,
                result,
            } => {
                let list_val = self.evaluate_expression(list, bindings)?;
                let items = match list_val {
                    Value::List(items) => items,
                    Value::Null => return Ok(Value::Null),
                    _ => {
                        return Err(ExecuteError::TypeError(
                            "list comprehension requires a list".to_string(),
                        ));
                    }
                };
                let mut output = Vec::new();
                for item in items {
                    let mut local_bindings = bindings.clone();
                    local_bindings.insert(Arc::from(variable.as_str()), BindingValue::Scalar(item));
                    // Apply predicate filter
                    if let Some(pred) = predicate {
                        let pred_val = self.evaluate_expression(pred, &local_bindings)?;
                        if !matches!(pred_val, Value::Bool(true)) {
                            continue;
                        }
                    }
                    // Evaluate result expression
                    let result_val = self.evaluate_expression(result, &local_bindings)?;
                    output.push(result_val);
                }
                Ok(Value::List(output))
            }
            Expression::HasLabels(var, labels) => match bindings.get(var.as_str()) {
                Some(BindingValue::Node(id)) => Ok(Value::Bool(
                    self.graph_ref().node_has_all_labels(*id, labels),
                )),
                // `r:KNOWS`: リレーションシップは型が一致するか
                Some(BindingValue::Edge(id)) => Ok(Value::Bool(
                    labels.len() == 1
                        && self
                            .graph_ref()
                            .get_edge(*id)
                            .is_some_and(|e| e.label == labels[0]),
                )),
                Some(BindingValue::Scalar(Value::Null)) => Ok(Value::Null),
                Some(_) => Err(ExecuteError::TypeError(format!(
                    "{var}:{} requires a node or relationship",
                    labels.join(":")
                ))),
                None => Err(ExecuteError::UndefinedVariable(var.clone())),
            },
            Expression::Parameter(name) => self
                .params
                .get(name)
                .cloned()
                .ok_or_else(|| ExecuteError::UndefinedVariable(format!("${}", name))),
            Expression::ExistsSubquery(subquery) => {
                // Evaluate patterns starting from current bindings
                let mut matches = vec![bindings.clone()];
                for pattern in &subquery.patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                // Apply WHERE filter
                if let Some(ref where_expr) = subquery.where_clause {
                    matches.retain(|b| {
                        self.evaluate_expression(where_expr, b)
                            .map(|v| matches!(v, Value::Bool(true)))
                            .unwrap_or(false)
                    });
                }
                Ok(Value::Bool(!matches.is_empty()))
            }
            Expression::CountSubquery(subquery) => {
                // Evaluate patterns starting from current bindings
                let mut matches = vec![bindings.clone()];
                for pattern in &subquery.patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                // Apply WHERE filter
                if let Some(ref where_expr) = subquery.where_clause {
                    matches.retain(|b| {
                        self.evaluate_expression(where_expr, b)
                            .map(|v| matches!(v, Value::Bool(true)))
                            .unwrap_or(false)
                    });
                }
                Ok(Value::Int(matches.len() as i64))
            }
            Expression::CollectSubquery(body) => {
                // Evaluate patterns starting from current bindings
                let mut matches = vec![bindings.clone()];
                for pattern in &body.patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                // Apply WHERE filter
                if let Some(ref where_expr) = body.where_clause {
                    matches.retain(|b| {
                        self.evaluate_expression(where_expr, b)
                            .map(|v| matches!(v, Value::Bool(true)))
                            .unwrap_or(false)
                    });
                }
                // Evaluate return item for each match
                let values: Vec<Value> = matches
                    .iter()
                    .map(|b| self.evaluate_return_item(&body.return_item, b))
                    .collect::<Result<_, _>>()?;
                Ok(Value::List(values))
            }
            Expression::ListPredicate {
                kind,
                variable,
                list,
                predicate,
            } => {
                let list_val = self.evaluate_expression(list, bindings)?;
                let items = match list_val {
                    Value::List(items) => items,
                    Value::Null => return Ok(Value::Null),
                    _ => {
                        return Err(ExecuteError::TypeError(
                            "list predicate requires a list".to_string(),
                        ));
                    }
                };
                let mut count = 0usize;
                for item in &items {
                    let mut local_bindings = bindings.clone();
                    local_bindings.insert(
                        Arc::from(variable.as_str()),
                        BindingValue::Scalar(item.clone()),
                    );
                    let pred_val = self.evaluate_expression(predicate, &local_bindings)?;
                    if matches!(pred_val, Value::Bool(true)) {
                        count += 1;
                    }
                }
                let result = match kind {
                    ListPredicateKind::All => count == items.len(),
                    ListPredicateKind::Any => count > 0,
                    ListPredicateKind::None => count == 0,
                    ListPredicateKind::Single => count == 1,
                };
                Ok(Value::Bool(result))
            }
            Expression::Exists(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                Ok(Value::Bool(!matches!(val, Value::Null)))
            }
            Expression::IsEmpty(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => Ok(Value::Bool(items.is_empty())),
                    Value::String(s) => Ok(Value::Bool(s.is_empty())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "isEmpty() requires a list or string".to_string(),
                    )),
                }
            }
            Expression::PatternPredicate(patterns) => {
                // Evaluate patterns against current bindings; return true if any match exists.
                // After matching, verify that variables already bound in the outer scope still
                // hold the same value (pattern matching may overwrite them with different nodes).
                let mut matches = vec![bindings.clone()];
                for pattern in patterns {
                    matches = self.match_pattern(pattern, matches)?;
                }
                matches.retain(|new_b| {
                    bindings
                        .iter()
                        .all(|(k, v)| new_b.get(k).is_none_or(|nv| nv == v))
                });
                Ok(Value::Bool(!matches.is_empty()))
            }
            Expression::Map(map) => {
                let mut result = std::collections::HashMap::new();
                for (k, v) in map {
                    result.insert(k.clone(), self.evaluate_expression(v, bindings)?);
                }
                Ok(Value::Map(result))
            }
            Expression::ScalarFn(func) => self.evaluate_scalar_function(func, bindings),
        }
    }

    pub(super) fn evaluate_case_expression(
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
                // Simple CASE: `operand = value` が true のとき（null は一致しない）
                self.cypher_equals(op_val, &condition_value) == Some(true)
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

    pub(super) fn apply_binary_op(
        &self,
        left: &Value,
        op: BinaryOp,
        right: &Value,
    ) -> Result<Value, ExecuteError> {
        // null を含む演算は null（AND / OR / XOR / IN は三値論理で個別に扱う）
        if matches!(left, Value::Null) || matches!(right, Value::Null) {
            match op {
                BinaryOp::And | BinaryOp::Or | BinaryOp::Xor | BinaryOp::In => {}
                _ => return Ok(Value::Null),
            }
        }
        match op {
            BinaryOp::Eq => Ok(bool_or_null(self.cypher_equals(left, right))),
            BinaryOp::Neq => Ok(bool_or_null(self.cypher_equals(left, right).map(|b| !b))),
            BinaryOp::Lt => self.compare_values(left, right, |ord| ord.is_lt()),
            BinaryOp::Gt => self.compare_values(left, right, |ord| ord.is_gt()),
            BinaryOp::Lte => self.compare_values(left, right, |ord| ord.is_le()),
            BinaryOp::Gte => self.compare_values(left, right, |ord| ord.is_ge()),
            // Kleene の三値論理: false AND null = false、true OR null = true、他の null 混じりは null
            BinaryOp::And => match (truth(left, "AND")?, truth(right, "AND")?) {
                (Some(false), _) | (_, Some(false)) => Ok(Value::Bool(false)),
                (Some(true), Some(true)) => Ok(Value::Bool(true)),
                _ => Ok(Value::Null),
            },
            BinaryOp::Or => match (truth(left, "OR")?, truth(right, "OR")?) {
                (Some(true), _) | (_, Some(true)) => Ok(Value::Bool(true)),
                (Some(false), Some(false)) => Ok(Value::Bool(false)),
                _ => Ok(Value::Null),
            },
            BinaryOp::Xor => match (left, right) {
                (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a ^ b)),
                (Value::Null, Value::Bool(_) | Value::Null) | (Value::Bool(_), Value::Null) => {
                    Ok(Value::Null)
                }
                _ => Err(ExecuteError::TypeError("XOR requires booleans".to_string())),
            },
            BinaryOp::Add => {
                if let (Value::List(a), Value::List(b)) = (left, right) {
                    let mut result = a.clone();
                    result.extend(b.iter().cloned());
                    return Ok(Value::List(result));
                }
                // テンポラル演算: Date + Duration, DateTime + Duration
                match (left, right) {
                    (
                        Value::Date(d),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let new_days = temporal::add_duration_to_date(*d, *months, *days, *millis);
                        return Ok(Value::Date(new_days));
                    }
                    (
                        Value::DateTime(ms),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let (y, mo, day, h, mi, s, frac) = temporal::millis_to_datetime(*ms);
                        let base_days = temporal::ymd_to_days(y, mo, day);
                        let new_days = temporal::add_duration_to_date(base_days, *months, *days, 0);
                        let (ny, nmo, nd) = temporal::days_to_ymd(new_days);
                        let new_ms =
                            temporal::datetime_to_millis(ny, nmo, nd, h, mi, s, frac) + millis;
                        return Ok(Value::DateTime(new_ms));
                    }
                    _ => {}
                }
                // String concatenation: 'a' + 'b', 'n=' + 1
                match (left, right) {
                    (Value::String(a), Value::String(b)) => {
                        return Ok(Value::String(format!("{a}{b}")));
                    }
                    (
                        Value::String(a),
                        other @ (Value::Int(_) | Value::Float(_) | Value::Bool(_)),
                    ) => {
                        return Ok(Value::String(format!("{a}{other}")));
                    }
                    (
                        other @ (Value::Int(_) | Value::Float(_) | Value::Bool(_)),
                        Value::String(b),
                    ) => {
                        return Ok(Value::String(format!("{other}{b}")));
                    }
                    _ => {}
                }
                self.arithmetic_op(left, right, i64::checked_add, |a, b| a + b)
            }
            BinaryOp::Sub => {
                // テンポラル演算: Date - Date = Duration, DateTime - DateTime = Duration
                match (left, right) {
                    (Value::Date(a), Value::Date(b)) => {
                        let diff_days = a - b;
                        return Ok(Value::Duration {
                            months: 0,
                            days: diff_days,
                            millis: 0,
                        });
                    }
                    (Value::DateTime(a), Value::DateTime(b)) => {
                        let diff_ms = a - b;
                        return Ok(Value::Duration {
                            months: 0,
                            days: 0,
                            millis: diff_ms,
                        });
                    }
                    (
                        Value::Date(d),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let new_days =
                            temporal::add_duration_to_date(*d, -*months, -*days, -*millis);
                        return Ok(Value::Date(new_days));
                    }
                    (
                        Value::DateTime(ms),
                        Value::Duration {
                            months,
                            days,
                            millis,
                        },
                    ) => {
                        use maharit_core::temporal;
                        let (y, mo, day, h, mi, s, frac) = temporal::millis_to_datetime(*ms);
                        let base_days = temporal::ymd_to_days(y, mo, day);
                        let new_days =
                            temporal::add_duration_to_date(base_days, -*months, -*days, 0);
                        let (ny, nmo, nd) = temporal::days_to_ymd(new_days);
                        let new_ms =
                            temporal::datetime_to_millis(ny, nmo, nd, h, mi, s, frac) - millis;
                        return Ok(Value::DateTime(new_ms));
                    }
                    _ => {}
                }
                self.arithmetic_op(left, right, i64::checked_sub, |a, b| a - b)
            }
            BinaryOp::Mul => self.arithmetic_op(left, right, i64::checked_mul, |a, b| a * b),
            BinaryOp::Div => {
                if matches!(right, Value::Int(0)) && matches!(left, Value::Int(_)) {
                    return Err(ExecuteError::TypeError("division by zero".to_string()));
                }
                self.arithmetic_op(left, right, i64::checked_div, |a, b| a / b)
            }
            BinaryOp::Mod => {
                if matches!(right, Value::Int(0)) && matches!(left, Value::Int(_)) {
                    return Err(ExecuteError::TypeError("division by zero".to_string()));
                }
                self.arithmetic_op(left, right, i64::checked_rem, |a, b| a % b)
            }
            BinaryOp::Regex => match (left, right) {
                (Value::String(s), Value::String(pattern)) => {
                    // Cypher =~ is full-match, so anchor the pattern
                    let anchored = format!("(?s)\\A(?:{})\\z", pattern);
                    let re = Regex::new(&anchored)
                        .map_err(|e| ExecuteError::TypeError(format!("invalid regex: {}", e)))?;
                    Ok(Value::Bool(re.is_match(s)))
                }
                _ => Err(ExecuteError::TypeError(
                    "=~ requires string operands".to_string(),
                )),
            },
            BinaryOp::Contains => match (left, right) {
                (Value::String(haystack), Value::String(needle)) => {
                    let result = self.string_contains(haystack, needle);
                    Ok(Value::Bool(result))
                }
                _ => Err(ExecuteError::TypeError(
                    "CONTAINS requires string operands".to_string(),
                )),
            },
            BinaryOp::StartsWith => match (left, right) {
                (Value::String(s), Value::String(prefix)) => Ok(Value::Bool(
                    s.to_lowercase().starts_with(&prefix.to_lowercase()),
                )),
                _ => Err(ExecuteError::TypeError(
                    "STARTS WITH requires string operands".to_string(),
                )),
            },
            BinaryOp::EndsWith => match (left, right) {
                (Value::String(s), Value::String(suffix)) => Ok(Value::Bool(
                    s.to_lowercase().ends_with(&suffix.to_lowercase()),
                )),
                _ => Err(ExecuteError::TypeError(
                    "ENDS WITH requires string operands".to_string(),
                )),
            },
            // 一致があれば true。無くて、左辺が null かリストに比べられない要素（null）があれば null
            BinaryOp::In => match right {
                Value::List(items) if items.is_empty() => Ok(Value::Bool(false)),
                Value::List(items) => {
                    let mut unknown = false;
                    for item in items {
                        match self.cypher_equals(left, item) {
                            Some(true) => return Ok(Value::Bool(true)),
                            Some(false) => {}
                            None => unknown = true,
                        }
                    }
                    Ok(if unknown {
                        Value::Null
                    } else {
                        Value::Bool(false)
                    })
                }
                Value::Null => Ok(Value::Null),
                _ => Ok(Value::Bool(false)),
            },
        }
    }

    /// `=` の意味での比較。どちらかが null（リスト・マップの中の null を含む）で
    /// 結果が決まらなければ `None`（null）。
    pub(super) fn cypher_equals(&self, left: &Value, right: &Value) -> Option<bool> {
        match (left, right) {
            (Value::Null, _) | (_, Value::Null) => None,
            (Value::List(a), Value::List(b)) => {
                if a.len() != b.len() {
                    return Some(false);
                }
                let mut unknown = false;
                for (x, y) in a.iter().zip(b) {
                    match self.cypher_equals(x, y) {
                        Some(false) => return Some(false),
                        None => unknown = true,
                        Some(true) => {}
                    }
                }
                if unknown { None } else { Some(true) }
            }
            (Value::Map(a), Value::Map(b)) => {
                if a.len() != b.len() || a.keys().any(|k| !b.contains_key(k)) {
                    return Some(false);
                }
                let mut unknown = false;
                for (k, x) in a {
                    match self.cypher_equals(x, &b[k]) {
                        Some(false) => return Some(false),
                        None => unknown = true,
                        Some(true) => {}
                    }
                }
                if unknown { None } else { Some(true) }
            }
            _ => Some(self.values_equal(left, right)),
        }
    }

    /// 同じ値か（null 同士も同じとみなす）。DISTINCT・グルーピングなど、値を区別するとき用。
    /// `=` の評価には [`Self::cypher_equals`] を使う。
    pub(super) fn values_equal(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                ((*a as f64) - b).abs() < f64::EPSILON
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::DateTime(a), Value::DateTime(b)) => a == b,
            (
                Value::Duration {
                    months: ma,
                    days: da,
                    millis: msa,
                },
                Value::Duration {
                    months: mb,
                    days: db,
                    millis: msb,
                },
            ) => ma == mb && da == db && msa == msb,
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.values_equal(x, y))
            }
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, x)| b.get(k).is_some_and(|y| self.values_equal(x, y)))
            }
            (Value::Edge(a), Value::Edge(b)) => a == b,
            (
                Value::Node(a) | Value::NodeData { id: a, .. },
                Value::Node(b) | Value::NodeData { id: b, .. },
            ) => a == b,
            (
                Value::Path {
                    nodes: na,
                    edges: ea,
                },
                Value::Path {
                    nodes: nb,
                    edges: eb,
                },
            ) => na == nb && ea == eb,
            _ => false,
        }
    }

    pub(super) fn compare_values<F>(
        &self,
        left: &Value,
        right: &Value,
        pred: F,
    ) -> Result<Value, ExecuteError>
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
            (Value::Date(a), Value::Date(b)) => a.cmp(b),
            (Value::DateTime(a), Value::DateTime(b)) => a.cmp(b),
            // 比べられない組み合わせ（null を含む、型が違うなど）は null
            _ => return Ok(Value::Null),
        };

        Ok(Value::Bool(pred(ordering)))
    }

    pub(super) fn arithmetic_op<F, G>(
        &self,
        left: &Value,
        right: &Value,
        int_op: F,
        float_op: G,
    ) -> Result<Value, ExecuteError>
    where
        F: Fn(i64, i64) -> Option<i64>,
        G: Fn(f64, f64) -> f64,
    {
        match (left, right) {
            // 整数演算は checked: オーバーフローや 0 除算で panic させない
            // （リリースビルドは panic=abort なので、サーバー全体が落ちる）
            (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
                .map(Value::Int)
                .ok_or_else(|| ExecuteError::TypeError("integer overflow".to_string())),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(*a, *b))),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(float_op(*a as f64, *b))),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(float_op(*a, *b as f64))),
            _ => Err(ExecuteError::TypeError(
                "arithmetic requires numbers".to_string(),
            )),
        }
    }

    pub(super) fn apply_unary_op(&self, op: UnaryOp, val: &Value) -> Result<Value, ExecuteError> {
        match op {
            UnaryOp::Not => match val {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                Value::Null => Ok(Value::Null),
                _ => Err(ExecuteError::TypeError("NOT requires boolean".to_string())),
            },
            UnaryOp::Neg => match val {
                Value::Null => Ok(Value::Null),
                Value::Int(n) => n
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or_else(|| ExecuteError::TypeError("integer overflow".to_string())),
                Value::Float(n) => Ok(Value::Float(-n)),
                _ => Err(ExecuteError::TypeError(
                    "negation requires number".to_string(),
                )),
            },
            UnaryOp::IsNull => Ok(Value::Bool(matches!(val, Value::Null))),
            UnaryOp::IsNotNull => Ok(Value::Bool(!matches!(val, Value::Null))),
            UnaryOp::IsNormalized => match val {
                Value::Null => Ok(Value::Null),
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

    /// Evaluate the CONTAINS operator with support for phrase and fuzzy search syntax.
    ///
    /// Query formats:
    /// - `"phrase terms"` — phrase search: all tokens must appear consecutively in haystack
    /// - `term~` or `term~N` — fuzzy search: any token in haystack within edit distance N
    /// - plain text — standard case-insensitive substring containment
    pub(super) fn string_contains(&self, haystack: &str, needle: &str) -> bool {
        // Phrase search: needle surrounded by double quotes
        if needle.len() >= 2 {
            let nb = needle.as_bytes();
            if nb[0] == b'"' && nb[needle.len() - 1] == b'"' {
                let phrase = &needle[1..needle.len() - 1];
                return self.contains_phrase(haystack, phrase);
            }
        }

        // Fuzzy search: needle ends with '~' (optionally followed by digit)
        if let Some(tilde_pos) = needle.rfind('~') {
            let term = &needle[..tilde_pos];
            let suffix = &needle[tilde_pos + 1..];
            if !term.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
                let max_distance: usize = if suffix.is_empty() {
                    2
                } else {
                    suffix.parse().unwrap_or(2)
                };
                return self.contains_fuzzy(haystack, term, max_distance);
            }
        }

        // Standard case-insensitive substring search
        haystack.to_lowercase().contains(&needle.to_lowercase())
    }

    /// Check whether all tokens of `phrase` appear consecutively in `haystack`.
    pub(super) fn contains_phrase(&self, haystack: &str, phrase: &str) -> bool {
        let haystack_tokens: Vec<&str> = haystack
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        let phrase_tokens: Vec<String> = phrase
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect();

        if phrase_tokens.is_empty() {
            return false;
        }
        if phrase_tokens.len() > haystack_tokens.len() {
            return false;
        }

        'outer: for i in 0..=(haystack_tokens.len() - phrase_tokens.len()) {
            for (j, phrase_token) in phrase_tokens.iter().enumerate() {
                if haystack_tokens[i + j].to_lowercase() != *phrase_token {
                    continue 'outer;
                }
            }
            return true;
        }

        false
    }

    /// Check whether any token in `haystack` is within `max_distance` Levenshtein
    /// distance from `term`.
    pub(super) fn contains_fuzzy(&self, haystack: &str, term: &str, max_distance: usize) -> bool {
        let term_lower = term.to_lowercase();
        haystack
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .any(|token| levenshtein_distance(token, &term_lower) <= max_distance)
    }
}

/// 論理演算の引数を真偽値（null は `None`）として読む。
fn truth(value: &Value, op: &str) -> Result<Option<bool>, ExecuteError> {
    match value {
        Value::Bool(b) => Ok(Some(*b)),
        Value::Null => Ok(None),
        _ => Err(ExecuteError::TypeError(format!("{op} requires booleans"))),
    }
}

fn bool_or_null(value: Option<bool>) -> Value {
    value.map_or(Value::Null, Value::Bool)
}
