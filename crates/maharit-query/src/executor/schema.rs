//! 制約・全文索引・プロパティ索引の作成、削除、一覧。

use super::*;

impl<'a> Executor<'a> {
    // ========== CONSTRAINT DDL ==========

    pub(super) fn execute_create_constraint(
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
            ConstraintTypeAst::RequiredLabel(required_label) => {
                ConstraintType::RequiredLabel(required_label)
            }
            ConstraintTypeAst::EndpointLabel {
                source_label,
                target_label,
            } => ConstraintType::EndpointLabel {
                source_label,
                target_label,
            },
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
                columns: vec![Value::String(format!("Constraint '{}' created", cc.name))],
            }],
        ))
    }

    pub(super) fn execute_drop_constraint(
        &mut self,
        dc: DropConstraintStatement,
    ) -> Result<ResultSet, ExecuteError> {
        self.constraints.drop_constraint(&dc.name)?;

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!("Constraint '{}' dropped", dc.name))],
            }],
        ))
    }

    pub(super) fn execute_show_constraints(&self) -> Result<ResultSet, ExecuteError> {
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

    pub(super) fn execute_create_fulltext_index(
        &mut self,
        cfi: CreateFulltextIndexStatement,
    ) -> Result<ResultSet, ExecuteError> {
        self.fulltext
            .create_index(&cfi.name, &cfi.label, cfi.properties)?;

        // Index existing nodes that match the label
        let node_ids: Vec<NodeId> = self
            .graph_ref()
            .all_nodes()
            .into_iter()
            .filter(|n| n.has_label(&cfi.label))
            .map(|n| n.id)
            .collect();

        for node_id in node_ids {
            if let Some(node) = self.graph_ref().get_node(node_id) {
                let props = Arc::clone(&node.properties);
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

    pub(super) fn execute_drop_fulltext_index(
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

    // ========== PROPERTY INDEX ==========

    pub(super) fn execute_create_index(
        &mut self,
        ci: CreateIndexStatement,
    ) -> Result<ResultSet, ExecuteError> {
        let def = IndexDefinition::new(ci.label.clone(), ci.property.clone());
        self.property_index.create_index(def);

        // Index existing nodes that match the label
        let node_ids: Vec<NodeId> = self
            .graph_ref()
            .all_nodes()
            .into_iter()
            .filter(|n| n.has_label(&ci.label))
            .map(|n| n.id)
            .collect();

        for node_id in node_ids {
            if let Some(node) = self.graph_ref().get_node(node_id)
                && let Some(val) = node.get_property(&ci.property)
            {
                self.property_index
                    .index_property(node_id, &ci.property, val);
            }
        }

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!(
                    "Index created on :{}({})",
                    ci.label, ci.property
                ))],
            }],
        ))
    }

    pub(super) fn execute_drop_index(
        &mut self,
        di: DropIndexStatement,
    ) -> Result<ResultSet, ExecuteError> {
        self.property_index.drop_index(&di.label, &di.property);

        Ok(ResultSet::new(
            vec!["result".to_string()],
            vec![Row {
                columns: vec![Value::String(format!(
                    "Index dropped on :{}({})",
                    di.label, di.property
                ))],
            }],
        ))
    }

    pub(super) fn execute_show_indexes(&self) -> Result<ResultSet, ExecuteError> {
        let indexes = self.property_index.list_indexes();

        let mut rows = Vec::new();
        for def in indexes {
            rows.push(Row {
                columns: vec![
                    Value::String(def.label.clone()),
                    Value::String(def.property.clone()),
                ],
            });
        }

        Ok(ResultSet::new(
            vec!["label".to_string(), "property".to_string()],
            rows,
        ))
    }
}
