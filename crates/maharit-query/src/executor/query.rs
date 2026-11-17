//! MATCH から始まる読み取り文の実行（UNION、CALL サブクエリ、WITH、
//! WHERE の索引への押し下げ、件数の近道）と列名の組み立て。

use super::*;

impl<'a> Executor<'a> {
    // ========== UNION ==========

    pub(super) fn execute_union(
        &mut self,
        union_stmt: UnionStatement,
    ) -> Result<ResultSet, ExecuteError> {
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

    // ========== MATCH ==========

    pub(super) fn execute_match(&mut self, m: MatchStatement) -> Result<ResultSet, ExecuteError> {
        // Fast path: `MATCH (n:L) RETURN count(*)` / `count(n)` with no filters
        // answers from the label/node count without materializing any bindings.
        if let Some(rs) = self.try_simple_count(&m) {
            return Ok(rs);
        }

        // Detect an early-termination limit: applicable only when there is no ORDER BY
        // and no aggregation, since both require collecting all rows before outputting.
        let has_aggregation = m.return_clause.items.iter().any(Self::is_aggregate);

        let early_limit: Option<usize> = if !has_aggregation && m.return_clause.order_by.is_none() {
            m.return_clause
                .limit
                .as_ref()
                .and_then(|e| self.resolve_skip_limit(e).ok())
                .map(|n| {
                    // Include SKIP in the early cutoff so ORDER-agnostic queries
                    // still yield the correct slice.
                    let skip = m
                        .return_clause
                        .skip
                        .as_ref()
                        .and_then(|e| self.resolve_skip_limit(e).ok())
                        .unwrap_or(0) as usize;
                    n as usize + skip
                })
        } else {
            None
        };

        // Process each segment, applying the early cutoff after each one.
        let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

        for segment in &m.segments {
            all_bindings = self.execute_query_segment(segment, all_bindings)?;

            if let Some(limit) = early_limit
                && all_bindings.len() >= limit
            {
                all_bindings.truncate(limit);
                break;
            }
        }

        // Execute CALL subquery if present
        if let Some(ref call) = m.call_clause {
            all_bindings = self.execute_call_subquery(call, all_bindings)?;
        }

        // Build result set
        self.build_result_set(&m.return_clause, &all_bindings)
    }

    /// Fast path for a filter-free node count: `MATCH (n:L) RETURN count(*)`
    /// or `count(n)`. Returns `Some(result)` when the query matches this shape,
    /// answering from `nodes_by_label`/`node_count` without building bindings.
    /// Returns `None` (fall back to the general path) for anything more complex.
    pub(super) fn try_simple_count(&self, m: &MatchStatement) -> Option<ResultSet> {
        if m.call_clause.is_some() || m.segments.len() != 1 {
            return None;
        }
        let seg = &m.segments[0];
        if seg.where_clause.is_some() || seg.with_clause.is_some() || !seg.unwinds.is_empty() {
            return None;
        }
        if seg.match_clauses.len() != 1 {
            return None;
        }
        let clause = &seg.match_clauses[0];
        if clause.optional || clause.patterns.len() != 1 {
            return None;
        }
        let np = match &clause.patterns[0] {
            Pattern::Node(np) => np,
            _ => return None,
        };
        // Inline properties or multiple labels would filter the set, so the
        // simple count identity no longer holds.
        if !np.properties.is_empty() || np.properties_param.is_some() || np.labels.len() > 1 {
            return None;
        }

        let rc = &m.return_clause;
        if rc.order_by.is_some() || rc.skip.is_some() || rc.limit.is_some() || rc.items.is_empty() {
            return None;
        }

        // Every return item must be a plain count(*) or count(<the node var>).
        let node_var = np.variable.as_deref();
        if !rc.items.iter().all(|it| Self::is_plain_count(it, node_var)) {
            return None;
        }

        let cnt = match np.labels.first() {
            Some(label) => self.graph_ref().nodes_by_label(label).len(),
            None => self.graph_ref().node_count(),
        } as i64;

        let columns: Vec<String> = rc
            .items
            .iter()
            .map(|it| self.return_item_to_column_name(it))
            .collect();
        let row_values = vec![Value::Int(cnt); rc.items.len()];
        Some(ResultSet::new(
            columns,
            vec![Row {
                columns: row_values,
            }],
        ))
    }

    /// True if `item` is `count(*)` or `count(v)` where `v` is `node_var`
    /// (unwrapping any `AS` alias). Both count exactly the matched node set.
    pub(super) fn is_plain_count(item: &ReturnItem, node_var: Option<&str>) -> bool {
        match item {
            ReturnItem::Alias(inner, _) => Self::is_plain_count(inner, node_var),
            ReturnItem::Aggregate(AggregateFunction::Count(inner)) => match inner {
                None => true, // count(*)
                Some(boxed) => match boxed.as_ref() {
                    ReturnItem::Variable(v) => node_var == Some(v.as_str()),
                    _ => false,
                },
            },
            _ => false,
        }
    }

    pub(super) fn execute_call_subquery(
        &self,
        call: &CallSubquery,
        outer_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        for outer in outer_bindings {
            // Run every UNION branch and concatenate their rows.
            let (col_names, mut rows) = self.call_subquery_branch_rows(call, &outer)?;
            let mut dedup = false;
            let mut branch = call;
            while let Some((union_type, next)) = &branch.union {
                let (next_cols, next_rows) = self.call_subquery_branch_rows(next, &outer)?;
                if next_cols != col_names {
                    return Err(ExecuteError::TypeError(format!(
                        "all UNION branches in CALL {{}} must return the same columns: [{}] vs [{}]",
                        col_names.join(", "),
                        next_cols.join(", ")
                    )));
                }
                rows.extend(next_rows);
                dedup |= *union_type == UnionType::Union;
                branch = next;
            }
            if dedup {
                let mut seen = HashSet::new();
                rows.retain(|row| {
                    let key: Vec<String> = row.columns.iter().map(|v| format!("{}", v)).collect();
                    seen.insert(key.join("\x00"))
                });
            }

            // No inner results - skip this outer row (inner join semantics).
            // Otherwise combine the outer binding with each inner row's columns as scalars.
            for inner_row in &rows {
                let mut merged = outer.clone();
                for (col_name, col_val) in col_names.iter().zip(inner_row.columns.iter()) {
                    merged.insert(
                        Arc::from(col_name.as_str()),
                        BindingValue::Scalar(col_val.clone()),
                    );
                }
                result.push(merged);
            }
        }

        Ok(result)
    }

    /// Evaluate one CALL subquery branch (ignoring its UNION continuation) for
    /// a single outer row. Returns the output column names and rows.
    pub(super) fn call_subquery_branch_rows(
        &self,
        call: &CallSubquery,
        outer: &Bindings,
    ) -> Result<(Vec<String>, Vec<Row>), ExecuteError> {
        // Build inner starting bindings from WITH imports
        let inner_start = if let Some(ref imports) = call.with_import {
            let mut inner = Bindings::new();
            for var in imports {
                if let Some(val) = outer.get(var.as_str()) {
                    inner.insert(Arc::from(var.as_str()), val.clone());
                }
            }
            inner
        } else {
            // Without WITH, inner subquery has no access to outer bindings
            Bindings::new()
        };

        // Execute inner MATCH
        let mut inner_bindings = vec![inner_start];
        inner_bindings = self.execute_match_clause(&call.match_clause, inner_bindings)?;

        // Apply inner WHERE
        if let Some(ref where_expr) = call.where_clause {
            inner_bindings.retain(|b| {
                self.evaluate_expression(where_expr, b)
                    .map(|v| matches!(v, Value::Bool(true)))
                    .unwrap_or(false)
            });
        }

        // Evaluate each return item for all inner bindings
        // We build a temporary ReturnClause to use build_result_set for aggregation support
        let temp_return_clause = ReturnClause {
            distinct: false,
            items: call
                .return_items
                .iter()
                .map(|ri| ri.expression.clone())
                .collect(),
            order_by: None,
            skip: None,
            limit: None,
        };
        let inner_result = self.build_result_set(&temp_return_clause, &inner_bindings)?;

        // Compute effective column names (alias takes priority over generated name)
        let col_names: Vec<String> = call
            .return_items
            .iter()
            .enumerate()
            .map(|(i, ri)| {
                ri.alias.clone().unwrap_or_else(|| {
                    inner_result
                        .columns
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("col{}", i))
                })
            })
            .collect();

        Ok((col_names, inner_result.rows))
    }

    pub(super) fn execute_query_segment(
        &mut self,
        segment: &QuerySegment,
        mut bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Filter pushdown: pull simple `var.prop = <const>` equalities out of the
        // WHERE clause and inject them into the matching node patterns. This lets
        // `match_node_pattern` use the property index (or at least filter during
        // the scan) instead of materializing every label match and filtering
        // afterwards. The original WHERE retain below still runs, so semantics are
        // unchanged — we only prune candidates earlier.
        let pushable = match &segment.where_clause {
            Some(w) => {
                let mut v = Vec::new();
                Self::collect_pushable_equalities(w, &mut v);
                v
            }
            None => Vec::new(),
        };

        // Range pushdown: collect `var.prop <op> <numeric>` predicates and stash
        // them so `match_node_pattern` can narrow candidates via the range index
        // (only an optimization; the WHERE retain below guarantees correctness).
        //
        // Skip entirely when no property index is defined: range narrowing needs
        // an index to fire, so without one this collection is pure overhead on
        // every WHERE query.
        self.range_hints = match &segment.where_clause {
            Some(w) if self.property_index.has_any_index() => {
                let mut v = Vec::new();
                Self::collect_range_predicates(w, &mut v);
                v
            }
            _ => Vec::new(),
        };

        // UNWIND: 行ごとにリストを展開する（null は 0 行、リスト以外は 1 行）
        for unwind in &segment.unwinds {
            let mut expanded = Vec::new();
            for b in bindings {
                let items = match self.evaluate_expression(&unwind.expression, &b)? {
                    Value::List(items) => items,
                    Value::Null => Vec::new(),
                    other => vec![other],
                };
                for item in items {
                    let value = match item {
                        Value::Node(id) | Value::NodeData { id, .. } => BindingValue::Node(id),
                        Value::Edge(id) => BindingValue::Edge(id),
                        other => BindingValue::Scalar(other),
                    };
                    let mut row = b.clone();
                    row.insert(Arc::from(unwind.variable.as_str()), value);
                    expanded.push(row);
                }
            }
            bindings = expanded;
        }

        // Execute MATCH clauses (augmented with pushed-down predicates where safe).
        for match_clause in &segment.match_clauses {
            if pushable.is_empty() || match_clause.optional {
                // OPTIONAL MATCH is skipped: pushing a predicate into it could turn
                // a NULL-preserving row into a filtered one before the WHERE runs.
                bindings = self.execute_match_clause(match_clause, bindings)?;
            } else {
                let augmented = MatchClause {
                    patterns: Self::augment_patterns(&match_clause.patterns, &pushable),
                    optional: match_clause.optional,
                };
                bindings = self.execute_match_clause(&augmented, bindings)?;
            }
        }

        // Confine range hints to this segment's node scan so they cannot affect
        // later WHERE-clause pattern predicates, WITH, or subquery evaluation.
        self.range_hints.clear();

        // Apply WHERE filter
        if let Some(where_expr) = &segment.where_clause {
            bindings.retain(|b| {
                self.evaluate_expression(where_expr, b)
                    .map(|v| matches!(v, Value::Bool(true)))
                    .unwrap_or(false)
            });
        }

        // Apply WITH clause if present
        if let Some(with_clause) = &segment.with_clause {
            bindings = self.apply_with_clause(with_clause, bindings)?;
        }

        Ok(bindings)
    }

    // ── WHERE 述語のパターン押し下げ（フィルタプッシュダウン）補助 ──────────────

    /// 押し下げ可能な定数（リテラル or パラメータ）かどうか。
    /// 他変数を参照する式は、マッチ時点で束縛されているとは限らないため対象外。
    pub(super) fn is_pushable_const(expr: &Expression) -> bool {
        matches!(expr, Expression::Literal(_) | Expression::Parameter(_))
    }

    /// WHERE 式からトップレベルで AND 連結された `var.prop = <const>` の等価述語を
    /// 収集する。OR / NOT などの下には降りない（誤って絞り込まないため）。
    pub(super) fn collect_pushable_equalities(
        expr: &Expression,
        out: &mut Vec<(String, String, Expression)>,
    ) {
        match expr {
            Expression::BinaryOp(l, BinaryOp::And, r) => {
                Self::collect_pushable_equalities(l, out);
                Self::collect_pushable_equalities(r, out);
            }
            Expression::BinaryOp(l, BinaryOp::Eq, r) => match (l.as_ref(), r.as_ref()) {
                (Expression::Property(v, p), rhs) if Self::is_pushable_const(rhs) => {
                    out.push((v.clone(), p.clone(), rhs.clone()));
                }
                (lhs, Expression::Property(v, p)) if Self::is_pushable_const(lhs) => {
                    out.push((v.clone(), p.clone(), lhs.clone()));
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// WHERE 式からトップレベル AND で連結された数値の範囲述語
    /// `var.prop <op> <numeric literal>` を収集する（op ∈ Lt/Gt/Lte/Gte）。
    /// プロパティが左右どちらの辺でも、演算子を向きに合わせて正規化する。
    /// 収集結果は範囲索引での候補絞り込みに使う（正当性は WHERE retain が保証）。
    pub(super) fn collect_range_predicates(
        expr: &Expression,
        out: &mut Vec<(String, String, BinaryOp, PropertyValue)>,
    ) {
        // リテラルを数値の PropertyValue に変換（それ以外は None）。
        fn numeric(expr: &Expression) -> Option<PropertyValue> {
            match expr {
                Expression::Literal(Literal::Int(n)) => Some(PropertyValue::Int(*n)),
                Expression::Literal(Literal::Float(f)) => Some(PropertyValue::Float(*f)),
                _ => None,
            }
        }
        // プロパティが右辺にある場合に演算子を反転する（`40 < n.age` → `n.age > 40`）。
        fn flip(op: BinaryOp) -> BinaryOp {
            match op {
                BinaryOp::Lt => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Lt,
                BinaryOp::Lte => BinaryOp::Gte,
                BinaryOp::Gte => BinaryOp::Lte,
                other => other,
            }
        }
        match expr {
            Expression::BinaryOp(l, BinaryOp::And, r) => {
                Self::collect_range_predicates(l, out);
                Self::collect_range_predicates(r, out);
            }
            Expression::BinaryOp(l, op, r)
                if matches!(
                    op,
                    BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Lte | BinaryOp::Gte
                ) =>
            {
                match (l.as_ref(), r.as_ref()) {
                    (Expression::Property(v, p), rhs) => {
                        if let Some(val) = numeric(rhs) {
                            out.push((v.clone(), p.clone(), *op, val));
                        }
                    }
                    (lhs, Expression::Property(v, p)) => {
                        if let Some(val) = numeric(lhs) {
                            out.push((v.clone(), p.clone(), flip(*op), val));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// パターン列の各ノードパターンに、対応する変数の等価述語を注入したコピーを返す。
    pub(super) fn augment_patterns(
        patterns: &[Pattern],
        eqs: &[(String, String, Expression)],
    ) -> Vec<Pattern> {
        patterns
            .iter()
            .map(|p| match p {
                Pattern::Node(np) => Pattern::Node(Self::augment_node_pattern(np, eqs)),
                Pattern::Path(pp) => Pattern::Path(PathPattern {
                    variable: pp.variable.clone(),
                    start: Self::augment_node_pattern(&pp.start, eqs),
                    segments: pp
                        .segments
                        .iter()
                        .map(|seg| PathSegment {
                            edge: seg.edge.clone(),
                            node: Self::augment_node_pattern(&seg.node, eqs),
                        })
                        .collect(),
                    shortest: pp.shortest,
                }),
            })
            .collect()
    }

    /// ノードパターンの変数に一致する等価述語をプロパティ制約として追加する。
    /// 既存のインラインプロパティは上書きしない（両方を AND として扱う）。
    pub(super) fn augment_node_pattern(
        np: &NodePattern,
        eqs: &[(String, String, Expression)],
    ) -> NodePattern {
        let var = match &np.variable {
            Some(v) => v,
            None => return np.clone(),
        };
        let mut properties = np.properties.clone();
        for (v, prop, val) in eqs {
            if v == var {
                properties
                    .entry(prop.clone())
                    .or_insert_with(|| val.clone());
            }
        }
        NodePattern {
            variable: np.variable.clone(),
            labels: np.labels.clone(),
            properties,
            properties_param: np.properties_param.clone(),
        }
    }

    pub(super) fn apply_with_clause(
        &self,
        with_clause: &WithClause,
        bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Determine effective column names (alias takes priority)
        let col_names: Vec<String> = with_clause
            .items
            .iter()
            .map(|item| {
                if let Some(ref alias) = item.alias {
                    alias.clone()
                } else {
                    match &item.expression {
                        ReturnItem::Alias(_, name) => name.clone(),
                        ReturnItem::Variable(v) => v.clone(),
                        ReturnItem::Property(v, p) => format!("{}.{}", v, p),
                        ReturnItem::Aggregate(agg) => self.aggregate_to_name(agg),
                        ReturnItem::Function(func) => self.function_to_name(func),
                        ReturnItem::All => "*".to_string(),
                        ReturnItem::Expr(e) => Self::expression_to_display(e),
                    }
                }
            })
            .collect();

        // Check if any item is an aggregate — if so, delegate to aggregated path
        let has_aggregation = with_clause
            .items
            .iter()
            .any(|item| Self::is_aggregate(&item.expression));

        if has_aggregation {
            // Build a temporary ReturnClause so we can reuse build_aggregated_result_set
            let temp_return_clause = ReturnClause {
                distinct: with_clause.distinct,
                items: with_clause
                    .items
                    .iter()
                    .map(|wi| {
                        // Wrap the expression in an alias if one was provided so that
                        // return_item_to_column_name produces the right column name.
                        if let Some(ref alias) = wi.alias {
                            ReturnItem::Alias(Box::new(wi.expression.clone()), alias.clone())
                        } else {
                            wi.expression.clone()
                        }
                    })
                    .collect(),
                order_by: with_clause.order_by.clone(),
                skip: with_clause.skip.clone(),
                limit: with_clause.limit.clone(),
            };

            let result_set = self.build_aggregated_result_set(&temp_return_clause, &bindings)?;

            // Convert each result row back to Bindings
            let result: Vec<Bindings> = result_set
                .rows
                .into_iter()
                .map(|row| {
                    let mut new_binding = Bindings::new();
                    for (col, value) in col_names.iter().zip(row.columns) {
                        match value {
                            Value::Node(id) | Value::NodeData { id, .. } => {
                                new_binding.insert(Arc::from(col.as_str()), BindingValue::Node(id));
                            }
                            Value::Path { nodes, edges } => {
                                new_binding.insert(
                                    Arc::from(col.as_str()),
                                    BindingValue::Path { nodes, edges },
                                );
                            }
                            other => {
                                new_binding
                                    .insert(Arc::from(col.as_str()), BindingValue::Scalar(other));
                            }
                        }
                    }
                    new_binding
                })
                .collect();

            return Ok(result);
        }

        // No aggregation: project bindings row-by-row
        let mut result: Vec<Bindings> = Vec::new();
        let mut keys: Vec<Vec<Value>> = Vec::new();
        let mut order_slots: Option<Vec<Option<usize>>> = None;

        for binding in &bindings {
            let mut new_binding = Bindings::new();
            let mut projected = Vec::with_capacity(with_clause.items.len());

            for (item, var_name) in with_clause.items.iter().zip(col_names.iter()) {
                let value = self.evaluate_return_item(&item.expression, binding)?;
                if with_clause.order_by.is_some() {
                    projected.push(value.clone());
                }

                // `WITH x` / `WITH x AS y` passes the binding through unchanged, so a
                // relationship stays a relationship (a `Value` cannot represent one).
                if let ReturnItem::Variable(v) = &item.expression
                    && let Some(bound) = binding.get(v.as_str())
                {
                    new_binding.insert(Arc::from(var_name.as_str()), bound.clone());
                    continue;
                }

                // Convert Value back to BindingValue for the new binding
                match value {
                    Value::Node(id) | Value::NodeData { id, .. } => {
                        new_binding.insert(Arc::from(var_name.as_str()), BindingValue::Node(id));
                    }
                    Value::Path { nodes, edges } => {
                        new_binding.insert(
                            Arc::from(var_name.as_str()),
                            BindingValue::Path { nodes, edges },
                        );
                    }
                    other => {
                        new_binding
                            .insert(Arc::from(var_name.as_str()), BindingValue::Scalar(other));
                    }
                }
            }

            if let Some(ref order_by) = with_clause.order_by {
                let row = Row { columns: projected };
                // ORDER BY may use both the incoming variables and the WITH
                // aliases (e.g. `WITH r AS rel ORDER BY rel.since`).
                let mut scope = binding.clone();
                scope.extend(new_binding.iter().map(|(k, v)| (k.clone(), v.clone())));
                let slots =
                    order_slots.get_or_insert_with(|| self.order_key_slots(order_by, &col_names));
                keys.push(self.order_keys(order_by, slots, &row, Some(&scope))?);
            }
            result.push(new_binding);
        }

        // Apply DISTINCT if needed (keeping ORDER BY keys aligned)
        if with_clause.distinct {
            let mut seen = std::collections::HashSet::new();
            let has_keys = !keys.is_empty();
            let mut kept = Vec::new();
            let mut kept_keys = Vec::new();
            let mut keys_iter = keys.into_iter();
            for binding in result {
                let key = has_keys.then(|| keys_iter.next().unwrap_or_default());
                if seen.insert(Self::bindings_key(&binding)) {
                    kept.push(binding);
                    kept_keys.extend(key);
                }
            }
            result = kept;
            keys = kept_keys;
        }

        // Apply ORDER BY (was previously ignored for non-aggregating WITH)
        if let Some(ref order_by) = with_clause.order_by {
            let mut pairs: Vec<(Vec<Value>, Bindings)> = keys.into_iter().zip(result).collect();
            pairs.sort_by(|a, b| self.compare_keys(&a.0, &b.0, order_by));
            result = pairs.into_iter().map(|(_, b)| b).collect();
        }

        // Apply SKIP
        if let Some(ref skip_expr) = with_clause.skip {
            let skip = self.resolve_skip_limit(skip_expr)? as usize;
            result = result.into_iter().skip(skip).collect();
        }

        // Apply LIMIT
        if let Some(ref limit_expr) = with_clause.limit {
            let limit = self.resolve_skip_limit(limit_expr)? as usize;
            result = result.into_iter().take(limit).collect();
        }

        Ok(result)
    }

    /// DISTINCT 判定用のバインディングのキー。`HashMap` の走査順はインスタンスごとに
    /// 異なるため（RandomState）、Debug 表示をそのまま使うと同じ内容でも別キーになる。
    /// 変数名でソートしてから文字列化する。
    pub(super) fn bindings_key(binding: &Bindings) -> String {
        let mut entries: Vec<_> = binding.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        format!("{:?}", entries)
    }

    pub(super) fn aggregate_to_name(&self, agg: &AggregateFunction) -> String {
        match agg {
            AggregateFunction::Count(_) => "count".to_string(),
            AggregateFunction::Sum(_) => "sum".to_string(),
            AggregateFunction::Avg(_) => "avg".to_string(),
            AggregateFunction::Min(_) => "min".to_string(),
            AggregateFunction::Max(_) => "max".to_string(),
            AggregateFunction::Collect(_) => "collect".to_string(),
            AggregateFunction::PercentileCont(..) => "percentileCont".to_string(),
            AggregateFunction::PercentileDisc(..) => "percentileDisc".to_string(),
            AggregateFunction::StDev(_) => "stDev".to_string(),
            AggregateFunction::StDevP(_) => "stDevP".to_string(),
            AggregateFunction::CountDistinct(_) => "count".to_string(),
            AggregateFunction::SumDistinct(_) => "sum".to_string(),
            AggregateFunction::AvgDistinct(_) => "avg".to_string(),
            AggregateFunction::CollectDistinct(_) => "collect".to_string(),
        }
    }

    pub(super) fn expression_to_display(expr: &Expression) -> String {
        match expr {
            Expression::Variable(v) => v.clone(),
            Expression::Property(v, p) => format!("{}.{}", v, p),
            Expression::Literal(lit) => match lit {
                Literal::Null => "null".to_string(),
                Literal::Bool(b) => b.to_string(),
                Literal::Int(n) => n.to_string(),
                Literal::Float(n) => n.to_string(),
                Literal::String(s) => format!("\"{}\"", s),
            },
            _ => "expr".to_string(),
        }
    }

    pub(super) fn function_to_name(&self, func: &ScalarFunction) -> String {
        match func {
            ScalarFunction::Nodes(_) => "nodes".to_string(),
            ScalarFunction::Relationships(_) => "relationships".to_string(),
            ScalarFunction::Length(_) => "length".to_string(),
            ScalarFunction::ShortestPath { .. } => "shortestPath".to_string(),
            ScalarFunction::AllShortestPaths { .. } => "allShortestPaths".to_string(),
            ScalarFunction::Trim(_) => "trim".to_string(),
            ScalarFunction::LTrim(_) => "ltrim".to_string(),
            ScalarFunction::RTrim(_) => "rtrim".to_string(),
            ScalarFunction::ToLower(_) => "toLower".to_string(),
            ScalarFunction::ToUpper(_) => "toUpper".to_string(),
            ScalarFunction::Reverse(_) => "reverse".to_string(),
            ScalarFunction::ToString(_) => "toString".to_string(),
            ScalarFunction::Size(_) => "size".to_string(),
            ScalarFunction::Left(..) => "left".to_string(),
            ScalarFunction::Right(..) => "right".to_string(),
            ScalarFunction::Substring(..) => "substring".to_string(),
            ScalarFunction::Split(..) => "split".to_string(),
            ScalarFunction::Replace(..) => "replace".to_string(),
            ScalarFunction::Abs(_) => "abs".to_string(),
            ScalarFunction::Ceil(_) => "ceil".to_string(),
            ScalarFunction::Floor(_) => "floor".to_string(),
            ScalarFunction::Round(..) => "round".to_string(),
            ScalarFunction::Sign(_) => "sign".to_string(),
            ScalarFunction::Rand => "rand".to_string(),
            ScalarFunction::IsNaN(_) => "isNaN".to_string(),
            ScalarFunction::Log(_) => "log".to_string(),
            ScalarFunction::Log10(_) => "log10".to_string(),
            ScalarFunction::Sqrt(_) => "sqrt".to_string(),
            ScalarFunction::Math(f, _) => f.name().to_string(),
            ScalarFunction::Math2(f, ..) => f.name().to_string(),
            ScalarFunction::E => "e".to_string(),
            ScalarFunction::Pi => "pi".to_string(),
            ScalarFunction::Id(_) => "id".to_string(),
            ScalarFunction::ElementId(_) => "elementId".to_string(),
            ScalarFunction::Type(_) => "type".to_string(),
            ScalarFunction::StartNode(_) => "startNode".to_string(),
            ScalarFunction::EndNode(_) => "endNode".to_string(),
            ScalarFunction::Labels(_) => "labels".to_string(),
            ScalarFunction::Properties(_) => "properties".to_string(),
            ScalarFunction::Keys(_) => "keys".to_string(),
            ScalarFunction::Coalesce(_) => "coalesce".to_string(),
            ScalarFunction::NullIf(..) => "nullIf".to_string(),
            ScalarFunction::ToBoolean(_) => "toBoolean".to_string(),
            ScalarFunction::ToFloat(_) => "toFloat".to_string(),
            ScalarFunction::ToInteger(_) => "toInteger".to_string(),
            ScalarFunction::Timestamp => "timestamp".to_string(),
            ScalarFunction::RandomUUID => "randomUUID".to_string(),
            ScalarFunction::Head(_) => "head".to_string(),
            ScalarFunction::Last(_) => "last".to_string(),
            ScalarFunction::Tail(_) => "tail".to_string(),
            ScalarFunction::Range(..) => "range".to_string(),
            ScalarFunction::Reduce { .. } => "reduce".to_string(),
            ScalarFunction::DateFunc(_) => "date".to_string(),
            ScalarFunction::DateTimeFunc(_) => "datetime".to_string(),
            ScalarFunction::DurationFunc(_) => "duration".to_string(),
        }
    }
}
