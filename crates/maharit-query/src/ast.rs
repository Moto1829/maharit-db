use std::collections::HashMap;

/// クエリ文全体
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Create(CreateClause),
    Match(MatchStatement),
    Delete(DeleteStatement),
}

/// CREATE文
#[derive(Debug, Clone, PartialEq)]
pub struct CreateClause {
    pub patterns: Vec<Pattern>,
}

/// MATCH文（MATCH + WHERE + RETURN）
#[derive(Debug, Clone, PartialEq)]
pub struct MatchStatement {
    pub patterns: Vec<Pattern>,
    pub where_clause: Option<Expression>,
    pub return_clause: ReturnClause,
}

/// DELETE文（MATCH + WHERE + SET + DELETE）
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteStatement {
    pub patterns: Vec<Pattern>,
    pub where_clause: Option<Expression>,
    pub set_clause: Option<SetClause>,
    pub delete_clause: DeleteClause,
}

/// DELETE句
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteClause {
    /// DETACH DELETE かどうか（関連エッジも削除）
    pub detach: bool,
    /// 削除対象の変数名
    pub variables: Vec<String>,
}

/// SET句
#[derive(Debug, Clone, PartialEq)]
pub struct SetClause {
    pub items: Vec<SetItem>,
}

/// SET項目: n.prop = value
#[derive(Debug, Clone, PartialEq)]
pub struct SetItem {
    pub variable: String,
    pub property: String,
    pub value: Expression,
}

/// パターン（ノードとエッジの連鎖）
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Node(NodePattern),
    Path(PathPattern),
}

/// ノードパターン: (variable:Label {props})
#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    pub variable: Option<String>,
    pub label: Option<String>,
    pub properties: HashMap<String, Literal>,
}

/// パスパターン: (a)-[r:TYPE]->(b)
#[derive(Debug, Clone, PartialEq)]
pub struct PathPattern {
    pub start: NodePattern,
    pub segments: Vec<PathSegment>,
}

/// パスの1セグメント: -[r:TYPE]->(node)
#[derive(Debug, Clone, PartialEq)]
pub struct PathSegment {
    pub edge: EdgePattern,
    pub node: NodePattern,
}

/// エッジパターン: [variable:TYPE {props}]
#[derive(Debug, Clone, PartialEq)]
pub struct EdgePattern {
    pub variable: Option<String>,
    pub edge_type: Option<String>,
    pub properties: HashMap<String, Literal>,
    pub direction: EdgeDirection,
}

/// エッジの方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    Outgoing,  // ->
    Incoming,  // <-
    Both,      // --
}

/// RETURN句
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnClause {
    pub items: Vec<ReturnItem>,
}

/// RETURN項目
#[derive(Debug, Clone, PartialEq)]
pub enum ReturnItem {
    /// 変数そのもの: RETURN n
    Variable(String),
    /// プロパティアクセス: RETURN n.name
    Property(String, String),
    /// 全プロパティ: RETURN *
    All,
    /// 集計関数: RETURN COUNT(n)
    Aggregate(AggregateFunction),
}

/// 集計関数
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateFunction {
    /// COUNT(expr) or COUNT(*)
    Count(Option<Box<ReturnItem>>),
    /// SUM(expr)
    Sum(Box<ReturnItem>),
    /// AVG(expr)
    Avg(Box<ReturnItem>),
    /// MIN(expr)
    Min(Box<ReturnItem>),
    /// MAX(expr)
    Max(Box<ReturnItem>),
    /// COLLECT(expr)
    Collect(Box<ReturnItem>),
}

/// 式
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Literal(Literal),
    Variable(String),
    Property(String, String), // variable.property
    BinaryOp(Box<Expression>, BinaryOp, Box<Expression>),
    UnaryOp(UnaryOp, Box<Expression>),
}

/// 二項演算子
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
}

/// 単項演算子
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

/// リテラル値
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}
