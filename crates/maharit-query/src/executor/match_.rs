//! パターン照合（ノード・パス・可変長・最短経路）と、プロパティによる絞り込み。

use super::*;

impl<'a> Executor<'a> {
    pub(super) fn execute_match_clause(
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
                    // No matches found - keep the original bindings and bind the
                    // clause's new variables to null.
                    let mut bindings = bindings;
                    for var in pattern_variables(&clause.patterns) {
                        if !bindings.contains_key(var.as_str()) {
                            bindings
                                .insert(Arc::from(var.as_str()), BindingValue::Scalar(Value::Null));
                        }
                    }
                    result.push(bindings);
                } else {
                    // Found matches - use them
                    result.extend(matches);
                }
            }

            Ok(result)
        } else {
            // Regular MATCH: process each input binding lazily through all patterns.
            // Processing one binding at a time reduces peak memory for multi-pattern
            // queries, since the intermediate expansion of one binding is discarded
            // before the next input binding is processed. This avoids the O(M^N) peak
            // that would occur if we fully expanded all bindings after each pattern step.
            let mut result = Vec::new();
            for b in current_bindings {
                let expanded = self.match_patterns_for_binding(&clause.patterns, b)?;
                result.extend(expanded);
            }
            Ok(result)
        }
    }

    pub(super) fn match_pattern(
        &self,
        pattern: &Pattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        match pattern {
            Pattern::Node(node_pattern) => self.match_node_pattern(node_pattern, current_bindings),
            Pattern::Path(path_pattern) => self.match_path_pattern(path_pattern, current_bindings),
        }
    }

    /// Chains a single binding through multiple patterns lazily.
    ///
    /// Unlike [`match_pattern`] (which processes all input bindings as a batch),
    /// this method threads one input binding through each pattern in sequence and
    /// short-circuits as soon as any pattern produces zero matches, avoiding
    /// unnecessary work and reducing peak memory usage for multi-pattern queries.
    pub(super) fn match_patterns_for_binding(
        &self,
        patterns: &[Pattern],
        initial: Bindings,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut matches = vec![initial];
        for pattern in patterns {
            if matches.is_empty() {
                break;
            }
            matches = self.match_pattern(pattern, matches)?;
        }
        Ok(matches)
    }

    /// スキャン候補を範囲索引で絞り込む（該当する範囲ヒントと索引がある場合）。
    ///
    /// 戻り値は「WHERE を満たすノードを必ず含むスーパーセット」。最終的な正当性は
    /// `node_matches_pattern` と WHERE の retain が保証するため、多少広くても問題ない。
    /// int/float の両範囲索引を union するので、プロパティの数値型が混在していても
    /// 取りこぼさない。
    pub(super) fn range_index_candidates(&self, pattern: &NodePattern) -> Option<Vec<NodeId>> {
        // 数値 f を i64 に安全に丸める（範囲外はクランプ）。
        fn floor_i64(f: f64) -> i64 {
            if f >= i64::MAX as f64 {
                i64::MAX
            } else if f <= i64::MIN as f64 {
                i64::MIN
            } else {
                f.floor() as i64
            }
        }
        fn ceil_i64(f: f64) -> i64 {
            if f >= i64::MAX as f64 {
                i64::MAX
            } else if f <= i64::MIN as f64 {
                i64::MIN
            } else {
                f.ceil() as i64
            }
        }

        let var = pattern.variable.as_deref()?;
        let label = pattern.labels.first()?;

        for (v, prop, op, val) in &self.range_hints {
            if v != var || !self.property_index.has_index(label, prop) {
                continue;
            }
            let f = match val {
                PropertyValue::Int(n) => *n as f64,
                PropertyValue::Float(x) => *x,
                _ => continue,
            };
            let lower_open = matches!(op, BinaryOp::Gt | BinaryOp::Gte);
            let upper_open = matches!(op, BinaryOp::Lt | BinaryOp::Lte);

            let mut ids: Vec<NodeId> = Vec::new();
            if lower_open {
                ids.extend(
                    self.property_index
                        .find_by_int_range(prop, floor_i64(f), i64::MAX),
                );
                ids.extend(self.property_index.find_by_float_range(prop, f, f64::MAX));
            } else if upper_open {
                ids.extend(
                    self.property_index
                        .find_by_int_range(prop, i64::MIN, ceil_i64(f)),
                );
                ids.extend(self.property_index.find_by_float_range(prop, f64::MIN, f));
            } else {
                continue;
            }

            ids.sort_unstable();
            ids.dedup();
            return Some(ids);
        }
        None
    }

    pub(super) fn match_node_pattern(
        &self,
        pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let mut result = Vec::new();

        // Candidate node set for the scan path, computed on first use and reused
        // across input bindings. It is built lazily because bound variables and
        // property-index lookups never scan: listing every node of the label
        // would then dominate the cost of a point lookup.
        //
        // Priority:
        // 1. A WHERE range predicate on an indexed numeric property → use the
        //    range index for a candidate superset (narrows the scan a lot).
        // 2. Otherwise, if the pattern has a label → enumerate only that label's
        //    nodes via the label index.
        // 3. Otherwise → all node ids.
        //
        // `node_matches_pattern` re-verifies labels/properties and the WHERE
        // retain re-checks the exact predicate, so a candidate *superset* is
        // always safe.
        let scan_candidates = std::cell::OnceCell::new();
        let scan_candidates = || -> &Vec<NodeId> {
            scan_candidates.get_or_init(|| {
                self.range_index_candidates(pattern).unwrap_or_else(|| {
                    match pattern.labels.first() {
                        Some(label) => self.graph_ref().nodes_by_label(label),
                        None => self.graph_ref().node_ids(),
                    }
                })
            })
        };

        for bindings in current_bindings {
            // Check if variable is already bound
            if let Some(var) = &pattern.variable
                && let Some(bound_value) = bindings.get(var.as_str())
            {
                if let Some(bound_id) = bound_value.as_node() {
                    // Variable already bound, check if it matches
                    if self.node_matches_pattern(bound_id, pattern, &bindings)? {
                        result.push(bindings);
                    }
                }
                continue;
            }

            // Try index-based lookup first: if the pattern has a label and at least one
            // property that is indexed and has a literal value, use the index to avoid
            // a full scan.
            //
            // The index holds every node, including ones an ACL session cannot
            // read. That is fine: index hits are only candidates, and
            // `node_matches_pattern` re-checks labels and property values
            // through `graph_ref()`, which is the ACL view when one is active.
            let mut used_index = false;
            if let Some(label) = pattern.labels.first() {
                'index_loop: for (prop_key, prop_expr) in &pattern.properties {
                    if self.property_index.has_index(label, prop_key)
                        && let Expression::Literal(lit) = prop_expr
                    {
                        let prop_val = PropertyValue::from(lit.clone());
                        let candidate_ids =
                            self.property_index.find_by_property(prop_key, &prop_val);
                        for node_id in candidate_ids {
                            if self.node_matches_pattern(node_id, pattern, &bindings)? {
                                let mut new_bindings = bindings.clone();
                                if let Some(var) = &pattern.variable {
                                    new_bindings.insert(
                                        Arc::from(var.as_str()),
                                        BindingValue::Node(node_id),
                                    );
                                }
                                result.push(new_bindings);
                            }
                        }
                        used_index = true;
                        break 'index_loop;
                    }
                }
            }

            if used_index {
                continue;
            }

            // Find matching nodes.
            // For large graphs use parallel filtering to determine which node IDs
            // match the label/property predicates. Property expression evaluation
            // only reads the graph, so sharing `&self` across threads is safe here
            // through the pattern closure.
            let all_node_ids = scan_candidates();
            if all_node_ids.len() >= PARALLEL_MATCH_THRESHOLD {
                // Phase 1 (parallel): determine which node IDs pass label + property filters.
                // Note: evaluate_expression needs &self; we wrap it in a closure that
                // captures an immutable `self` reference — safe because all reads are
                // independent (no mutation occurs during MATCH).
                let matching_ids: Vec<Result<Option<NodeId>, ExecuteError>> = all_node_ids
                    .par_iter()
                    .map(|&node_id| {
                        if self.node_matches_pattern(node_id, pattern, &bindings)? {
                            Ok(Some(node_id))
                        } else {
                            Ok(None)
                        }
                    })
                    .collect();

                // Phase 2 (sequential): assemble bindings for matched nodes.
                for res in matching_ids {
                    let node_id = match res? {
                        Some(id) => id,
                        None => continue,
                    };
                    let mut new_bindings = bindings.clone();
                    if let Some(var) = &pattern.variable {
                        new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(node_id));
                    }
                    result.push(new_bindings);
                }
            } else {
                // Sequential path for small graphs.
                for &node_id in all_node_ids {
                    if self.node_matches_pattern(node_id, pattern, &bindings)? {
                        let mut new_bindings = bindings.clone();
                        if let Some(var) = &pattern.variable {
                            new_bindings
                                .insert(Arc::from(var.as_str()), BindingValue::Node(node_id));
                        }
                        result.push(new_bindings);
                    }
                }
            }
        }

        Ok(result)
    }

    pub(super) fn match_path_pattern(
        &self,
        pattern: &PathPattern,
        current_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        // Each hop continues from the node matched by the previous hop, so every
        // node on the path needs a binding. Anonymous nodes (`()`, `(:Label)`)
        // get a hidden variable name that no query can spell, which is removed
        // again once the whole path has matched.
        // A path variable additionally needs every relationship bound.
        let has_anonymous = pattern.start.variable.is_none()
            || pattern.segments.iter().any(|s| s.node.variable.is_none())
            || (pattern.variable.is_some()
                && pattern.segments.iter().any(|s| s.edge.variable.is_none()));
        let named;
        let pattern = if has_anonymous {
            let mut p = pattern.clone();
            let mut n = 0;
            let mut fresh = || {
                n += 1;
                Some(format!("{ANON_VAR_PREFIX}{n}"))
            };
            if p.start.variable.is_none() {
                p.start.variable = fresh();
            }
            let bind_edges = p.variable.is_some();
            for segment in &mut p.segments {
                if segment.node.variable.is_none() {
                    segment.node.variable = fresh();
                }
                if bind_edges && segment.edge.variable.is_none() {
                    segment.edge.variable = fresh();
                }
            }
            named = p;
            &named
        } else {
            pattern
        };

        let mut bindings = self.match_node_pattern(&pattern.start, current_bindings)?;
        let mut prev = &pattern.start;
        for segment in &pattern.segments {
            bindings = match pattern.shortest {
                Some(kind) => self.match_shortest_segment(segment, prev, bindings, kind)?,
                None => self.match_segment(segment, prev, bindings)?,
            };
            prev = &segment.node;
        }

        if let Some(path_var) = &pattern.variable {
            for b in &mut bindings {
                let path = Self::assemble_path(pattern, b)?;
                b.insert(Arc::from(path_var.as_str()), path);
            }
        }

        if has_anonymous {
            for b in &mut bindings {
                b.retain(|k, _| !k.starts_with(ANON_VAR_PREFIX));
            }
        }
        Ok(bindings)
    }

    /// Build the value of a path variable from the node / relationship
    /// bindings of a fully matched (and fully named) path pattern.
    pub(super) fn assemble_path(
        pattern: &PathPattern,
        b: &Bindings,
    ) -> Result<BindingValue, ExecuteError> {
        let node_of = |var: &Option<String>| {
            var.as_deref()
                .and_then(|v| b.get(v))
                .and_then(|v| v.as_node())
                .ok_or_else(|| ExecuteError::TypeError("path node is not bound".to_string()))
        };
        let mut nodes = vec![node_of(&pattern.start.variable)?];
        let mut edges = Vec::new();
        for segment in &pattern.segments {
            match segment.edge.variable.as_deref().and_then(|v| b.get(v)) {
                Some(BindingValue::Edge(id)) => edges.push(*id),
                // Variable-length hop: its nodes include the hop's start node.
                Some(BindingValue::Path {
                    nodes: hop_nodes,
                    edges: hop_edges,
                }) => {
                    edges.extend(hop_edges.iter().copied());
                    nodes.extend(hop_nodes.iter().skip(1).copied());
                    continue;
                }
                _ => {
                    return Err(ExecuteError::TypeError(
                        "path relationship is not bound".to_string(),
                    ));
                }
            }
            nodes.push(node_of(&segment.node.variable)?);
        }
        Ok(BindingValue::Path { nodes, edges })
    }

    pub(super) fn match_segment(
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

    pub(super) fn match_single_hop(
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
            .get(prev_var.as_str())
            .and_then(|v| v.as_node())
            .ok_or_else(|| ExecuteError::UndefinedVariable(prev_var.clone()))?;

        // Get edges from previous node
        let edges = self.adjacent_edges(prev_id, &segment.edge);

        for edge in edges {
            if !self.edge_matches_properties(edge.id, &segment.edge, bindings)? {
                continue;
            }

            // A relationship variable bound earlier only matches that relationship.
            if let Some(bound) = segment
                .edge
                .variable
                .as_deref()
                .and_then(|v| bindings.get(v))
                && !matches!(bound, BindingValue::Edge(id) if *id == edge.id)
            {
                continue;
            }

            // Get the other node
            let next_id = self.get_next_node(prev_id, edge, segment.edge.direction);

            // Check if next node matches pattern
            if self.node_matches_pattern(next_id, &segment.node, bindings)? {
                let mut new_bindings = bindings.clone();

                if let Some(var) = &segment.node.variable {
                    new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(next_id));
                }
                if let Some(var) = &segment.edge.variable {
                    new_bindings.insert(Arc::from(var.as_str()), BindingValue::Edge(edge.id));
                }

                result.push(new_bindings);
            }
        }

        Ok(result)
    }

    pub(super) fn match_variable_length_segment(
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
                .get(prev_var.as_str())
                .and_then(|v| v.as_node())
                .ok_or_else(|| ExecuteError::UndefinedVariable(prev_var.clone()))?;

            // BFS to find all reachable nodes within the range
            // Track: (current_node, depth, path_edges, visited_nodes)
            let mut visited_paths: Vec<(NodeId, u32, Vec<u64>, Vec<NodeId>)> =
                vec![(start_id, 0, vec![], vec![start_id])];
            let mut found_paths: Vec<(NodeId, Vec<u64>, Vec<NodeId>)> = Vec::new();

            while let Some((current_id, depth, path_edges, visited_nodes)) = visited_paths.pop() {
                // If within range and matches target pattern, add to results
                if depth >= range.min
                    && self.node_matches_pattern(current_id, &segment.node, &bindings)?
                {
                    found_paths.push((current_id, path_edges.clone(), visited_nodes.clone()));
                }

                // Don't explore further if at max depth
                if depth >= max_depth {
                    continue;
                }

                // Get edges and explore neighbors
                let edges = self.adjacent_edges(current_id, &segment.edge);

                for edge in edges {
                    if !self.edge_matches_properties(edge.id, &segment.edge, &bindings)? {
                        continue;
                    }

                    let next_id = self.get_next_node(current_id, edge, segment.edge.direction);

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
                    new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(end_id));
                }

                // Bind the path variable (edge list) if specified
                if let Some(var) = &segment.edge.variable {
                    new_bindings.insert(
                        Arc::from(var.as_str()),
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

    /// `shortestPath` / `allShortestPaths` の 1 セグメントを BFS で解く。
    ///
    /// 始点ごとに幅優先で探索し、終点パターンに合うノードそれぞれについて
    /// 最短経路（`All` なら同じ長さの経路すべて）を束縛する。探索は始点からの
    /// 距離で層ごとに進むため、各ノードに最初に届いた層がその最短距離になる。
    pub(super) fn match_shortest_segment(
        &self,
        segment: &PathSegment,
        prev_pattern: &NodePattern,
        current_bindings: Vec<Bindings>,
        kind: ShortestPathKind,
    ) -> Result<Vec<Bindings>, ExecuteError> {
        let (min, max) = match &segment.edge.length_range {
            Some(range) => (range.min, range.max),
            None => (1, Some(1)),
        };
        if min > 1 {
            return Err(ExecuteError::TypeError(
                "shortestPath() supports only a minimum length of 0 or 1".to_string(),
            ));
        }
        let prev_var = prev_pattern
            .variable
            .as_ref()
            .ok_or_else(|| ExecuteError::TypeError("path pattern requires variable".to_string()))?;

        let mut result = Vec::new();
        for bindings in current_bindings {
            let start_id = bindings
                .get(prev_var.as_str())
                .and_then(|v| v.as_node())
                .ok_or_else(|| ExecuteError::UndefinedVariable(prev_var.clone()))?;
            // When the end node is already bound only its layer needs to be reached.
            let bound_end = segment
                .node
                .variable
                .as_deref()
                .and_then(|v| bindings.get(v))
                .and_then(|v| v.as_node());

            // dist[n]: hop count from start; preds[n]: (previous node, edge) on a shortest path.
            let mut dist: HashMap<NodeId, u32> = HashMap::from([(start_id, 0)]);
            let mut preds: HashMap<NodeId, Vec<(NodeId, u64)>> = HashMap::new();
            let mut targets = Vec::new();
            let mut frontier = vec![start_id];
            let mut depth = 0u32;
            while !frontier.is_empty() {
                for &node in &frontier {
                    if depth >= min && self.node_matches_pattern(node, &segment.node, &bindings)? {
                        targets.push(node);
                    }
                }
                if bound_end.is_some_and(|end| dist.contains_key(&end))
                    || max.is_some_and(|m| depth >= m)
                {
                    break;
                }
                let mut next = Vec::new();
                for &node in &frontier {
                    for edge in self.adjacent_edges(node, &segment.edge) {
                        if !self.edge_matches_properties(edge.id, &segment.edge, &bindings)? {
                            continue;
                        }
                        let to = self.get_next_node(node, edge, segment.edge.direction);
                        match dist.get(&to) {
                            None => {
                                dist.insert(to, depth + 1);
                                preds.insert(to, vec![(node, edge.id)]);
                                next.push(to);
                            }
                            Some(&d) if d == depth + 1 && kind == ShortestPathKind::All => {
                                preds.entry(to).or_default().push((node, edge.id));
                            }
                            Some(_) => {}
                        }
                    }
                }
                frontier = next;
                depth += 1;
            }

            for end_id in targets {
                for (nodes, edges) in Self::shortest_paths_to(&preds, start_id, end_id, kind) {
                    let mut new_bindings = bindings.clone();
                    if let Some(var) = &segment.node.variable {
                        new_bindings.insert(Arc::from(var.as_str()), BindingValue::Node(end_id));
                    }
                    if let Some(var) = &segment.edge.variable {
                        new_bindings
                            .insert(Arc::from(var.as_str()), BindingValue::Path { nodes, edges });
                    }
                    result.push(new_bindings);
                }
            }
        }
        Ok(result)
    }

    /// BFS の先行リストから `start` → `end` の最短経路を復元する
    /// （`Single` なら 1 本、`All` ならすべて）。戻り値は (ノード列, エッジ列)。
    pub(super) fn shortest_paths_to(
        preds: &HashMap<NodeId, Vec<(NodeId, u64)>>,
        start: NodeId,
        end: NodeId,
        kind: ShortestPathKind,
    ) -> Vec<(Vec<NodeId>, Vec<u64>)> {
        // Partial paths are built backwards from `end`.
        let mut done = Vec::new();
        let mut stack = vec![(vec![end], Vec::new())];
        while let Some((mut nodes, mut edges)) = stack.pop() {
            let Some(&head) = nodes.last() else {
                continue;
            };
            if head == start {
                nodes.reverse();
                edges.reverse();
                done.push((nodes, edges));
                if kind == ShortestPathKind::Single {
                    break;
                }
                continue;
            }
            let Some(ps) = preds.get(&head) else { continue };
            for &(prev, edge) in ps {
                let mut n = nodes.clone();
                let mut e = edges.clone();
                n.push(prev);
                e.push(edge);
                stack.push((n, e));
            }
        }
        done
    }

    /// `node_id` に接続し、パターンの向きと種別に合うリレーションシップ（ID と両端）。
    /// 向きが無指定なら出る向き → 入る向きの順に並ぶ（自己ループは両方に現れる）。
    pub(super) fn adjacent_edges(&self, node_id: NodeId, pattern: &EdgePattern) -> Vec<EdgeEnds> {
        let graph = self.graph_ref();
        let types = &pattern.edge_types;
        let mut edges = Vec::new();
        match pattern.direction {
            EdgeDirection::Outgoing => graph.adjacent_edges(node_id, true, types, &mut edges),
            EdgeDirection::Incoming => graph.adjacent_edges(node_id, false, types, &mut edges),
            EdgeDirection::Both => {
                graph.adjacent_edges(node_id, true, types, &mut edges);
                graph.adjacent_edges(node_id, false, types, &mut edges);
            }
        }
        edges
    }

    pub(super) fn get_next_node(
        &self,
        current_id: NodeId,
        edge: EdgeEnds,
        direction: EdgeDirection,
    ) -> NodeId {
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

    pub(super) fn node_matches_pattern(
        &self,
        node_id: NodeId,
        pattern: &NodePattern,
        bindings: &Bindings,
    ) -> Result<bool, ExecuteError> {
        // A variable bound earlier (previous pattern, WITH, subquery import)
        // only matches that same node.
        if let Some(bound) = pattern.variable.as_deref().and_then(|v| bindings.get(v))
            && bound.as_node() != Some(node_id)
        {
            return Ok(false);
        }

        let backend = self.graph_ref();

        // Check labels without cloning the whole node. A node must have ALL
        // labels specified in the pattern (AND). When the pattern has labels,
        // this also confirms the node exists.
        if !pattern.labels.is_empty() {
            if !backend.node_has_all_labels(node_id, &pattern.labels) {
                return Ok(false);
            }
        } else if pattern.properties.is_empty()
            && pattern.properties_param.is_none()
            && !backend.contains_node(node_id)
        {
            // No labels and no properties: still require the node to exist.
            return Ok(false);
        }

        // Check properties, cloning only the individual property values needed
        // rather than the entire node.
        for (key, expected_expr) in &pattern.properties {
            match backend.get_node_property(node_id, key) {
                Some(actual) => {
                    let expected_val = self.evaluate_expression(expected_expr, bindings)?;
                    if !self.property_value_matches(&actual, &expected_val) {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }
        for (key, expected_expr) in self.param_property_exprs(&pattern.properties_param)? {
            match backend.get_node_property(node_id, &key) {
                Some(actual) => {
                    let expected_val = self.evaluate_expression(&expected_expr, bindings)?;
                    if !self.property_value_matches(&actual, &expected_val) {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }

        Ok(true)
    }

    /// パターンの `$props` を、マップの各キーを参照する式（`$props['key']`）の
    /// 並びに展開する。パラメータが無ければ空。マップ以外ならエラー。
    pub(super) fn param_property_exprs(
        &self,
        param: &Option<String>,
    ) -> Result<Vec<(String, Expression)>, ExecuteError> {
        let Some(name) = param else {
            return Ok(Vec::new());
        };
        match self.params.get(name) {
            Some(Value::Map(map)) => Ok(map
                .keys()
                .map(|k| {
                    (
                        k.clone(),
                        Expression::IndexAccess(
                            Box::new(Expression::Parameter(name.clone())),
                            Box::new(Expression::Literal(Literal::String(k.clone()))),
                        ),
                    )
                })
                .collect()),
            Some(other) => Err(ExecuteError::TypeError(format!(
                "parameter ${name} used as pattern properties must be a map, got {other:?}"
            ))),
            None => Err(ExecuteError::UndefinedVariable(format!("${name}"))),
        }
    }

    /// パターンのインラインプロパティと `$props` の展開をまとめて返す。
    pub(super) fn pattern_property_exprs(
        &self,
        properties: &HashMap<String, Expression>,
        param: &Option<String>,
    ) -> Result<Vec<(String, Expression)>, ExecuteError> {
        let mut out: Vec<(String, Expression)> = properties
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.extend(self.param_property_exprs(param)?);
        Ok(out)
    }

    /// リレーションシップがパターンのインラインプロパティ（`-[r {w: 1}]-`）を満たすか。
    pub(super) fn edge_matches_properties(
        &self,
        edge_id: EdgeId,
        pattern: &EdgePattern,
        bindings: &Bindings,
    ) -> Result<bool, ExecuteError> {
        if pattern.properties.is_empty() && pattern.properties_param.is_none() {
            return Ok(true);
        }
        let Some(edge) = self.graph_ref().get_edge(edge_id) else {
            return Ok(false);
        };
        for (key, expected_expr) in
            self.pattern_property_exprs(&pattern.properties, &pattern.properties_param)?
        {
            let Some(actual) = edge.properties.get(&key) else {
                return Ok(false);
            };
            let expected = self.evaluate_expression(&expected_expr, bindings)?;
            if !self.property_value_matches(actual, &expected) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn property_value_matches(&self, actual: &PropertyValue, expected: &Value) -> bool {
        match (actual, expected) {
            (PropertyValue::Null, Value::Null) => true,
            (PropertyValue::Bool(a), Value::Bool(e)) => a == e,
            (PropertyValue::Int(a), Value::Int(e)) => a == e,
            (PropertyValue::Float(a), Value::Float(e)) => (a - e).abs() < f64::EPSILON,
            (PropertyValue::Int(a), Value::Float(e)) => (*a as f64 - e).abs() < f64::EPSILON,
            (PropertyValue::Float(a), Value::Int(e)) => (a - *e as f64).abs() < f64::EPSILON,
            (PropertyValue::String(a), Value::String(e)) => a == e,
            // リスト・日付など: 値に直して比べる
            (PropertyValue::List(_), Value::List(_)) => {
                self.cypher_equals(&Value::from(actual), expected) == Some(true)
            }
            _ => false,
        }
    }
}
