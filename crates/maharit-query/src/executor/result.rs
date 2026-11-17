//! RETURN の評価、ORDER BY
//!  SKIP
//!  LIMIT、結果表の組み立て。

use super::*;

impl<'a> Executor<'a> {
    pub(super) fn build_result_set(
        &self,
        return_clause: &ReturnClause,
        bindings_list: &[Bindings],
    ) -> Result<ResultSet, ExecuteError> {
        // Check if any aggregation is present
        let has_aggregation = return_clause.items.iter().any(Self::is_aggregate);

        if has_aggregation {
            return self.build_aggregated_result_set(return_clause, bindings_list);
        }

        // Build column names
        let columns: Vec<String> = return_clause
            .items
            .iter()
            .map(|item| self.return_item_to_column_name(item))
            .collect();

        // Resolve SKIP and LIMIT expressions (integer literal or $parameter)
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
        // With ORDER BY and LIMIT, only the first `needed` rows are kept.
        let needed = match (resolved_skip, resolved_limit) {
            (Some(skip), Some(limit)) => Some(skip.saturating_add(limit) as usize),
            (None, Some(limit)) => Some(limit as usize),
            _ => None,
        };
        let order_slots = return_clause
            .order_by
            .as_ref()
            .map(|order_by| self.order_key_slots(order_by, &columns));

        // ORDER BY ... LIMIT without DISTINCT: sort on the keys alone and
        // evaluate the RETURN items only for the rows that remain. (DISTINCT
        // needs every row's values before it can drop duplicates.)
        if let (Some(order_by), Some(slots), Some(needed)) =
            (&return_clause.order_by, &order_slots, needed)
            && !return_clause.distinct
            && needed < bindings_list.len()
        {
            let mut rows =
                self.build_top_rows(return_clause, order_by, slots, bindings_list, needed)?;
            if let Some(skip) = resolved_skip {
                rows.drain(..(skip as usize).min(rows.len()));
            }
            return Ok(ResultSet::new(columns, rows));
        }

        // Build rows (and ORDER BY keys, which may refer to values that are
        // not returned, e.g. `RETURN n.name ORDER BY n.age`).
        let mut rows = Vec::new();
        let mut keys = Vec::new();

        for bindings in bindings_list {
            let mut row_values = Vec::new();

            for item in &return_clause.items {
                row_values.push(self.evaluate_return_item(item, bindings)?);
            }

            let row = Row {
                columns: row_values,
            };
            if let (Some(order_by), Some(slots)) = (&return_clause.order_by, &order_slots) {
                keys.push(self.order_keys(order_by, slots, &row, Some(bindings))?);
            }
            rows.push(row);
        }

        // Apply DISTINCT (keeping each surviving row's ORDER BY keys aligned)
        if return_clause.distinct {
            let mut seen = std::collections::HashSet::new();
            let has_keys = !keys.is_empty();
            let mut kept_rows = Vec::new();
            let mut kept_keys = Vec::new();
            let mut keys_iter = keys.into_iter();
            for row in rows {
                let key = has_keys.then(|| keys_iter.next().unwrap_or_default());
                if seen.insert(self.row_to_key(&row)) {
                    kept_rows.push(row);
                    kept_keys.extend(key);
                }
            }
            rows = kept_rows;
            keys = kept_keys;
        }

        // Apply ORDER BY with optional LIMIT optimization
        if let Some(ref order_by) = return_clause.order_by {
            rows = self.sort_rows_by_keys(rows, keys, order_by, needed);
        }

        // Apply SKIP
        if let Some(skip) = resolved_skip {
            let skip = skip as usize;
            if skip < rows.len() {
                rows = rows.into_iter().skip(skip).collect();
            } else {
                rows.clear();
            }
        }

        // Apply LIMIT
        if let Some(limit) = resolved_limit {
            rows.truncate(limit as usize);
        }

        Ok(ResultSet::new(columns, rows))
    }

    pub(super) fn row_to_key(&self, row: &Row) -> String {
        row.columns
            .iter()
            .map(|v| format!("{:?}", v))
            .collect::<Vec<_>>()
            .join("|")
    }

    /// ORDER BY の各キーの値を求める。
    ///
    /// キーが返却列（列名が一致）ならその値を使う。そうでなければ射影前の
    /// バインディング `source` から評価する（`RETURN n.name ORDER BY n.age`）。
    /// `source` が無い（集計後）場合に返却列でないキーはエラーにする。以前は
    /// 列が見つからないキーを黙って無視し、並べ替えずに結果を返していた。
    ///
    /// `slots` は [`order_key_slots`](Self::order_key_slots) の結果（行ごとに
    /// 列名を作って探さないよう、呼び出し側が 1 回だけ求める）。
    pub(super) fn order_keys(
        &self,
        order_by: &OrderByClause,
        slots: &[Option<usize>],
        row: &Row,
        source: Option<&Bindings>,
    ) -> Result<Vec<Value>, ExecuteError> {
        order_by
            .items
            .iter()
            .zip(slots)
            .map(|(item, slot)| {
                if let Some(idx) = slot {
                    return Ok(row.columns[*idx].clone());
                }
                match source {
                    Some(bindings) => self.evaluate_return_item(&item.expression, bindings),
                    None => Err(ExecuteError::TypeError(format!(
                        "ORDER BY {}: with aggregation, ORDER BY can only refer to returned columns (add it to RETURN or use an alias)",
                        self.return_item_to_column_name(&item.expression)
                    ))),
                }
            })
            .collect()
    }

    /// ORDER BY の各キーが返却列のどれに当たるか（当たらなければ `None`）。
    pub(super) fn order_key_slots(
        &self,
        order_by: &OrderByClause,
        columns: &[String],
    ) -> Vec<Option<usize>> {
        order_by
            .items
            .iter()
            .map(|item| {
                let name = self.return_item_to_column_name(&item.expression);
                columns.iter().position(|c| *c == name)
            })
            .collect()
    }

    /// `ORDER BY … LIMIT` で残る行だけを組み立てる。
    ///
    /// 全行について並べ替えキーだけを求め、上位 `needed` 件を選んでから、その行の
    /// RETURN の式を評価する。残らない行の RETURN の式は評価しない。
    pub(super) fn build_top_rows(
        &self,
        return_clause: &ReturnClause,
        order_by: &OrderByClause,
        slots: &[Option<usize>],
        bindings_list: &[Bindings],
        needed: usize,
    ) -> Result<Vec<Row>, ExecuteError> {
        if needed == 0 {
            return Ok(Vec::new());
        }
        let mut keyed: Vec<(Vec<Value>, usize)> = Vec::with_capacity(bindings_list.len());
        for (i, bindings) in bindings_list.iter().enumerate() {
            let key = order_by
                .items
                .iter()
                .zip(slots)
                .map(|(item, slot)| match slot {
                    Some(idx) => self.evaluate_return_item(&return_clause.items[*idx], bindings),
                    None => self.evaluate_return_item(&item.expression, bindings),
                })
                .collect::<Result<Vec<_>, _>>()?;
            keyed.push((key, i));
        }
        // 同じキーの行は入力の順に並べる（全件を安定ソートしたときと同じ結果にする）
        let cmp = |a: &(Vec<Value>, usize), b: &(Vec<Value>, usize)| {
            self.compare_keys(&a.0, &b.0, order_by).then(a.1.cmp(&b.1))
        };
        if needed < keyed.len() {
            keyed.select_nth_unstable_by(needed, cmp);
            keyed.truncate(needed);
        }
        keyed.sort_unstable_by(cmp);
        keyed
            .into_iter()
            .map(|(_, i)| {
                let columns = return_clause
                    .items
                    .iter()
                    .map(|item| self.evaluate_return_item(item, &bindings_list[i]))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Row { columns })
            })
            .collect()
    }

    pub(super) fn compare_keys(
        &self,
        a: &[Value],
        b: &[Value],
        order_by: &OrderByClause,
    ) -> std::cmp::Ordering {
        for (i, item) in order_by.items.iter().enumerate() {
            let cmp = self.compare_values_for_sort(&a[i], &b[i], item.direction, item.nulls_order);
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    }

    /// `keys[i]` を `rows[i]` の並べ替えキーとしてソートする。`needed` があり行数より
    /// 少なければ上位 `needed` 件だけを部分選択してからソートする（TopN 最適化）。
    pub(super) fn sort_rows_by_keys(
        &self,
        rows: Vec<Row>,
        keys: Vec<Vec<Value>>,
        order_by: &OrderByClause,
        needed: Option<usize>,
    ) -> Vec<Row> {
        let mut pairs: Vec<(Vec<Value>, Row)> = keys.into_iter().zip(rows).collect();
        let cmp =
            |a: &(Vec<Value>, Row), b: &(Vec<Value>, Row)| self.compare_keys(&a.0, &b.0, order_by);
        if let Some(n) = needed
            && n < pairs.len()
        {
            if n == 0 {
                return Vec::new();
            }
            pairs.select_nth_unstable_by(n, cmp);
            pairs.truncate(n);
        }
        pairs.sort_by(cmp);
        pairs.into_iter().map(|(_, row)| row).collect()
    }

    pub(super) fn compare_values_for_sort(
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

    /// Alias の内側を再帰的に辿って、集計関数かどうかを判定するヘルパー。
    pub(super) fn is_aggregate(item: &ReturnItem) -> bool {
        match item {
            ReturnItem::Aggregate(_) => true,
            ReturnItem::Alias(inner, _) => Self::is_aggregate(inner),
            _ => false,
        }
    }

    /// RETURN 項目の結果列名（エイリアス、または式の表示形）。シャーディングの
    /// コーディネーターがシャードの結果を列名で突き合わせるために使う。
    pub fn column_name(item: &ReturnItem) -> String {
        match item {
            ReturnItem::Alias(_, name) => name.clone(),
            ReturnItem::Variable(v) => v.clone(),
            ReturnItem::Property(v, p) => format!("{}.{}", v, p),
            ReturnItem::All => "*".to_string(),
            ReturnItem::Expr(e) => Self::expression_to_display(e),
            ReturnItem::Aggregate(agg) => match agg {
                AggregateFunction::Count(inner) => match inner {
                    None => "COUNT(*)".to_string(),
                    Some(inner) => format!("count({})", Self::column_name(inner)),
                },
                AggregateFunction::Sum(inner) => {
                    format!("SUM({})", Self::column_name(inner))
                }
                AggregateFunction::Avg(inner) => {
                    format!("AVG({})", Self::column_name(inner))
                }
                AggregateFunction::Min(inner) => {
                    format!("MIN({})", Self::column_name(inner))
                }
                AggregateFunction::Max(inner) => {
                    format!("MAX({})", Self::column_name(inner))
                }
                AggregateFunction::Collect(inner) => {
                    format!("COLLECT({})", Self::column_name(inner))
                }
                AggregateFunction::PercentileCont(e, p) => format!(
                    "percentileCont({}, {})",
                    Self::column_name(e),
                    Self::column_name(p)
                ),
                AggregateFunction::PercentileDisc(e, p) => format!(
                    "percentileDisc({}, {})",
                    Self::column_name(e),
                    Self::column_name(p)
                ),
                AggregateFunction::StDev(inner) => {
                    format!("stDev({})", Self::column_name(inner))
                }
                AggregateFunction::StDevP(inner) => {
                    format!("stDevP({})", Self::column_name(inner))
                }
                AggregateFunction::CountDistinct(inner) => {
                    format!("COUNT(DISTINCT {})", Self::column_name(inner))
                }
                AggregateFunction::SumDistinct(inner) => {
                    format!("SUM(DISTINCT {})", Self::column_name(inner))
                }
                AggregateFunction::AvgDistinct(inner) => {
                    format!("AVG(DISTINCT {})", Self::column_name(inner))
                }
                AggregateFunction::CollectDistinct(inner) => {
                    format!("COLLECT(DISTINCT {})", Self::column_name(inner))
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
                ScalarFunction::Trim(e) => format!("trim({})", Self::expression_to_display(e)),
                ScalarFunction::LTrim(e) => format!("ltrim({})", Self::expression_to_display(e)),
                ScalarFunction::RTrim(e) => format!("rtrim({})", Self::expression_to_display(e)),
                ScalarFunction::ToLower(e) => {
                    format!("toLower({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToUpper(e) => {
                    format!("toUpper({})", Self::expression_to_display(e))
                }
                ScalarFunction::Reverse(e) => {
                    format!("reverse({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToString(e) => {
                    format!("toString({})", Self::expression_to_display(e))
                }
                ScalarFunction::Size(e) => format!("size({})", Self::expression_to_display(e)),
                ScalarFunction::Left(e1, e2) => format!(
                    "left({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::Right(e1, e2) => format!(
                    "right({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::Substring(e1, e2, e3) => {
                    if let Some(e3) = e3 {
                        format!(
                            "substring({}, {}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2),
                            Self::expression_to_display(e3)
                        )
                    } else {
                        format!(
                            "substring({}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2)
                        )
                    }
                }
                ScalarFunction::Split(e1, e2) => format!(
                    "split({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::Replace(e1, e2, e3) => format!(
                    "replace({}, {}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2),
                    Self::expression_to_display(e3)
                ),
                ScalarFunction::Abs(e) => format!("abs({})", Self::expression_to_display(e)),
                ScalarFunction::Ceil(e) => format!("ceil({})", Self::expression_to_display(e)),
                ScalarFunction::Floor(e) => format!("floor({})", Self::expression_to_display(e)),
                ScalarFunction::Round(e, p) => {
                    if let Some(p) = p {
                        format!(
                            "round({}, {})",
                            Self::expression_to_display(e),
                            Self::expression_to_display(p)
                        )
                    } else {
                        format!("round({})", Self::expression_to_display(e))
                    }
                }
                ScalarFunction::Sign(e) => format!("sign({})", Self::expression_to_display(e)),
                ScalarFunction::Rand => "rand()".to_string(),
                ScalarFunction::IsNaN(e) => format!("isNaN({})", Self::expression_to_display(e)),
                ScalarFunction::Log(e) => format!("log({})", Self::expression_to_display(e)),
                ScalarFunction::Log10(e) => format!("log10({})", Self::expression_to_display(e)),
                ScalarFunction::Sqrt(e) => format!("sqrt({})", Self::expression_to_display(e)),
                ScalarFunction::Math(f, e) => {
                    format!("{}({})", f.name(), Self::expression_to_display(e))
                }
                ScalarFunction::Math2(f, a, b) => format!(
                    "{}({}, {})",
                    f.name(),
                    Self::expression_to_display(a),
                    Self::expression_to_display(b)
                ),
                ScalarFunction::E => "e()".to_string(),
                ScalarFunction::Pi => "pi()".to_string(),
                ScalarFunction::Id(var) => format!("id({})", var),
                ScalarFunction::ElementId(var) => format!("elementId({})", var),
                ScalarFunction::Type(var) => format!("type({})", var),
                ScalarFunction::StartNode(var) => format!("startNode({})", var),
                ScalarFunction::EndNode(var) => format!("endNode({})", var),
                ScalarFunction::Labels(var) => format!("labels({})", var),
                ScalarFunction::Properties(var) => format!("properties({})", var),
                ScalarFunction::Keys(var) => format!("keys({})", var),
                ScalarFunction::Coalesce(_) => "coalesce(...)".to_string(),
                ScalarFunction::NullIf(e1, e2) => format!(
                    "nullIf({}, {})",
                    Self::expression_to_display(e1),
                    Self::expression_to_display(e2)
                ),
                ScalarFunction::ToBoolean(e) => {
                    format!("toBoolean({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToFloat(e) => {
                    format!("toFloat({})", Self::expression_to_display(e))
                }
                ScalarFunction::ToInteger(e) => {
                    format!("toInteger({})", Self::expression_to_display(e))
                }
                ScalarFunction::Timestamp => "timestamp()".to_string(),
                ScalarFunction::RandomUUID => "randomUUID()".to_string(),
                ScalarFunction::Head(e) => format!("head({})", Self::expression_to_display(e)),
                ScalarFunction::Last(e) => format!("last({})", Self::expression_to_display(e)),
                ScalarFunction::Tail(e) => format!("tail({})", Self::expression_to_display(e)),
                ScalarFunction::Range(e1, e2, e3) => {
                    if let Some(e3) = e3 {
                        format!(
                            "range({}, {}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2),
                            Self::expression_to_display(e3)
                        )
                    } else {
                        format!(
                            "range({}, {})",
                            Self::expression_to_display(e1),
                            Self::expression_to_display(e2)
                        )
                    }
                }
                ScalarFunction::Reduce { .. } => "reduce(...)".to_string(),
                ScalarFunction::DateFunc(_) => "date(...)".to_string(),
                ScalarFunction::DateTimeFunc(_) => "datetime(...)".to_string(),
                ScalarFunction::DurationFunc(_) => "duration(...)".to_string(),
            },
        }
    }

    pub(super) fn return_item_to_column_name(&self, item: &ReturnItem) -> String {
        Self::column_name(item)
    }

    pub(super) fn evaluate_return_item(
        &self,
        item: &ReturnItem,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        match item {
            ReturnItem::Variable(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                Ok(Value::NodeData {
                                    id: *node_id,
                                    labels: node.labels.clone(),
                                    properties: Arc::clone(&node.properties),
                                })
                            } else {
                                Ok(Value::Node(*node_id))
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            // Return edge as a simple value
                            Ok(Value::Edge(*edge_id))
                        }
                        BindingValue::Path { nodes, edges } => Ok(Value::Path {
                            nodes: nodes.clone(),
                            edges: edges.clone(),
                        }),
                        BindingValue::Scalar(value) => Ok(value.clone()),
                    }
                } else {
                    // Not in scope (e.g. dropped by WITH): an error, not null (bug/162)
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ReturnItem::Property(var, prop) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                Ok(node
                                    .get_property(prop)
                                    .map(Value::from)
                                    .unwrap_or(Value::Null))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                Ok(edge
                                    .get_property(prop)
                                    .map(Value::from)
                                    .unwrap_or(Value::Null))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        // Maps, nodes and relationships held as values, temporal fields
                        BindingValue::Scalar(_) => self.evaluate_expression(
                            &Expression::Property(var.clone(), prop.clone()),
                            bindings,
                        ),
                        _ => Ok(Value::Null),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ReturnItem::All => {
                // For *, we return the first bound node variable
                for binding_value in bindings.values() {
                    if let BindingValue::Node(node_id) = binding_value
                        && let Some(node) = self.graph_ref().get_node(*node_id)
                    {
                        return Ok(Value::NodeData {
                            id: *node_id,
                            labels: node.labels.clone(),
                            properties: Arc::clone(&node.properties),
                        });
                    }
                }
                Ok(Value::Null)
            }
            ReturnItem::Alias(inner, _) => self.evaluate_return_item(inner, bindings),
            ReturnItem::Aggregate(_) => {
                // Aggregates are handled separately
                Ok(Value::Null)
            }
            ReturnItem::Function(func) => self.evaluate_scalar_function(func, bindings),
            ReturnItem::Expr(expr) => self.evaluate_expression(expr, bindings),
        }
    }
}
