use std::collections::HashMap;

/// クエリ文全体
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Create(CreateClause),
    Match(MatchStatement),
    Delete(DeleteStatement),
    Union(UnionStatement),
    /// MATCH + CREATE 複合クエリ
    MatchCreate(MatchCreateStatement),
    /// MATCH + SET 複合クエリ
    MatchSet(MatchSetStatement),
    /// MERGE句（upsert操作）
    Merge(MergeStatement),
    /// MATCH + REMOVE 複合クエリ
    MatchRemove(MatchRemoveStatement),
    /// UNWIND句
    Unwind(UnwindStatement),
}

/// UNION文
#[derive(Debug, Clone, PartialEq)]
pub struct UnionStatement {
    pub queries: Vec<MatchStatement>,
    pub union_type: UnionType,
}

/// UNIONの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnionType {
    /// 重複排除
    Union,
    /// 重複許容
    UnionAll,
}

/// CREATE文
#[derive(Debug, Clone, PartialEq)]
pub struct CreateClause {
    pub patterns: Vec<Pattern>,
}

/// MATCH句（パターンとオプショナルフラグ）
#[derive(Debug, Clone, PartialEq)]
pub struct MatchClause {
    pub patterns: Vec<Pattern>,
    pub optional: bool,
}

/// MATCH文（MATCH + OPTIONAL MATCH + WHERE + WITH/RETURN）
#[derive(Debug, Clone, PartialEq)]
pub struct MatchStatement {
    /// クエリセグメントのリスト（WITH句で区切られる）
    pub segments: Vec<QuerySegment>,
    /// 最終的なRETURN句
    pub return_clause: ReturnClause,
}

/// クエリセグメント（MATCH + WHERE + WITH）
#[derive(Debug, Clone, PartialEq)]
pub struct QuerySegment {
    /// MATCH句（OPTIONAL MATCH含む）
    pub match_clauses: Vec<MatchClause>,
    /// WHERE句
    pub where_clause: Option<Expression>,
    /// WITH句（最後のセグメント以外で必須）
    pub with_clause: Option<WithClause>,
}

/// WITH句
#[derive(Debug, Clone, PartialEq)]
pub struct WithClause {
    /// DISTINCT かどうか
    pub distinct: bool,
    /// 投影項目
    pub items: Vec<WithItem>,
    /// ORDER BY句
    pub order_by: Option<OrderByClause>,
    /// SKIP句
    pub skip: Option<u64>,
    /// LIMIT句
    pub limit: Option<u64>,
}

/// WITH項目
#[derive(Debug, Clone, PartialEq)]
pub struct WithItem {
    /// 式
    pub expression: ReturnItem,
    /// エイリアス（AS name）
    pub alias: Option<String>,
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
    /// Variable-length path: *min..max (None means exactly 1 hop)
    pub length_range: Option<LengthRange>,
}

/// Length range for variable-length paths
#[derive(Debug, Clone, PartialEq)]
pub struct LengthRange {
    /// Minimum number of hops (default: 1)
    pub min: u32,
    /// Maximum number of hops (None means unlimited)
    pub max: Option<u32>,
}

/// エッジの方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    Outgoing, // ->
    Incoming, // <-
    Both,     // --
}

/// RETURN句
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnClause {
    /// RETURN DISTINCT かどうか
    pub distinct: bool,
    pub items: Vec<ReturnItem>,
    /// ORDER BY句
    pub order_by: Option<OrderByClause>,
    /// SKIP句
    pub skip: Option<u64>,
    /// LIMIT句
    pub limit: Option<u64>,
}

/// ORDER BY句
#[derive(Debug, Clone, PartialEq)]
pub struct OrderByClause {
    pub items: Vec<OrderByItem>,
}

/// ORDER BY項目
#[derive(Debug, Clone, PartialEq)]
pub struct OrderByItem {
    pub expression: OrderByExpression,
    pub direction: OrderDirection,
    pub nulls_order: NullsOrder,
}

/// ORDER BY式
#[derive(Debug, Clone, PartialEq)]
pub enum OrderByExpression {
    /// 変数: ORDER BY n
    Variable(String),
    /// プロパティ: ORDER BY n.name
    Property(String, String),
}

/// ソート方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderDirection {
    #[default]
    Asc,
    Desc,
}

/// NULL値の順序
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullsOrder {
    /// NULL値を先頭に（NULLS FIRST）
    First,
    /// NULL値を末尾に（NULLS LAST）
    Last,
    /// デフォルト（ASCならLAST、DESCならFIRST）
    Default,
}

impl Default for NullsOrder {
    fn default() -> Self {
        NullsOrder::Default
    }
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
    /// スカラー関数: RETURN nodes(r), length(r)
    Function(ScalarFunction),
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

/// スカラー関数（パス操作用）
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarFunction {
    /// nodes(path) - パス内のノードリストを返す
    Nodes(String),
    /// relationships(path) - パス内のエッジリストを返す
    Relationships(String),
    /// length(path) - パスの長さ（ホップ数）を返す
    Length(String),
    /// shortestPath(start, end) - 2ノード間の最短パスを返す
    ShortestPath { start: String, end: String },
    /// allShortestPaths(start, end) - 2ノード間の全最短パスを返す
    AllShortestPaths { start: String, end: String },
}

/// 式
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Literal(Literal),
    Variable(String),
    Property(String, String), // variable.property
    BinaryOp(Box<Expression>, BinaryOp, Box<Expression>),
    UnaryOp(UnaryOp, Box<Expression>),
    /// CASE式
    Case(CaseExpression),
    /// リスト式: [expr, expr, ...]
    List(Vec<Expression>),
}

/// CASE式
#[derive(Debug, Clone, PartialEq)]
pub struct CaseExpression {
    /// 単純CASE式の場合の比較対象（CASE expr WHEN ...）
    pub operand: Option<Box<Expression>>,
    /// WHEN節のリスト
    pub when_clauses: Vec<WhenClause>,
    /// ELSE節（省略可能）
    pub else_clause: Option<Box<Expression>>,
}

/// WHEN節
#[derive(Debug, Clone, PartialEq)]
pub struct WhenClause {
    /// 条件（検索CASE）または比較値（単純CASE）
    pub condition: Expression,
    /// 結果値
    pub result: Expression,
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
    Regex,
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

/// MATCH + CREATE 複合文
#[derive(Debug, Clone, PartialEq)]
pub struct MatchCreateStatement {
    /// MATCHセグメント（WITH句で区切られる）
    pub segments: Vec<QuerySegment>,
    /// WHERE句
    pub where_clause: Option<Expression>,
    /// CREATE句
    pub create_clause: CreateClause,
}

/// MATCH + SET 複合文
#[derive(Debug, Clone, PartialEq)]
pub struct MatchSetStatement {
    /// MATCHセグメント
    pub segments: Vec<QuerySegment>,
    /// WHERE句
    pub where_clause: Option<Expression>,
    /// SET句
    pub set_clause: SetClause,
    /// RETURN句（省略可）
    pub return_clause: Option<ReturnClause>,
}

/// MERGE文
#[derive(Debug, Clone, PartialEq)]
pub struct MergeStatement {
    /// オプショナルなMATCH前段（MATCH + MERGE の組み合わせ）
    pub match_clauses: Vec<MatchClause>,
    /// MATCH用のWHERE句
    pub where_clause: Option<Expression>,
    /// MERGEパターン
    pub patterns: Vec<Pattern>,
    /// ON CREATE SET句
    pub on_create_set: Option<SetClause>,
    /// ON MATCH SET句
    pub on_match_set: Option<SetClause>,
    /// RETURN句（省略可）
    pub return_clause: Option<ReturnClause>,
}

/// MATCH + REMOVE 複合文
#[derive(Debug, Clone, PartialEq)]
pub struct MatchRemoveStatement {
    /// MATCHセグメント
    pub segments: Vec<QuerySegment>,
    /// WHERE句
    pub where_clause: Option<Expression>,
    /// REMOVE句
    pub remove_clause: RemoveClause,
    /// RETURN句（省略可）
    pub return_clause: Option<ReturnClause>,
}

/// REMOVE句
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveClause {
    pub items: Vec<RemoveItem>,
}

/// REMOVE項目
#[derive(Debug, Clone, PartialEq)]
pub enum RemoveItem {
    /// プロパティ削除: REMOVE n.prop
    Property(String, String),
    /// ラベル削除: REMOVE n:Label
    Label(String, String),
}

/// UNWIND文
#[derive(Debug, Clone, PartialEq)]
pub struct UnwindStatement {
    /// UNWIND対象の式
    pub expression: Expression,
    /// AS 変数名
    pub variable: String,
    /// 後続のCREATE句（省略可）
    pub create_clause: Option<CreateClause>,
    /// 後続のSET句（CREATE用）
    pub set_clause: Option<SetClause>,
    /// RETURN句（省略可）
    pub return_clause: Option<ReturnClause>,
}

/// リスト式（UNWIND用）
#[derive(Debug, Clone, PartialEq)]
pub enum ListExpression {
    /// リテラル配列: [1, 2, 3]
    LiteralList(Vec<Literal>),
    /// プロパティアクセス: n.hobbies
    Property(String, String),
}
