//! スカラー関数の評価（文字列・数値・リスト・時間型・パスなど）。

use super::*;

impl<'a> Executor<'a> {
    pub(super) fn evaluate_scalar_function(
        &self,
        func: &ScalarFunction,
        bindings: &Bindings,
    ) -> Result<Value, ExecuteError> {
        match func {
            ScalarFunction::Nodes(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    if let BindingValue::Path { nodes, .. } = binding_value {
                        // Return list of node data
                        let node_values: Vec<Value> = nodes
                            .iter()
                            .map(|&node_id| {
                                if let Some(node) = self.graph_ref().get_node(node_id) {
                                    Value::NodeData {
                                        id: node_id,
                                        labels: node.labels.clone(),
                                        properties: Arc::clone(&node.properties),
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    if let BindingValue::Path { edges, .. } = binding_value {
                        // Return list of edge IDs (or edge data if available)
                        let edge_values: Vec<Value> =
                            edges.iter().map(|&edge_id| Value::Edge(edge_id)).collect();
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
                if let Some(binding_value) = bindings.get(var.as_str()) {
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
                    .get(start.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(start.clone()))?;
                let end_id = bindings
                    .get(end.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(end.clone()))?;

                if let Some(path) = traversal::shortest_path(self.graph_ref(), start_id, end_id) {
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
                    .get(start.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(start.clone()))?;
                let end_id = bindings
                    .get(end.as_str())
                    .and_then(|v| v.as_node())
                    .ok_or_else(|| ExecuteError::UndefinedVariable(end.clone()))?;

                let paths = traversal::all_shortest_paths(self.graph_ref(), start_id, end_id);
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
            ScalarFunction::Trim(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.trim().to_string())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "trim() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::LTrim(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.trim_start().to_string())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "ltrim() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::RTrim(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.trim_end().to_string())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "rtrim() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::ToLower(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.to_lowercase())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "toLower() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::ToUpper(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.to_uppercase())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "toUpper() requires a string".to_string(),
                    )),
                }
            }
            ScalarFunction::Reverse(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s.chars().rev().collect())),
                    Value::List(items) => Ok(Value::List(items.into_iter().rev().collect())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "reverse() requires a string or list".to_string(),
                    )),
                }
            }
            ScalarFunction::ToString(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::String(s)),
                    Value::Int(n) => Ok(Value::String(format!("{}", n))),
                    Value::Float(n) => Ok(Value::String(format!("{}", n))),
                    Value::Bool(b) => {
                        Ok(Value::String(if b { "true" } else { "false" }.to_string()))
                    }
                    Value::Null => Ok(Value::String("null".to_string())),
                    _ => Err(ExecuteError::TypeError(
                        "toString() unsupported type".to_string(),
                    )),
                }
            }
            ScalarFunction::Size(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => Ok(Value::Int(s.chars().count() as i64)),
                    Value::List(items) => Ok(Value::Int(items.len() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "size() requires a string or list".to_string(),
                    )),
                }
            }
            ScalarFunction::Left(str_expr, len_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let len_val = self.evaluate_expression(len_expr, bindings)?;
                match (&s_val, &len_val) {
                    (Value::String(s), Value::Int(len)) => {
                        let len = *len as usize;
                        Ok(Value::String(s.chars().take(len).collect()))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "left() requires (string, int)".to_string(),
                    )),
                }
            }
            ScalarFunction::Right(str_expr, len_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let len_val = self.evaluate_expression(len_expr, bindings)?;
                match (&s_val, &len_val) {
                    (Value::String(s), Value::Int(len)) => {
                        let len = *len as usize;
                        let chars: Vec<char> = s.chars().collect();
                        let start = chars.len().saturating_sub(len);
                        Ok(Value::String(chars[start..].iter().collect()))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "right() requires (string, int)".to_string(),
                    )),
                }
            }
            ScalarFunction::Substring(str_expr, start_expr, len_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let start_val = self.evaluate_expression(start_expr, bindings)?;
                let len_val = len_expr
                    .as_ref()
                    .map(|e| self.evaluate_expression(e, bindings))
                    .transpose()?;
                match (&s_val, &start_val) {
                    (Value::String(s), Value::Int(start)) => {
                        let start = *start as usize;
                        let chars: Vec<char> = s.chars().collect();
                        if start >= chars.len() {
                            return Ok(Value::String(String::new()));
                        }
                        match len_val {
                            Some(Value::Int(len)) => {
                                let len = len as usize;
                                Ok(Value::String(chars[start..].iter().take(len).collect()))
                            }
                            None => Ok(Value::String(chars[start..].iter().collect())),
                            Some(Value::Null) => Ok(Value::Null),
                            _ => Err(ExecuteError::TypeError(
                                "substring() length must be an integer".to_string(),
                            )),
                        }
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "substring() requires (string, int)".to_string(),
                    )),
                }
            }
            ScalarFunction::Split(str_expr, delim_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let d_val = self.evaluate_expression(delim_expr, bindings)?;
                match (&s_val, &d_val) {
                    (Value::String(s), Value::String(delim)) => {
                        let parts: Vec<Value> = s
                            .split(delim.as_str())
                            .map(|p| Value::String(p.to_string()))
                            .collect();
                        Ok(Value::List(parts))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "split() requires (string, string)".to_string(),
                    )),
                }
            }
            ScalarFunction::Replace(str_expr, search_expr, rep_expr) => {
                let s_val = self.evaluate_expression(str_expr, bindings)?;
                let search_val = self.evaluate_expression(search_expr, bindings)?;
                let rep_val = self.evaluate_expression(rep_expr, bindings)?;
                match (&s_val, &search_val, &rep_val) {
                    (Value::String(s), Value::String(search), Value::String(rep)) => {
                        Ok(Value::String(s.replace(search.as_str(), rep.as_str())))
                    }
                    (Value::Null, _, _) | (_, Value::Null, _) | (_, _, Value::Null) => {
                        Ok(Value::Null)
                    }
                    _ => Err(ExecuteError::TypeError(
                        "replace() requires (string, string, string)".to_string(),
                    )),
                }
            }
            ScalarFunction::Abs(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => n
                        .checked_abs()
                        .map(Value::Int)
                        .ok_or_else(|| ExecuteError::TypeError("integer overflow".to_string())),
                    Value::Float(n) => Ok(Value::Float(n.abs())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "abs() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Ceil(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n)),
                    Value::Float(n) => Ok(Value::Int(n.ceil() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "ceil() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Floor(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n)),
                    Value::Float(n) => Ok(Value::Int(n.floor() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "floor() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Round(expr, precision_expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => {
                        if precision_expr.is_some() {
                            Ok(Value::Float(n as f64))
                        } else {
                            Ok(Value::Int(n))
                        }
                    }
                    Value::Float(n) => {
                        if let Some(p_expr) = precision_expr {
                            let p_val = self.evaluate_expression(p_expr, bindings)?;
                            match p_val {
                                Value::Int(p) => {
                                    let factor = 10f64.powi(p as i32);
                                    Ok(Value::Float((n * factor).round() / factor))
                                }
                                Value::Null => Ok(Value::Null),
                                _ => Err(ExecuteError::TypeError(
                                    "round() precision must be an integer".to_string(),
                                )),
                            }
                        } else {
                            Ok(Value::Int(n.round() as i64))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "round() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Sign(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n.signum())),
                    Value::Float(n) => Ok(Value::Int(n.signum() as i64)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "sign() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Rand => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                use std::time::SystemTime;
                let mut hasher = DefaultHasher::new();
                SystemTime::now().hash(&mut hasher);
                std::thread::current().id().hash(&mut hasher);
                let hash = hasher.finish();
                let result = (hash as f64) / (u64::MAX as f64);
                Ok(Value::Float(result))
            }
            ScalarFunction::IsNaN(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Float(n) => Ok(Value::Bool(n.is_nan())),
                    Value::Int(_) => Ok(Value::Bool(false)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "isNaN() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Log(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Float((n as f64).ln())),
                    Value::Float(n) => Ok(Value::Float(n.ln())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "log() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Log10(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Float((n as f64).log10())),
                    Value::Float(n) => Ok(Value::Float(n.log10())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "log10() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Sqrt(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Float((n as f64).sqrt())),
                    Value::Float(n) => Ok(Value::Float(n.sqrt())),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "sqrt() requires a numeric value".to_string(),
                    )),
                }
            }
            ScalarFunction::Math(f, expr) => match self.evaluate_expression(expr, bindings)? {
                Value::Int(n) => Ok(Value::Float(f.apply(n as f64))),
                Value::Float(n) => Ok(Value::Float(f.apply(n))),
                Value::Null => Ok(Value::Null),
                _ => Err(ExecuteError::TypeError(format!(
                    "{}() requires a numeric value",
                    f.name()
                ))),
            },
            ScalarFunction::Math2(f, a, b) => {
                let as_f64 = |v: Value| match v {
                    Value::Int(n) => Ok(Some(n as f64)),
                    Value::Float(n) => Ok(Some(n)),
                    Value::Null => Ok(None),
                    _ => Err(ExecuteError::TypeError(format!(
                        "{}() requires numeric values",
                        f.name()
                    ))),
                };
                let a = as_f64(self.evaluate_expression(a, bindings)?)?;
                let b = as_f64(self.evaluate_expression(b, bindings)?)?;
                match (a, b) {
                    (Some(a), Some(b)) => Ok(Value::Float(f.apply(a, b))),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::E => Ok(Value::Float(std::f64::consts::E)),
            ScalarFunction::Pi => Ok(Value::Float(std::f64::consts::PI)),
            ScalarFunction::Id(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(id) => Ok(Value::Int(*id as i64)),
                        BindingValue::Edge(id) => Ok(Value::Int(*id as i64)),
                        _ => Err(ExecuteError::TypeError(
                            "id() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::ElementId(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(id) => Ok(Value::String(format!("node:{}", id))),
                        BindingValue::Edge(id) => Ok(Value::String(format!("edge:{}", id))),
                        _ => Err(ExecuteError::TypeError(
                            "elementId() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Type(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                Ok(Value::String(edge.label.clone()))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "type() requires an edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::StartNode(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let node_id = edge.from;
                                if let Some(node) = self.graph_ref().get_node(node_id) {
                                    Ok(Value::NodeData {
                                        id: node_id,
                                        labels: node.labels.clone(),
                                        properties: Arc::clone(&node.properties),
                                    })
                                } else {
                                    Ok(Value::Node(node_id))
                                }
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "startNode() requires an edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::EndNode(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let node_id = edge.to;
                                if let Some(node) = self.graph_ref().get_node(node_id) {
                                    Ok(Value::NodeData {
                                        id: node_id,
                                        labels: node.labels.clone(),
                                        properties: Arc::clone(&node.properties),
                                    })
                                } else {
                                    Ok(Value::Node(node_id))
                                }
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "endNode() requires an edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Labels(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                let labels: Vec<Value> = node
                                    .labels
                                    .iter()
                                    .map(|l| Value::String(l.clone()))
                                    .collect();
                                Ok(Value::List(labels))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "labels() requires a node variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Properties(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                let props: Vec<Value> = node
                                    .properties
                                    .iter()
                                    .map(|(k, v)| {
                                        Value::List(vec![Value::String(k.clone()), Value::from(v)])
                                    })
                                    .collect();
                                Ok(Value::List(props))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let props: Vec<Value> = edge
                                    .properties
                                    .iter()
                                    .map(|(k, v)| {
                                        Value::List(vec![Value::String(k.clone()), Value::from(v)])
                                    })
                                    .collect();
                                Ok(Value::List(props))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "properties() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Keys(var) => {
                if let Some(binding_value) = bindings.get(var.as_str()) {
                    match binding_value {
                        BindingValue::Node(node_id) => {
                            if let Some(node) = self.graph_ref().get_node(*node_id) {
                                let keys: Vec<Value> = node
                                    .properties
                                    .keys()
                                    .map(|k| Value::String(k.clone()))
                                    .collect();
                                Ok(Value::List(keys))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        BindingValue::Edge(edge_id) => {
                            if let Some(edge) = self.graph_ref().get_edge(*edge_id) {
                                let keys: Vec<Value> = edge
                                    .properties
                                    .keys()
                                    .map(|k| Value::String(k.clone()))
                                    .collect();
                                Ok(Value::List(keys))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                        _ => Err(ExecuteError::TypeError(
                            "keys() requires a node or edge variable".to_string(),
                        )),
                    }
                } else {
                    Err(ExecuteError::UndefinedVariable(var.clone()))
                }
            }
            ScalarFunction::Coalesce(exprs) => {
                for expr in exprs {
                    let val = self.evaluate_expression(expr, bindings)?;
                    if val != Value::Null {
                        return Ok(val);
                    }
                }
                Ok(Value::Null)
            }
            ScalarFunction::NullIf(expr1, expr2) => {
                let val1 = self.evaluate_expression(expr1, bindings)?;
                let val2 = self.evaluate_expression(expr2, bindings)?;
                if val1 == val2 {
                    Ok(Value::Null)
                } else {
                    Ok(val1)
                }
            }
            ScalarFunction::ToBoolean(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Bool(b) => Ok(Value::Bool(b)),
                    Value::String(s) => match s.to_lowercase().as_str() {
                        "true" => Ok(Value::Bool(true)),
                        "false" => Ok(Value::Bool(false)),
                        _ => Ok(Value::Null),
                    },
                    Value::Null => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::ToFloat(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Float(f) => Ok(Value::Float(f)),
                    Value::Int(n) => Ok(Value::Float(n as f64)),
                    Value::String(s) => match s.parse::<f64>() {
                        Ok(f) => Ok(Value::Float(f)),
                        Err(_) => Ok(Value::Null),
                    },
                    Value::Null => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::ToInteger(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::Int(n) => Ok(Value::Int(n)),
                    Value::Float(f) => Ok(Value::Int(f as i64)),
                    Value::String(s) => match s.parse::<i64>() {
                        Ok(n) => Ok(Value::Int(n)),
                        Err(_) => {
                            // Try parsing as float first, then truncate
                            match s.parse::<f64>() {
                                Ok(f) => Ok(Value::Int(f as i64)),
                                Err(_) => Ok(Value::Null),
                            }
                        }
                    },
                    Value::Null => Ok(Value::Null),
                    _ => Ok(Value::Null),
                }
            }
            ScalarFunction::Timestamp => {
                use std::time::SystemTime;
                let millis = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                Ok(Value::Int(millis))
            }
            ScalarFunction::RandomUUID => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                use std::time::SystemTime;
                // Simple UUID v4 generation without external crate
                let mut bytes = [0u8; 16];
                for i in 0..4 {
                    let mut hasher = DefaultHasher::new();
                    SystemTime::now().hash(&mut hasher);
                    std::thread::current().id().hash(&mut hasher);
                    (i as u64).hash(&mut hasher);
                    let hash = hasher.finish();
                    let offset = i * 4;
                    bytes[offset] = (hash >> 56) as u8;
                    bytes[offset + 1] = (hash >> 48) as u8;
                    bytes[offset + 2] = (hash >> 40) as u8;
                    bytes[offset + 3] = (hash >> 32) as u8;
                }
                // Set version 4
                bytes[6] = (bytes[6] & 0x0f) | 0x40;
                // Set variant
                bytes[8] = (bytes[8] & 0x3f) | 0x80;
                let uuid = format!(
                    "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                    bytes[0],
                    bytes[1],
                    bytes[2],
                    bytes[3],
                    bytes[4],
                    bytes[5],
                    bytes[6],
                    bytes[7],
                    bytes[8],
                    bytes[9],
                    bytes[10],
                    bytes[11],
                    bytes[12],
                    bytes[13],
                    bytes[14],
                    bytes[15],
                );
                Ok(Value::String(uuid))
            }
            ScalarFunction::Head(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => Ok(items.into_iter().next().unwrap_or(Value::Null)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "head() requires a list".to_string(),
                    )),
                }
            }
            ScalarFunction::Last(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => Ok(items.into_iter().last().unwrap_or(Value::Null)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "last() requires a list".to_string(),
                    )),
                }
            }
            ScalarFunction::Tail(expr) => {
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::List(items) => {
                        if items.is_empty() {
                            Ok(Value::List(vec![]))
                        } else {
                            Ok(Value::List(items.into_iter().skip(1).collect()))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "tail() requires a list".to_string(),
                    )),
                }
            }
            ScalarFunction::Range(start_expr, end_expr, step_expr) => {
                let start_val = self.evaluate_expression(start_expr, bindings)?;
                let end_val = self.evaluate_expression(end_expr, bindings)?;
                let step_val = step_expr
                    .as_ref()
                    .map(|e| self.evaluate_expression(e, bindings))
                    .transpose()?;
                match (start_val, end_val) {
                    (Value::Int(start), Value::Int(end)) => {
                        let step = match step_val {
                            Some(Value::Int(s)) => s,
                            None => 1,
                            Some(Value::Null) => return Ok(Value::Null),
                            _ => {
                                return Err(ExecuteError::TypeError(
                                    "range() step must be an integer".to_string(),
                                ));
                            }
                        };
                        if step == 0 {
                            return Err(ExecuteError::TypeError(
                                "range() step cannot be zero".to_string(),
                            ));
                        }
                        // Compute the element count up-front in i128 (so the
                        // subtraction cannot overflow i64) and reject ranges
                        // that would exhaust memory before allocating.
                        let count: i128 = if step > 0 {
                            if end < start {
                                0
                            } else {
                                (end as i128 - start as i128) / step as i128 + 1
                            }
                        } else if end > start {
                            0
                        } else {
                            (start as i128 - end as i128) / (-(step as i128)) + 1
                        };
                        if count > MAX_RANGE_LEN as i128 {
                            return Err(ExecuteError::TypeError(format!(
                                "range() would produce {} elements, exceeding the limit of {}",
                                count, MAX_RANGE_LEN
                            )));
                        }
                        let mut result = Vec::with_capacity(count as usize);
                        let mut i = start;
                        // `checked_add` guards against i64 overflow near the
                        // bounds, which would otherwise wrap and loop forever.
                        if step > 0 {
                            while i <= end {
                                result.push(Value::Int(i));
                                match i.checked_add(step) {
                                    Some(next) => i = next,
                                    None => break,
                                }
                            }
                        } else {
                            while i >= end {
                                result.push(Value::Int(i));
                                match i.checked_add(step) {
                                    Some(next) => i = next,
                                    None => break,
                                }
                            }
                        }
                        Ok(Value::List(result))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError(
                        "range() requires integer arguments".to_string(),
                    )),
                }
            }
            ScalarFunction::Reduce {
                acc_var,
                init,
                item_var,
                list,
                body,
            } => {
                let list_val = self.evaluate_expression(list, bindings)?;
                let init_val = self.evaluate_expression(init, bindings)?;
                let items = match list_val {
                    Value::List(items) => items,
                    Value::Null => return Ok(Value::Null),
                    _ => {
                        return Err(ExecuteError::TypeError(
                            "reduce() requires a list".to_string(),
                        ));
                    }
                };
                let mut acc = init_val;
                for item in items {
                    let mut local_bindings = bindings.clone();
                    local_bindings.insert(Arc::from(acc_var.as_str()), BindingValue::Scalar(acc));
                    local_bindings.insert(Arc::from(item_var.as_str()), BindingValue::Scalar(item));
                    acc = self.evaluate_expression(body, &local_bindings)?;
                }
                Ok(acc)
            }
            ScalarFunction::DateFunc(arg) => {
                use maharit_core::temporal;
                use std::time::SystemTime;
                match arg {
                    None => {
                        let ms = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64;
                        let days = (ms / 86_400_000) as i32;
                        Ok(Value::Date(days))
                    }
                    Some(expr) => {
                        let val = self.evaluate_expression(expr, bindings)?;
                        match val {
                            Value::String(s) => match temporal::parse_date(&s) {
                                Some(d) => Ok(Value::Date(d)),
                                None => Err(ExecuteError::TypeError(format!(
                                    "invalid date string: {}",
                                    s
                                ))),
                            },
                            Value::Date(d) => Ok(Value::Date(d)),
                            Value::Map(map) => {
                                let get = |key: &str, default: i32| -> Result<i32, ExecuteError> {
                                    match map.get(key) {
                                        None | Some(Value::Null) => Ok(default),
                                        Some(Value::Int(n)) => i32::try_from(*n).map_err(|_| {
                                            ExecuteError::TypeError(format!(
                                                "date(): {key} is out of range: {n}"
                                            ))
                                        }),
                                        Some(other) => Err(ExecuteError::TypeError(format!(
                                            "date(): {key} must be an integer, got {other}"
                                        ))),
                                    }
                                };
                                let (year, month, day) =
                                    (get("year", 1970)?, get("month", 1)?, get("day", 1)?);
                                temporal::checked_ymd_to_days(year, month, day)
                                    .map(Value::Date)
                                    .ok_or_else(|| {
                                        ExecuteError::TypeError(format!(
                                            "invalid date: year {year}, month {month}, day {day}"
                                        ))
                                    })
                            }
                            _ => Err(ExecuteError::TypeError(
                                "date() requires a string or map argument".to_string(),
                            )),
                        }
                    }
                }
            }
            ScalarFunction::DateTimeFunc(arg) => {
                use maharit_core::temporal;
                use std::time::SystemTime;
                match arg {
                    None => {
                        let ms = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64;
                        Ok(Value::DateTime(ms))
                    }
                    Some(expr) => {
                        let val = self.evaluate_expression(expr, bindings)?;
                        match val {
                            Value::String(s) => match temporal::parse_datetime(&s) {
                                Some(ms) => Ok(Value::DateTime(ms)),
                                None => Err(ExecuteError::TypeError(format!(
                                    "invalid datetime string: {}",
                                    s
                                ))),
                            },
                            Value::DateTime(ms) => Ok(Value::DateTime(ms)),
                            _ => Err(ExecuteError::TypeError(
                                "datetime() requires a string argument".to_string(),
                            )),
                        }
                    }
                }
            }
            ScalarFunction::DurationFunc(expr) => {
                use maharit_core::temporal;
                let val = self.evaluate_expression(expr, bindings)?;
                match val {
                    Value::String(s) => match temporal::parse_duration(&s) {
                        Some((months, days, millis)) => Ok(Value::Duration {
                            months,
                            days,
                            millis,
                        }),
                        None => Err(ExecuteError::TypeError(format!(
                            "invalid duration string: {}",
                            s
                        ))),
                    },
                    Value::Map(map) => {
                        let get = |key: &str| -> Result<i64, ExecuteError> {
                            match map.get(key) {
                                None | Some(Value::Null) => Ok(0),
                                Some(Value::Int(n)) => Ok(*n),
                                Some(other) => Err(ExecuteError::TypeError(format!(
                                    "duration(): {key} must be an integer, got {other}"
                                ))),
                            }
                        };
                        let overflow = || ExecuteError::TypeError("integer overflow".to_string());
                        // 各成分を `unit` 倍して足す（途中で i64 を超えたらエラー）
                        let sum = |parts: &[(&str, i64)]| -> Result<i64, ExecuteError> {
                            parts.iter().try_fold(0i64, |acc, (key, unit)| {
                                get(key)?
                                    .checked_mul(*unit)
                                    .and_then(|v| acc.checked_add(v))
                                    .ok_or_else(overflow)
                            })
                        };
                        let months = sum(&[("years", 12), ("months", 1)])?;
                        let days = sum(&[("weeks", 7), ("days", 1)])?;
                        let millis = sum(&[
                            ("hours", 3_600_000),
                            ("minutes", 60_000),
                            ("seconds", 1_000),
                            ("milliseconds", 1),
                        ])?;
                        Ok(Value::Duration {
                            months: i32::try_from(months).map_err(|_| overflow())?,
                            days: i32::try_from(days).map_err(|_| overflow())?,
                            millis,
                        })
                    }
                    _ => Err(ExecuteError::TypeError(
                        "duration() requires a string or map argument".to_string(),
                    )),
                }
            }
        }
    }
}
