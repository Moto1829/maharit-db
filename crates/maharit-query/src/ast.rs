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
    /// FOREACH句
    Foreach(ForeachStatement),
    /// MATCH + FOREACH 複合クエリ
    MatchForeach(MatchForeachStatement),
    /// CREATE CONSTRAINT
    CreateConstraint(CreateConstraintStatement),
    /// DROP CONSTRAINT
    DropConstraint(DropConstraintStatement),
    /// SHOW CONSTRAINTS
    ShowConstraints,
    /// CREATE FULLTEXT INDEX
    CreateFulltextIndex(CreateFulltextIndexStatement),
    /// DROP FULLTEXT INDEX
    DropFulltextIndex(DropFulltextIndexStatement),
    /// CREATE USER
    CreateUser(CreateUserStatement),
    /// DROP USER
    DropUser(DropUserStatement),
    /// ALTER USER
    AlterUser(AlterUserStatement),
    /// SHOW USERS
    ShowUsers,
    /// EXPLAIN文（実行計画の表示）
    Explain(Box<Statement>),
    /// PROFILE文（実行統計付き実行）
    Profile(Box<Statement>),
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

/// サブクエリのパターン部（EXISTSとCOUNTに使用）
#[derive(Debug, Clone, PartialEq)]
pub struct SubqueryPattern {
    /// パターンリスト
    pub patterns: Vec<Pattern>,
    /// WHERE句
    pub where_clause: Option<Expression>,
}

/// COLLECTサブクエリ本体
#[derive(Debug, Clone, PartialEq)]
pub struct CollectSubqueryBody {
    /// パターンリスト
    pub patterns: Vec<Pattern>,
    /// WHERE句
    pub where_clause: Option<Expression>,
    /// RETURN対象の単一項目
    pub return_item: ReturnItem,
}

/// CALLサブクエリ
#[derive(Debug, Clone, PartialEq)]
pub struct CallSubquery {
    /// WITH句でインポートする変数（省略可）
    pub with_import: Option<Vec<String>>,
    /// 内部MATCHパターン
    pub match_clause: MatchClause,
    /// 内部WHERE句
    pub where_clause: Option<Expression>,
    /// 内部RETURN項目（エイリアスあり）
    pub return_items: Vec<CallReturnItem>,
}

/// CALLサブクエリのRETURN項目（エイリアスあり）
#[derive(Debug, Clone, PartialEq)]
pub struct CallReturnItem {
    /// 式
    pub expression: ReturnItem,
    /// エイリアス（AS name）
    pub alias: Option<String>,
}

/// MATCH文（MATCH + OPTIONAL MATCH + WHERE + WITH/RETURN）
#[derive(Debug, Clone, PartialEq)]
pub struct MatchStatement {
    /// クエリセグメントのリスト（WITH句で区切られる）
    pub segments: Vec<QuerySegment>,
    /// CALLサブクエリ（省略可）
    pub call_clause: Option<CallSubquery>,
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

/// SET項目
#[derive(Debug, Clone, PartialEq)]
pub enum SetItem {
    /// プロパティの上書き: n.prop = value
    Property(String, String, Expression),
    /// プロパティのマージ: n += {key: value, ...}
    MergeProperties(String, HashMap<String, Expression>),
    /// ラベルの追加: n:NewLabel
    AddLabel(String, String),
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
    pub properties: HashMap<String, Expression>,
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
    pub properties: HashMap<String, Expression>,
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
    /// 任意の式: RETURN [x IN list | expr], RETURN list[i], etc.
    Expr(Expression),
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
    /// percentileCont(expr, percentile) - 連続パーセンタイル（補間あり）
    PercentileCont(Box<ReturnItem>, Box<ReturnItem>),
    /// percentileDisc(expr, percentile) - 離散パーセンタイル（最近値）
    PercentileDisc(Box<ReturnItem>, Box<ReturnItem>),
    /// stDev(expr) - 標本標準偏差
    StDev(Box<ReturnItem>),
    /// stDevP(expr) - 母標準偏差
    StDevP(Box<ReturnItem>),
    /// COUNT(DISTINCT expr) - 重複排除カウント
    CountDistinct(Box<ReturnItem>),
    /// SUM(DISTINCT expr) - 重複排除合計
    SumDistinct(Box<ReturnItem>),
    /// AVG(DISTINCT expr) - 重複排除平均
    AvgDistinct(Box<ReturnItem>),
    /// COLLECT(DISTINCT expr) - 重複排除コレクト
    CollectDistinct(Box<ReturnItem>),
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
    /// trim(s) - 前後の空白を除去
    Trim(Box<Expression>),
    /// ltrim(s) - 先頭の空白を除去
    LTrim(Box<Expression>),
    /// rtrim(s) - 末尾の空白を除去
    RTrim(Box<Expression>),
    /// toLower(s) - 小文字に変換
    ToLower(Box<Expression>),
    /// toUpper(s) - 大文字に変換
    ToUpper(Box<Expression>),
    /// reverse(s) - 文字列を反転
    Reverse(Box<Expression>),
    /// toString(v) - 文字列に変換
    ToString(Box<Expression>),
    /// size(s) - 文字列の長さ
    Size(Box<Expression>),
    /// left(s, len) - 左からlen文字
    Left(Box<Expression>, Box<Expression>),
    /// right(s, len) - 右からlen文字
    Right(Box<Expression>, Box<Expression>),
    /// substring(s, start, len?) - 部分文字列
    Substring(Box<Expression>, Box<Expression>, Option<Box<Expression>>),
    /// split(s, delim) - 区切り文字で分割
    Split(Box<Expression>, Box<Expression>),
    /// replace(s, search, rep) - 文字列置換
    Replace(Box<Expression>, Box<Expression>, Box<Expression>),
    /// abs(v) - 絶対値
    Abs(Box<Expression>),
    /// ceil(v) - 切り上げ
    Ceil(Box<Expression>),
    /// floor(v) - 切り捨て
    Floor(Box<Expression>),
    /// round(v) or round(v, precision) - 四捨五入
    Round(Box<Expression>, Option<Box<Expression>>),
    /// sign(v) - 符号
    Sign(Box<Expression>),
    /// rand() - 乱数
    Rand,
    /// isNaN(v) - NaN判定
    IsNaN(Box<Expression>),
    /// log(v) - 自然対数
    Log(Box<Expression>),
    /// log10(v) - 常用対数
    Log10(Box<Expression>),
    /// sqrt(v) - 平方根
    Sqrt(Box<Expression>),
    /// e() - ネイピア数
    E,
    /// pi() - 円周率
    Pi,
    // ノード/エッジメタデータ（変数名パターン）
    /// id(v) - ノード/エッジのID
    Id(String),
    /// elementId(v) - 文字列ID
    ElementId(String),
    /// type(r) - エッジタイプ
    Type(String),
    /// startNode(r) - エッジの始点
    StartNode(String),
    /// endNode(r) - エッジの終点
    EndNode(String),
    /// labels(n) - ノードラベルのリスト
    Labels(String),
    /// properties(v) - プロパティMap
    Properties(String),
    /// keys(v) - プロパティキーのリスト
    Keys(String),
    // NULL処理（式パターン）
    /// coalesce(...) - 最初の非NULL値
    Coalesce(Vec<Expression>),
    /// nullIf(a, b) - 等しければNULL
    NullIf(Box<Expression>, Box<Expression>),
    // 型変換（1引数式パターン）
    /// toBoolean(v) - ブール変換
    ToBoolean(Box<Expression>),
    /// toFloat(v) - Float変換
    ToFloat(Box<Expression>),
    /// toInteger(v) - Int変換
    ToInteger(Box<Expression>),
    // ユーティリティ（0引数）
    /// timestamp() - Unixミリ秒
    Timestamp,
    /// randomUUID() - UUID v4
    RandomUUID,
    /// head(list) - 最初の要素
    Head(Box<Expression>),
    /// last(list) - 最後の要素
    Last(Box<Expression>),
    /// tail(list) - 最初を除くリスト
    Tail(Box<Expression>),
    /// range(start, end) or range(start, end, step)
    Range(Box<Expression>, Box<Expression>, Option<Box<Expression>>),
    /// reduce(acc = init, x IN list | body)
    Reduce {
        acc_var: String,
        init: Box<Expression>,
        item_var: String,
        list: Box<Expression>,
        body: Box<Expression>,
    },
}

/// リスト述語関数の種類
#[derive(Debug, Clone, PartialEq)]
pub enum ListPredicateKind {
    All,
    Any,
    None,
    Single,
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
    /// list[index]
    IndexAccess(Box<Expression>, Box<Expression>),
    /// list[start..end]
    ListSlice(Box<Expression>, Box<Expression>, Box<Expression>),
    /// [x IN list WHERE pred | expr]
    ListComprehension {
        variable: String,
        list: Box<Expression>,
        predicate: Option<Box<Expression>>,
        result: Box<Expression>,
    },
    /// パラメータ参照: $name
    Parameter(String),
    /// EXISTS { MATCH pattern } - パターン存在チェック
    ExistsSubquery(Box<SubqueryPattern>),
    /// COUNT { MATCH pattern } - パターンカウント
    CountSubquery(Box<SubqueryPattern>),
    /// COLLECT { MATCH ... RETURN expr } - サブクエリ結果リスト
    CollectSubquery(Box<CollectSubqueryBody>),
    /// all/any/none/single(variable IN list WHERE predicate)
    ListPredicate {
        kind: ListPredicateKind,
        variable: String,
        list: Box<Expression>,
        predicate: Box<Expression>,
    },
    /// exists(expr) - プロパティの存在チェック
    Exists(Box<Expression>),
    /// isEmpty(expr) - 空チェック (list/string)
    IsEmpty(Box<Expression>),
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
    Contains,
    StartsWith,
    EndsWith,
    /// value IN list
    In,
}

/// 単項演算子
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
    IsNormalized,
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

/// FOREACH内の更新操作
#[derive(Debug, Clone, PartialEq)]
pub enum ForeachClause {
    /// CREATE
    Create(CreateClause),
    /// SET
    Set(SetClause),
    /// REMOVE
    Remove(RemoveClause),
    /// DELETE
    Delete(DeleteClause),
    /// MERGE
    Merge(Vec<Pattern>),
    /// ネストしたFOREACH
    Foreach(Box<ForeachStatement>),
}

/// FOREACH文
#[derive(Debug, Clone, PartialEq)]
pub struct ForeachStatement {
    /// イテレーション変数
    pub variable: String,
    /// リスト式
    pub list: Expression,
    /// 更新操作リスト
    pub clauses: Vec<ForeachClause>,
}

/// MATCH + FOREACH 複合文
#[derive(Debug, Clone, PartialEq)]
pub struct MatchForeachStatement {
    /// MATCHセグメント
    pub segments: Vec<QuerySegment>,
    /// FOREACH句
    pub foreach_clause: ForeachStatement,
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

/// CREATE CONSTRAINT文
#[derive(Debug, Clone, PartialEq)]
pub struct CreateConstraintStatement {
    /// 制約名
    pub name: String,
    /// 対象ラベル
    pub label: String,
    /// 対象のノード変数名
    pub variable: String,
    /// 制約の種類
    pub constraint_type: ConstraintTypeAst,
    /// 対象プロパティ（複合制約の場合は複数）
    pub properties: Vec<String>,
}

/// 制約の種類（AST用）
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintTypeAst {
    /// IS UNIQUE
    Unique,
    /// IS NOT NULL
    NotNull,
    /// IS :: TYPE
    TypeCheck(PropertyTypeAst),
}

/// プロパティ型（AST用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyTypeAst {
    Integer,
    Float,
    String,
    Boolean,
}

/// DROP CONSTRAINT文
#[derive(Debug, Clone, PartialEq)]
pub struct DropConstraintStatement {
    /// 制約名
    pub name: String,
}

/// CREATE FULLTEXT INDEX文
#[derive(Debug, Clone, PartialEq)]
pub struct CreateFulltextIndexStatement {
    /// インデックス名
    pub name: String,
    /// 対象ラベル
    pub label: String,
    /// 対象のノード変数名
    pub variable: String,
    /// 対象プロパティリスト
    pub properties: Vec<String>,
}

/// DROP FULLTEXT INDEX文
#[derive(Debug, Clone, PartialEq)]
pub struct DropFulltextIndexStatement {
    /// インデックス名
    pub name: String,
}

/// CREATE USER文
#[derive(Debug, Clone, PartialEq)]
pub struct CreateUserStatement {
    /// ユーザー名
    pub username: String,
    /// パスワード
    pub password: String,
    /// ロール
    pub role: String,
}

/// DROP USER文
#[derive(Debug, Clone, PartialEq)]
pub struct DropUserStatement {
    /// ユーザー名
    pub username: String,
}

/// ALTER USER文
#[derive(Debug, Clone, PartialEq)]
pub struct AlterUserStatement {
    /// ユーザー名
    pub username: String,
    /// 新しいパスワード（省略可）
    pub password: Option<String>,
    /// 新しいロール（省略可）
    pub role: Option<String>,
}
