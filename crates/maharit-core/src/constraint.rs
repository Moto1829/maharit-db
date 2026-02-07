use std::collections::HashMap;

use thiserror::Error;

use crate::graph::{Graph, Node, NodeId};
use crate::property::PropertyValue;

/// 制約違反エラー
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ConstraintError {
    #[error("constraint already exists: {0}")]
    AlreadyExists(String),

    #[error("constraint not found: {0}")]
    NotFound(String),

    #[error("unique constraint violation: {name} - property '{property}' with value '{value}' already exists on label '{label}'")]
    UniqueViolation {
        name: String,
        label: String,
        property: String,
        value: String,
    },

    #[error("not null constraint violation: {name} - property '{property}' is required on label '{label}'")]
    NotNullViolation {
        name: String,
        label: String,
        property: String,
    },

    #[error("type constraint violation: {name} - property '{property}' on label '{label}' must be {expected_type}, got {actual_type}")]
    TypeViolation {
        name: String,
        label: String,
        property: String,
        expected_type: String,
        actual_type: String,
    },
}

/// プロパティの型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyType {
    Bool,
    Int,
    Float,
    String,
}

impl std::fmt::Display for PropertyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropertyType::Bool => write!(f, "BOOLEAN"),
            PropertyType::Int => write!(f, "INTEGER"),
            PropertyType::Float => write!(f, "FLOAT"),
            PropertyType::String => write!(f, "STRING"),
        }
    }
}

/// 制約の種類
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintType {
    /// ユニーク制約
    Unique,
    /// NOT NULL制約
    NotNull,
    /// 型制約
    TypeCheck(PropertyType),
}

impl std::fmt::Display for ConstraintType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstraintType::Unique => write!(f, "UNIQUE"),
            ConstraintType::NotNull => write!(f, "NOT NULL"),
            ConstraintType::TypeCheck(t) => write!(f, "TYPE({})", t),
        }
    }
}

/// スキーマ制約
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    /// 制約名
    pub name: String,
    /// 対象ラベル
    pub label: String,
    /// 対象プロパティ
    pub property: String,
    /// 制約の種類
    pub constraint_type: ConstraintType,
}

/// 制約マネージャー
#[derive(Debug, Clone, Default)]
pub struct ConstraintManager {
    /// name -> Constraint
    constraints: HashMap<String, Constraint>,
    /// 制約の有効/無効フラグ（バルクロード時に無効化）
    enabled: bool,
}

impl ConstraintManager {
    pub fn new() -> Self {
        Self {
            constraints: HashMap::new(),
            enabled: true,
        }
    }

    /// 制約を作成
    pub fn create_constraint(&mut self, constraint: Constraint) -> Result<(), ConstraintError> {
        if self.constraints.contains_key(&constraint.name) {
            return Err(ConstraintError::AlreadyExists(constraint.name));
        }
        self.constraints.insert(constraint.name.clone(), constraint);
        Ok(())
    }

    /// 制約を削除
    pub fn drop_constraint(&mut self, name: &str) -> Result<(), ConstraintError> {
        if self.constraints.remove(name).is_none() {
            return Err(ConstraintError::NotFound(name.to_string()));
        }
        Ok(())
    }

    /// 制約一覧を取得
    pub fn list_constraints(&self) -> Vec<&Constraint> {
        let mut list: Vec<_> = self.constraints.values().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// 制約を有効化
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// 制約を無効化（バルクロード用）
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// 制約が有効かどうか
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// ノード作成時のバリデーション
    pub fn validate_node_create(
        &self,
        graph: &Graph,
        label: &str,
        properties: &HashMap<String, PropertyValue>,
        exclude_node_id: Option<NodeId>,
    ) -> Result<(), ConstraintError> {
        if !self.enabled {
            return Ok(());
        }

        for constraint in self.constraints.values() {
            if constraint.label != label {
                continue;
            }

            match &constraint.constraint_type {
                ConstraintType::Unique => {
                    if let Some(value) = properties.get(&constraint.property) {
                        self.check_unique(graph, constraint, value, exclude_node_id)?;
                    }
                }
                ConstraintType::NotNull => {
                    if !properties.contains_key(&constraint.property) {
                        return Err(ConstraintError::NotNullViolation {
                            name: constraint.name.clone(),
                            label: constraint.label.clone(),
                            property: constraint.property.clone(),
                        });
                    }
                    if properties.get(&constraint.property) == Some(&PropertyValue::Null) {
                        return Err(ConstraintError::NotNullViolation {
                            name: constraint.name.clone(),
                            label: constraint.label.clone(),
                            property: constraint.property.clone(),
                        });
                    }
                }
                ConstraintType::TypeCheck(expected) => {
                    if let Some(value) = properties.get(&constraint.property) {
                        self.check_type(constraint, value, expected)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// プロパティ更新時のバリデーション
    pub fn validate_property_set(
        &self,
        graph: &Graph,
        node: &Node,
        property: &str,
        value: &PropertyValue,
    ) -> Result<(), ConstraintError> {
        if !self.enabled {
            return Ok(());
        }

        for constraint in self.constraints.values() {
            if constraint.label != node.label || constraint.property != property {
                continue;
            }

            match &constraint.constraint_type {
                ConstraintType::Unique => {
                    self.check_unique(graph, constraint, value, Some(node.id))?;
                }
                ConstraintType::NotNull => {
                    if matches!(value, PropertyValue::Null) {
                        return Err(ConstraintError::NotNullViolation {
                            name: constraint.name.clone(),
                            label: constraint.label.clone(),
                            property: constraint.property.clone(),
                        });
                    }
                }
                ConstraintType::TypeCheck(expected) => {
                    self.check_type(constraint, value, expected)?;
                }
            }
        }

        Ok(())
    }

    /// プロパティ削除時のバリデーション
    pub fn validate_property_remove(
        &self,
        node: &Node,
        property: &str,
    ) -> Result<(), ConstraintError> {
        if !self.enabled {
            return Ok(());
        }

        for constraint in self.constraints.values() {
            if constraint.label != node.label || constraint.property != property {
                continue;
            }

            if matches!(constraint.constraint_type, ConstraintType::NotNull) {
                return Err(ConstraintError::NotNullViolation {
                    name: constraint.name.clone(),
                    label: constraint.label.clone(),
                    property: constraint.property.clone(),
                });
            }
        }

        Ok(())
    }

    fn check_unique(
        &self,
        graph: &Graph,
        constraint: &Constraint,
        value: &PropertyValue,
        exclude_node_id: Option<NodeId>,
    ) -> Result<(), ConstraintError> {
        for node in graph.nodes() {
            if node.label != constraint.label {
                continue;
            }
            if let Some(exclude_id) = exclude_node_id {
                if node.id == exclude_id {
                    continue;
                }
            }
            if let Some(existing_value) = node.get_property(&constraint.property) {
                if existing_value == value {
                    return Err(ConstraintError::UniqueViolation {
                        name: constraint.name.clone(),
                        label: constraint.label.clone(),
                        property: constraint.property.clone(),
                        value: format!("{}", value),
                    });
                }
            }
        }
        Ok(())
    }

    fn check_type(
        &self,
        constraint: &Constraint,
        value: &PropertyValue,
        expected: &PropertyType,
    ) -> Result<(), ConstraintError> {
        if matches!(value, PropertyValue::Null) {
            return Ok(()); // NULL is allowed unless NOT NULL constraint
        }

        let type_ok = match (expected, value) {
            (PropertyType::Bool, PropertyValue::Bool(_)) => true,
            (PropertyType::Int, PropertyValue::Int(_)) => true,
            (PropertyType::Float, PropertyValue::Float(_)) => true,
            (PropertyType::String, PropertyValue::String(_)) => true,
            _ => false,
        };

        if !type_ok {
            let actual_type = match value {
                PropertyValue::Null => "NULL",
                PropertyValue::Bool(_) => "BOOLEAN",
                PropertyValue::Int(_) => "INTEGER",
                PropertyValue::Float(_) => "FLOAT",
                PropertyValue::String(_) => "STRING",
            };
            return Err(ConstraintError::TypeViolation {
                name: constraint.name.clone(),
                label: constraint.label.clone(),
                property: constraint.property.clone(),
                expected_type: expected.to_string(),
                actual_type: actual_type.to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_graph_with_alice() -> Graph {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", PropertyValue::String("Alice".to_string()));
            node.set_property("email", PropertyValue::String("alice@example.com".to_string()));
            node.set_property("age", PropertyValue::Int(30));
        }
        graph
    }

    #[test]
    fn test_create_constraint() {
        let mut cm = ConstraintManager::new();
        let constraint = Constraint {
            name: "unique_email".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        };
        assert!(cm.create_constraint(constraint).is_ok());
    }

    #[test]
    fn test_create_duplicate_constraint() {
        let mut cm = ConstraintManager::new();
        let constraint = Constraint {
            name: "unique_email".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        };
        cm.create_constraint(constraint.clone()).unwrap();
        assert!(matches!(
            cm.create_constraint(constraint),
            Err(ConstraintError::AlreadyExists(_))
        ));
    }

    #[test]
    fn test_drop_constraint() {
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "c1".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        })
        .unwrap();
        assert!(cm.drop_constraint("c1").is_ok());
        assert!(cm.list_constraints().is_empty());
    }

    #[test]
    fn test_drop_nonexistent_constraint() {
        let mut cm = ConstraintManager::new();
        assert!(matches!(
            cm.drop_constraint("nonexistent"),
            Err(ConstraintError::NotFound(_))
        ));
    }

    #[test]
    fn test_list_constraints() {
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "c2".to_string(),
            label: "Person".to_string(),
            property: "name".to_string(),
            constraint_type: ConstraintType::NotNull,
        })
        .unwrap();
        cm.create_constraint(Constraint {
            name: "c1".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        })
        .unwrap();

        let list = cm.list_constraints();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "c1"); // sorted by name
        assert_eq!(list[1].name, "c2");
    }

    #[test]
    fn test_unique_constraint_pass() {
        let graph = setup_graph_with_alice();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "unique_email".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        })
        .unwrap();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("bob@example.com".to_string()),
        );

        assert!(cm.validate_node_create(&graph, "Person", &props, None).is_ok());
    }

    #[test]
    fn test_unique_constraint_violation() {
        let graph = setup_graph_with_alice();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "unique_email".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        })
        .unwrap();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("alice@example.com".to_string()),
        );

        assert!(matches!(
            cm.validate_node_create(&graph, "Person", &props, None),
            Err(ConstraintError::UniqueViolation { .. })
        ));
    }

    #[test]
    fn test_unique_constraint_self_update() {
        let graph = setup_graph_with_alice();
        let node_id = graph.nodes().next().unwrap().id;

        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "unique_email".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        })
        .unwrap();

        // Updating same node with same value should pass
        let node = graph.get_node(node_id).unwrap();
        assert!(cm
            .validate_property_set(
                &graph,
                node,
                "email",
                &PropertyValue::String("alice@example.com".to_string()),
            )
            .is_ok());
    }

    #[test]
    fn test_not_null_constraint_pass() {
        let graph = Graph::new();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "require_name".to_string(),
            label: "Person".to_string(),
            property: "name".to_string(),
            constraint_type: ConstraintType::NotNull,
        })
        .unwrap();

        let mut props = HashMap::new();
        props.insert(
            "name".to_string(),
            PropertyValue::String("Bob".to_string()),
        );

        assert!(cm.validate_node_create(&graph, "Person", &props, None).is_ok());
    }

    #[test]
    fn test_not_null_constraint_violation_missing() {
        let graph = Graph::new();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "require_name".to_string(),
            label: "Person".to_string(),
            property: "name".to_string(),
            constraint_type: ConstraintType::NotNull,
        })
        .unwrap();

        let props = HashMap::new(); // empty - name missing

        assert!(matches!(
            cm.validate_node_create(&graph, "Person", &props, None),
            Err(ConstraintError::NotNullViolation { .. })
        ));
    }

    #[test]
    fn test_not_null_constraint_violation_null() {
        let graph = Graph::new();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "require_name".to_string(),
            label: "Person".to_string(),
            property: "name".to_string(),
            constraint_type: ConstraintType::NotNull,
        })
        .unwrap();

        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::Null);

        assert!(matches!(
            cm.validate_node_create(&graph, "Person", &props, None),
            Err(ConstraintError::NotNullViolation { .. })
        ));
    }

    #[test]
    fn test_not_null_prevent_remove() {
        let graph = setup_graph_with_alice();
        let node = graph.nodes().next().unwrap();

        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "require_name".to_string(),
            label: "Person".to_string(),
            property: "name".to_string(),
            constraint_type: ConstraintType::NotNull,
        })
        .unwrap();

        assert!(matches!(
            cm.validate_property_remove(node, "name"),
            Err(ConstraintError::NotNullViolation { .. })
        ));
    }

    #[test]
    fn test_type_constraint_pass() {
        let graph = Graph::new();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "age_type".to_string(),
            label: "Person".to_string(),
            property: "age".to_string(),
            constraint_type: ConstraintType::TypeCheck(PropertyType::Int),
        })
        .unwrap();

        let mut props = HashMap::new();
        props.insert("age".to_string(), PropertyValue::Int(25));

        assert!(cm.validate_node_create(&graph, "Person", &props, None).is_ok());
    }

    #[test]
    fn test_type_constraint_violation() {
        let graph = Graph::new();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "age_type".to_string(),
            label: "Person".to_string(),
            property: "age".to_string(),
            constraint_type: ConstraintType::TypeCheck(PropertyType::Int),
        })
        .unwrap();

        let mut props = HashMap::new();
        props.insert(
            "age".to_string(),
            PropertyValue::String("twenty-five".to_string()),
        );

        assert!(matches!(
            cm.validate_node_create(&graph, "Person", &props, None),
            Err(ConstraintError::TypeViolation { .. })
        ));
    }

    #[test]
    fn test_type_constraint_allows_null() {
        let graph = Graph::new();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "age_type".to_string(),
            label: "Person".to_string(),
            property: "age".to_string(),
            constraint_type: ConstraintType::TypeCheck(PropertyType::Int),
        })
        .unwrap();

        let mut props = HashMap::new();
        props.insert("age".to_string(), PropertyValue::Null);

        assert!(cm.validate_node_create(&graph, "Person", &props, None).is_ok());
    }

    #[test]
    fn test_constraint_different_label_ignored() {
        let graph = Graph::new();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "unique_email".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        })
        .unwrap();

        // Creating a Company node should not trigger Person constraints
        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("alice@example.com".to_string()),
        );

        assert!(cm.validate_node_create(&graph, "Company", &props, None).is_ok());
    }

    #[test]
    fn test_disable_enable_constraints() {
        let graph = setup_graph_with_alice();
        let mut cm = ConstraintManager::new();
        cm.create_constraint(Constraint {
            name: "unique_email".to_string(),
            label: "Person".to_string(),
            property: "email".to_string(),
            constraint_type: ConstraintType::Unique,
        })
        .unwrap();

        // Disable constraints
        cm.disable();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("alice@example.com".to_string()),
        );

        // Should pass even with duplicate
        assert!(cm.validate_node_create(&graph, "Person", &props, None).is_ok());

        // Re-enable
        cm.enable();
        assert!(cm.validate_node_create(&graph, "Person", &props, None).is_err());
    }
}
