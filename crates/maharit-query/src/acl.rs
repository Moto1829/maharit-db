//! ラベル・プロパティ単位のアクセス制御（ACL）をクエリ実行に適用する。
//!
//! サーバーはセッションのユーザーとロールから [`AccessPolicy`] を作り、
//! [`Executor::with_access_policy`](crate::Executor::with_access_policy) で渡す。
//! エグゼキュータはグラフを [`AclBackend`] で包み、次のように振る舞う。
//!
//! - 読み取りが拒否されたラベルを持つノードは存在しないものとして扱う（MATCH・件数・
//!   プロシージャのどこにも現れない）。両端のどちらかが見えないリレーションシップも見えない。
//! - 読み取りが拒否されたプロパティは無いもの（`null`）として扱い、`keys()` などにも出さない。
//! - 書き込みが拒否されたノード・プロパティを変更すると、その文はエラーになる
//!   （呼び出し側が文の変更をすべて取り消す）。

use maharit_core::{Edge, EdgeEnds, EdgeId, GraphBackend, GraphError, Node, NodeId, PropertyValue};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// 読み取りまたは書き込みのどちらか一方についての拒否規則。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelRules {
    /// 個別の規則が無いラベル（とラベルの無いノード）を拒否するか。
    pub deny_all: bool,
    /// ラベルごとの判定（`true` なら拒否）。`deny_all` より優先する。
    pub labels: HashMap<String, bool>,
    /// ラベルごとに拒否するプロパティ名。
    pub properties: HashMap<String, HashSet<String>>,
}

impl LabelRules {
    /// 何も拒否しないか。
    pub fn is_empty(&self) -> bool {
        !self.deny_all && !self.labels.values().any(|d| *d) && self.properties.is_empty()
    }

    /// `label` が拒否されているか。
    pub fn label_denied(&self, label: &str) -> bool {
        self.labels.get(label).copied().unwrap_or(self.deny_all)
    }

    /// `labels` を持つノードが拒否されているか（どれか 1 つでも拒否なら拒否）。
    pub fn node_denied(&self, labels: &[String]) -> bool {
        if labels.is_empty() {
            self.deny_all
        } else {
            labels.iter().any(|l| self.label_denied(l))
        }
    }

    /// `labels` を持つノードのプロパティ `key` が拒否されているか。
    pub fn property_denied(&self, labels: &[String], key: &str) -> bool {
        labels
            .iter()
            .any(|l| self.properties.get(l).is_some_and(|s| s.contains(key)))
    }

    /// `labels` を持つノードで拒否されるプロパティがあるか。
    fn has_property_rules_for(&self, labels: &[String]) -> bool {
        labels.iter().any(|l| self.properties.contains_key(l))
    }
}

/// 1 つのセッションに適用する ACL。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessPolicy {
    pub read: LabelRules,
    pub write: LabelRules,
}

impl AccessPolicy {
    /// 何も制限しないか（このときはラッパーを使わずに実行してよい）。
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty()
    }
}

/// [`AccessPolicy`] に従ってグラフを見せる・書き込みを検査するラッパー。
///
/// 書き込みが拒否されたときも変更そのものは下のグラフに渡し、違反を記録する。
/// エグゼキュータは文の終わりに違反を見てエラーを返し、サーバーが文の変更を
/// まとめて取り消す（途中で止めると、作りかけのノードなどが残るため）。
pub(crate) struct AclBackend<'a> {
    inner: *mut (dyn GraphBackend + 'a),
    policy: Arc<AccessPolicy>,
    violation: Mutex<Option<String>>,
}

// SAFETY: `inner` は Executor の `graph` と同じポインタで、Executor と同じ規則
// （読み取りは共有、書き込みは呼び出し側が排他を保証）でしか使わない。
unsafe impl Send for AclBackend<'_> {}
// SAFETY: 上と同じ。共有参照からは読み取りしかしない（違反の記録は `Mutex` で守る）。
unsafe impl Sync for AclBackend<'_> {}

impl<'a> AclBackend<'a> {
    pub(crate) fn new(inner: *mut (dyn GraphBackend + 'a), policy: AccessPolicy) -> Self {
        Self {
            inner,
            policy: Arc::new(policy),
            violation: Mutex::new(None),
        }
    }

    /// 包んでいるグラフ（ACL を通さない）。
    pub(crate) fn raw(&self) -> &dyn GraphBackend {
        // SAFETY: 構築時に有効なポインタを受け取り、Executor の生存中は有効。
        unsafe { &*self.inner }
    }

    fn raw_mut(&mut self) -> &mut dyn GraphBackend {
        // SAFETY: `&mut self` を通じてのみ呼ぶ。排他は Executor の graph_mut と同じ前提。
        unsafe { &mut *self.inner }
    }

    pub(crate) fn policy(&self) -> &AccessPolicy {
        &self.policy
    }

    /// 記録した書き込み違反を取り出す。
    pub(crate) fn take_violation(&self) -> Option<String> {
        self.violation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn deny(&self, message: String) {
        let mut v = self.violation.lock().unwrap_or_else(|e| e.into_inner());
        if v.is_none() {
            *v = Some(message);
        }
    }

    fn labels_of(&self, id: NodeId) -> Option<Vec<String>> {
        self.raw().get_node(id).map(|n| n.labels)
    }

    fn visible_labels(&self, labels: &[String]) -> bool {
        !self.policy.read.node_denied(labels)
    }

    fn node_visible(&self, id: NodeId) -> bool {
        self.labels_of(id).is_some_and(|l| self.visible_labels(&l))
    }

    fn edge_visible(&self, e: &Edge) -> bool {
        self.node_visible(e.from) && self.node_visible(e.to)
    }

    /// 読み取りが拒否されたプロパティを取り除いたノード。
    fn mask(&self, mut node: Node) -> Node {
        if self.policy.read.has_property_rules_for(&node.labels) {
            let props: HashMap<String, PropertyValue> = node
                .properties
                .iter()
                .filter(|(k, _)| !self.policy.read.property_denied(&node.labels, k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            node.properties = Arc::new(props);
        }
        node
    }

    fn label_list(labels: &[String]) -> String {
        if labels.is_empty() {
            "(no label)".to_string()
        } else {
            labels
                .iter()
                .map(|l| format!(":{l}"))
                .collect::<Vec<_>>()
                .join("")
        }
    }

    /// ノード `id` を変更してよいか確かめる（だめなら違反を記録）。
    fn check_node_write(&self, id: NodeId) {
        if let Some(labels) = self.labels_of(id)
            && self.policy.write.node_denied(&labels)
        {
            self.deny(format!(
                "no write access to nodes {}",
                Self::label_list(&labels)
            ));
        }
    }

    fn check_property_write(&self, id: NodeId, key: &str) {
        let Some(labels) = self.labels_of(id) else {
            return;
        };
        if self.policy.write.node_denied(&labels) {
            self.deny(format!(
                "no write access to nodes {}",
                Self::label_list(&labels)
            ));
        } else if self.policy.write.property_denied(&labels, key)
            || self.policy.read.property_denied(&labels, key)
        {
            // 読めないプロパティも書けない（見えない値を上書き・削除させない）
            self.deny(format!(
                "no write access to property {}.{key}",
                Self::label_list(&labels)
            ));
        }
    }

    fn check_label_write(&self, label: &str) {
        if self.policy.write.label_denied(label) {
            self.deny(format!("no write access to nodes :{label}"));
        }
    }
}

impl GraphBackend for AclBackend<'_> {
    fn get_node(&self, id: NodeId) -> Option<Node> {
        let node = self.raw().get_node(id)?;
        if self.visible_labels(&node.labels) {
            Some(self.mask(node))
        } else {
            None
        }
    }

    fn node_has_all_labels(&self, id: NodeId, labels: &[String]) -> bool {
        self.node_visible(id) && self.raw().node_has_all_labels(id, labels)
    }

    fn get_node_property(&self, id: NodeId, key: &str) -> Option<PropertyValue> {
        let labels = self.labels_of(id)?;
        if !self.visible_labels(&labels) || self.policy.read.property_denied(&labels, key) {
            return None;
        }
        self.raw().get_node_property(id, key)
    }

    fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        if self.policy.read.label_denied(label) {
            return Vec::new();
        }
        self.raw()
            .nodes_by_label(label)
            .into_iter()
            .filter(|id| self.node_visible(*id))
            .collect()
    }

    fn get_edge(&self, id: EdgeId) -> Option<Edge> {
        self.raw().get_edge(id).filter(|e| self.edge_visible(e))
    }

    fn node_ids(&self) -> Vec<NodeId> {
        self.raw()
            .node_ids()
            .into_iter()
            .filter(|id| self.node_visible(*id))
            .collect()
    }

    fn all_nodes(&self) -> Vec<Node> {
        self.raw()
            .all_nodes()
            .into_iter()
            .filter(|n| self.visible_labels(&n.labels))
            .map(|n| self.mask(n))
            .collect()
    }

    fn edge_ids(&self) -> Vec<EdgeId> {
        self.all_edges().into_iter().map(|e| e.id).collect()
    }

    fn all_edges(&self) -> Vec<Edge> {
        self.raw()
            .all_edges()
            .into_iter()
            .filter(|e| self.edge_visible(e))
            .collect()
    }

    fn outgoing_edges(&self, node_id: NodeId) -> Vec<Edge> {
        if !self.node_visible(node_id) {
            return Vec::new();
        }
        self.raw()
            .outgoing_edges(node_id)
            .into_iter()
            .filter(|e| self.node_visible(e.to))
            .collect()
    }

    fn incoming_edges(&self, node_id: NodeId) -> Vec<Edge> {
        if !self.node_visible(node_id) {
            return Vec::new();
        }
        self.raw()
            .incoming_edges(node_id)
            .into_iter()
            .filter(|e| self.node_visible(e.from))
            .collect()
    }

    fn adjacent_edges(
        &self,
        node_id: NodeId,
        outgoing: bool,
        types: &[String],
        out: &mut Vec<EdgeEnds>,
    ) {
        if !self.node_visible(node_id) {
            return;
        }
        let mut found = Vec::new();
        self.raw()
            .adjacent_edges(node_id, outgoing, types, &mut found);
        // 相手側のノードが見えないリレーションシップは隠す（outgoing_edges と同じ）
        out.extend(
            found
                .into_iter()
                .filter(|e| self.node_visible(if outgoing { e.to } else { e.from })),
        );
    }

    fn has_incident_edges(&self, node_id: NodeId) -> bool {
        // 見えないリレーションシップも数える: DETACH なしの DELETE で
        // 見えない相手とのつながりを切ってしまわないように
        self.raw().has_incident_edges(node_id)
    }

    fn node_count(&self) -> usize {
        self.node_ids().len()
    }

    fn edge_count(&self) -> usize {
        self.all_edges().len()
    }

    fn contains_node(&self, id: NodeId) -> bool {
        self.node_visible(id)
    }

    fn create_node_with_labels(&mut self, labels: Vec<String>) -> NodeId {
        if self.policy.write.node_denied(&labels) {
            self.deny(format!(
                "no write access to nodes {}",
                Self::label_list(&labels)
            ));
        }
        self.raw_mut().create_node_with_labels(labels)
    }

    fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<EdgeId, GraphError> {
        self.check_node_write(from);
        self.check_node_write(to);
        self.raw_mut().create_edge(from, to, label)
    }

    fn delete_node(&mut self, id: NodeId) -> Option<Node> {
        self.check_node_write(id);
        let hidden_edge = self
            .raw()
            .outgoing_edges(id)
            .iter()
            .map(|e| e.to)
            .chain(self.raw().incoming_edges(id).iter().map(|e| e.from))
            .any(|other| !self.node_visible(other));
        if hidden_edge {
            self.deny(
                "cannot delete a node that has relationships to nodes you cannot read".to_string(),
            );
        }
        self.raw_mut().delete_node(id)
    }

    fn delete_edge(&mut self, id: EdgeId) -> Option<Edge> {
        if let Some(e) = self.raw().get_edge(id) {
            self.check_node_write(e.from);
            self.check_node_write(e.to);
        }
        self.raw_mut().delete_edge(id)
    }

    fn set_node_property(&mut self, id: NodeId, key: &str, value: PropertyValue) {
        self.check_property_write(id, key);
        self.raw_mut().set_node_property(id, key, value);
    }

    fn remove_node_property(&mut self, id: NodeId, key: &str) -> Option<PropertyValue> {
        self.check_property_write(id, key);
        self.raw_mut().remove_node_property(id, key)
    }

    fn add_node_label(&mut self, id: NodeId, label: String) {
        self.check_node_write(id);
        self.check_label_write(&label);
        self.raw_mut().add_node_label(id, label);
    }

    fn remove_node_label(&mut self, id: NodeId, label: &str) {
        self.check_node_write(id);
        self.raw_mut().remove_node_label(id, label);
    }

    fn set_edge_property(&mut self, id: EdgeId, key: &str, value: PropertyValue) {
        if let Some(e) = self.raw().get_edge(id) {
            self.check_node_write(e.from);
            self.check_node_write(e.to);
        }
        self.raw_mut().set_edge_property(id, key, value);
    }

    fn remove_edge_property(&mut self, id: EdgeId, key: &str) -> Option<PropertyValue> {
        if let Some(e) = self.raw().get_edge(id) {
            self.check_node_write(e.from);
            self.check_node_write(e.to);
        }
        self.raw_mut().remove_edge_property(id, key)
    }

    fn restore_node(&mut self, id: NodeId, labels: Vec<String>) {
        self.raw_mut().restore_node(id, labels);
    }

    fn restore_edge(
        &mut self,
        id: EdgeId,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<(), GraphError> {
        self.raw_mut().restore_edge(id, from, to, label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AclAction, AclCommand, AclGrantee, AclStatement, AclTarget, Statement};
    use crate::executor::{ExecuteError, Executor, ResultSet, Value};
    use crate::parser::Parser;
    use maharit_core::Graph;

    fn parse(q: &str) -> Statement {
        Parser::new(q).unwrap().parse().unwrap()
    }

    fn run(
        g: &mut Graph,
        q: &str,
        policy: Option<&AccessPolicy>,
    ) -> Result<ResultSet, ExecuteError> {
        Executor::new(g)
            .with_access_policy(policy.cloned())
            .execute_with_params(parse(q), HashMap::new())
    }

    fn values(g: &mut Graph, q: &str, policy: &AccessPolicy) -> Vec<Value> {
        let mut v: Vec<Value> = run(g, q, Some(policy))
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.columns[0].clone())
            .collect();
        v.sort_by_key(|x| format!("{x:?}"));
        v
    }

    fn read_deny(labels: &[&str]) -> AccessPolicy {
        let mut p = AccessPolicy::default();
        for l in labels {
            p.read.labels.insert(l.to_string(), true);
        }
        p
    }

    fn sample() -> Graph {
        let mut g = Graph::new();
        run(
            &mut g,
            "CREATE (a:Public {name: 'a'})-[:R]->(b:Secret {name: 'b'})",
            None,
        )
        .unwrap();
        run(&mut g, "CREATE (:Account {name: 'u', password: 'p'})", None).unwrap();
        g
    }

    #[test]
    fn read_denied_label_is_invisible() {
        let mut g = sample();
        let p = read_deny(&["Secret"]);
        assert_eq!(
            values(&mut g, "MATCH (n) RETURN count(*)", &p),
            vec![Value::Int(2)]
        );
        assert_eq!(
            values(&mut g, "MATCH (n:Secret) RETURN count(n)", &p),
            vec![Value::Int(0)]
        );
        assert!(values(&mut g, "MATCH (n:Secret) RETURN n.name", &p).is_empty());
        assert_eq!(
            values(&mut g, "MATCH ()-[r]->() RETURN count(r)", &p),
            vec![Value::Int(0)]
        );
        let labels = values(&mut g, "CALL db.labels() YIELD label RETURN label", &p);
        assert!(!labels.contains(&Value::String("Secret".into())));
        // 規則が無ければ全部見える
        assert_eq!(
            values(
                &mut g,
                "MATCH (n) RETURN count(*)",
                &AccessPolicy::default()
            ),
            vec![Value::Int(3)]
        );
    }

    /// プロパティ索引は読めないノード・値も持っているが、ACL のあるセッションで
    /// 索引を使っても、読めないものには一致しない。
    #[test]
    fn index_lookups_respect_read_rules() {
        let mut g = Graph::new();
        let mut setup = Executor::new(&mut g);
        for q in [
            "CREATE INDEX FOR (n:Account) ON (n.name)",
            "CREATE INDEX FOR (n:Account) ON (n.level)",
            "CREATE INDEX FOR (n:Secret) ON (n.name)",
            "CREATE (:Account {name: 'u', level: 5})",
            "CREATE (:Account {name: 'v', level: 9})",
            "CREATE (:Secret {name: 'u'})",
            // 読めるラベルと読めないラベルの両方を持つノード
            "CREATE (:Account:Secret {name: 'w', level: 7})",
        ] {
            setup.execute_with_params(parse(q), HashMap::new()).unwrap();
        }
        let mut policy = read_deny(&["Secret"]);
        policy
            .read
            .properties
            .entry("Account".to_string())
            .or_default()
            .insert("level".to_string());
        let mut ex = setup.with_access_policy(Some(policy));
        let mut names = |q: &str| {
            let mut v: Vec<String> = ex
                .execute_with_params(parse(q), HashMap::new())
                .unwrap()
                .rows
                .into_iter()
                .map(|r| format!("{:?}", r.columns[0]))
                .collect();
            v.sort();
            v
        };
        let s = |v: &str| format!("{:?}", Value::String(v.into()));

        // 読めるノードは索引で引ける
        assert_eq!(
            names("MATCH (n:Account {name: 'u'}) RETURN n.name"),
            [s("u")]
        );
        assert_eq!(
            names("MATCH (n:Account) WHERE n.name = 'v' RETURN n.name"),
            [s("v")]
        );
        // 読めないラベルのノードは、索引に載っていても一致しない
        assert!(names("MATCH (n:Secret {name: 'u'}) RETURN n.name").is_empty());
        assert!(names("MATCH (n:Account {name: 'w'}) RETURN n.name").is_empty());
        // 読めないプロパティは、等値でも範囲でも一致しない
        assert!(names("MATCH (n:Account {level: 5}) RETURN n.name").is_empty());
        assert!(names("MATCH (n:Account) WHERE n.level = 5 RETURN n.name").is_empty());
        assert!(names("MATCH (n:Account) WHERE n.level > 0 RETURN n.name").is_empty());
        // 読めるプロパティの範囲検索は、読めるノードだけを返す
        assert_eq!(
            names("MATCH (n:Account) WHERE n.name >= 'a' RETURN n.name"),
            [s("u"), s("v")]
        );
    }

    #[test]
    fn deny_all_with_label_exception() {
        let mut g = sample();
        let mut p = AccessPolicy::default();
        p.read.deny_all = true;
        p.read.labels.insert("Public".into(), false);
        assert_eq!(
            values(&mut g, "MATCH (n) RETURN n.name", &p),
            vec![Value::String("a".into())]
        );
    }

    #[test]
    fn read_denied_property_is_null() {
        let mut g = sample();
        let mut p = AccessPolicy::default();
        p.read
            .properties
            .insert("Account".into(), HashSet::from(["password".to_string()]));
        assert_eq!(
            values(&mut g, "MATCH (u:Account) RETURN u.password", &p),
            vec![Value::Null]
        );
        assert_eq!(
            values(&mut g, "MATCH (u:Account) RETURN keys(u)", &p),
            vec![Value::List(vec![Value::String("name".into())])]
        );
        assert!(
            values(
                &mut g,
                "MATCH (u:Account) WHERE u.password = 'p' RETURN u.name",
                &p
            )
            .is_empty()
        );
        // 索引があっても読めない値では一致しない
        run(&mut g, "CREATE INDEX ON :Account(password)", None).unwrap();
        assert!(
            values(
                &mut g,
                "MATCH (u:Account {password: 'p'}) RETURN u.name",
                &p
            )
            .is_empty()
        );
        assert_eq!(
            values(
                &mut g,
                "MATCH (u:Account {password: 'p'}) RETURN u.name",
                &AccessPolicy::default()
            ),
            vec![Value::String("u".into())]
        );
        // 読めないプロパティは書けない
        let err = run(&mut g, "MATCH (u:Account) SET u.password = 'q'", Some(&p)).unwrap_err();
        assert!(matches!(err, ExecuteError::AccessDenied(_)), "{err}");
    }

    #[test]
    fn write_denied_label_fails_the_statement() {
        let mut p = AccessPolicy::default();
        p.write.labels.insert("Secret".into(), true);
        for q in [
            "CREATE (:Secret)",
            "MATCH (n:Secret) SET n.x = 1",
            "MATCH (n:Secret) DETACH DELETE n",
            "MATCH (n:Public) SET n:Secret",
            "MATCH (a:Public), (b:Secret) CREATE (a)-[:S]->(b)",
        ] {
            // 違反した文の変更はサーバーが取り消す。ここでは毎回作り直す
            let mut g = sample();
            let err = run(&mut g, q, Some(&p)).unwrap_err();
            assert!(matches!(err, ExecuteError::AccessDenied(_)), "{q}: {err}");
        }
        let mut g = sample();
        run(&mut g, "CREATE (:Public {name: 'c'})", Some(&p)).unwrap();
        run(&mut g, "MATCH (n:Public) SET n.x = 1", Some(&p)).unwrap();
    }

    #[test]
    fn cannot_delete_node_linked_to_hidden_node() {
        let mut g = sample();
        let p = read_deny(&["Secret"]);
        let err = run(&mut g, "MATCH (n:Public) DETACH DELETE n", Some(&p)).unwrap_err();
        assert!(matches!(err, ExecuteError::AccessDenied(_)), "{err}");
    }

    #[test]
    fn parses_acl_statements() {
        assert_eq!(
            parse("GRANT READ ON :Secret TO ROLE reader"),
            Statement::Acl(AclStatement {
                command: AclCommand::Grant,
                action: AclAction::Read,
                target: AclTarget::Label("Secret".into()),
                grantee: AclGrantee::Role("reader".into()),
            })
        );
        assert_eq!(
            parse("deny write on :Account(password) to user bob"),
            Statement::Acl(AclStatement {
                command: AclCommand::Deny,
                action: AclAction::Write,
                target: AclTarget::Property("Account".into(), "password".into()),
                grantee: AclGrantee::User("bob".into()),
            })
        );
        assert_eq!(
            parse("REVOKE READ ON :* FROM USER bob"),
            Statement::Acl(AclStatement {
                command: AclCommand::Revoke,
                action: AclAction::Read,
                target: AclTarget::AllLabels,
                grantee: AclGrantee::User("bob".into()),
            })
        );
        assert_eq!(parse("SHOW ACL"), Statement::ShowAcl);
        assert!(
            Parser::new("GRANT READ ON :A FROM USER bob")
                .unwrap()
                .parse()
                .is_err()
        );
        assert!(
            Parser::new("REVOKE READ ON :A TO USER bob")
                .unwrap()
                .parse()
                .is_err()
        );
    }
}
