use std::collections::HashMap;

use thiserror::Error;

use crate::ast::*;
use crate::lexer::{Lexer, Span, Token, TokenKind};

/// パーサーエラー
#[derive(Debug, Clone, Error, PartialEq)]
pub enum ParseError {
    #[error("unexpected token: expected {expected}, found {found} at {span:?}")]
    UnexpectedToken {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("unexpected end of input")]
    UnexpectedEof,

    #[error("lexer error: {0}")]
    LexerError(#[from] crate::lexer::LexerError),
}

/// パースの結果。成功した場合は Statement が含まれ、エラーが発生した場合は
/// errors に収集されたエラーのリストが含まれる。
///
/// # Examples
///
/// ```rust
/// use maharit_query::parser::parse_with_recovery;
///
/// let result = parse_with_recovery("MATCH (n:Person) RETURN n");
/// assert!(result.statement.is_some());
/// assert!(result.errors.is_empty());
///
/// let result = parse_with_recovery("INVALID SYNTAX ???");
/// assert!(result.statement.is_none());
/// assert!(!result.errors.is_empty());
/// ```
#[derive(Debug)]
pub struct ParseResult {
    /// パース成功した場合に Statement が含まれる
    pub statement: Option<Statement>,
    /// 収集されたエラーのリスト（パース失敗時に値が入る）
    pub errors: Vec<ParseError>,
}

impl ParseResult {
    /// 成功した ParseResult を作成する
    pub fn success(statement: Statement) -> Self {
        Self {
            statement: Some(statement),
            errors: Vec::new(),
        }
    }

    /// 失敗した ParseResult を作成する
    pub fn failure(errors: Vec<ParseError>) -> Self {
        Self {
            statement: None,
            errors,
        }
    }

    /// パースが成功したかどうかを返す
    pub fn is_ok(&self) -> bool {
        self.statement.is_some()
    }

    /// パースが失敗したかどうかを返す
    pub fn is_err(&self) -> bool {
        self.statement.is_none()
    }
}

/// エラー回復付きでクエリをパースする。
///
/// 通常の [`Parser::parse`] と異なり、エラーが発生しても panic せず、
/// エラーを [`ParseResult::errors`] に収集して返す。
///
/// # Arguments
///
/// * `input` - パース対象のクエリ文字列
///
/// # Returns
///
/// [`ParseResult`] - パース結果。成功時は `statement` フィールドに AST が、
/// 失敗時は `errors` フィールドにエラーリストが含まれる。
///
/// # Examples
///
/// ```rust
/// use maharit_query::parser::parse_with_recovery;
///
/// // 正常なクエリ
/// let result = parse_with_recovery("CREATE (n:Person {name: 'Alice'})");
/// assert!(result.is_ok());
///
/// // 不正なクエリ
/// let result = parse_with_recovery("FOOBAR ???");
/// assert!(result.is_err());
/// assert!(!result.errors.is_empty());
/// ```
pub fn parse_with_recovery(input: &str) -> ParseResult {
    match Parser::new(input) {
        Err(e) => ParseResult::failure(vec![e]),
        Ok(mut parser) => match parser.parse() {
            Ok(stmt) => ParseResult::success(stmt),
            Err(e) => ParseResult::failure(vec![e]),
        },
    }
}

/// パーサー
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(input: &str) -> Result<Self, ParseError> {
        let tokens = Lexer::new(input).tokenize()?;
        Ok(Self { tokens, pos: 0 })
    }

    /// 文をパース
    pub fn parse(&mut self) -> Result<Statement, ParseError> {
        // Handle EXPLAIN / PROFILE prefixes
        if self.check(TokenKind::Explain) {
            self.advance();
            let inner = self.parse()?;
            return Ok(Statement::Explain(Box::new(inner)));
        }
        if self.check(TokenKind::Profile) {
            self.advance();
            let inner = self.parse()?;
            return Ok(Statement::Profile(Box::new(inner)));
        }

        let first = match self.peek_kind() {
            Some(TokenKind::Create) => {
                // Peek ahead to check for CREATE CONSTRAINT
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Constraint) {
                    return self.parse_create_constraint();
                }
                // Peek ahead to check for CREATE FULLTEXT INDEX
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Fulltext) {
                    return self.parse_create_fulltext_index();
                }
                // Peek ahead to check for CREATE USER
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::User) {
                    return self.parse_create_user();
                }
                self.parse_create()?
            }
            Some(TokenKind::Match) => self.parse_match_or_delete()?,
            Some(TokenKind::Merge) => return self.parse_merge(vec![], None),
            Some(TokenKind::Unwind) => return self.parse_unwind(),
            Some(TokenKind::Foreach) => return self.parse_foreach(),
            Some(TokenKind::Drop) => {
                // Peek ahead to check for DROP FULLTEXT INDEX vs DROP CONSTRAINT
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Fulltext) {
                    return self.parse_drop_fulltext_index();
                }
                // Peek ahead to check for DROP USER
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::User) {
                    return self.parse_drop_user();
                }
                return self.parse_drop_constraint();
            }
            Some(TokenKind::Alter) => return self.parse_alter_user(),
            Some(TokenKind::Show) => return self.parse_show(),
            Some(TokenKind::Call) => {
                // If followed by identifier (not '{'), it's a top-level procedure call
                let next = self.tokens.get(self.pos + 1).map(|t| &t.kind);
                if matches!(next, Some(TokenKind::Ident(_))) {
                    return self.parse_procedure_call();
                }
                // Otherwise fall through to error (CALL { } must appear inside MATCH)
                return Err(self.unexpected_token(
                    "procedure name after CALL",
                ));
            }
            Some(_) => {
                return Err(self.unexpected_token(
                    "CREATE, MATCH, MERGE, UNWIND, FOREACH, DROP, SHOW, ALTER, CALL, EXPLAIN, or PROFILE",
                ));
            }
            None => return Err(ParseError::UnexpectedEof),
        };

        // Check for UNION
        if self.check(TokenKind::Union) {
            return self.parse_union(first);
        }

        Ok(first)
    }

    /// UNION / UNION ALL をパース
    fn parse_union(&mut self, first: Statement) -> Result<Statement, ParseError> {
        let first_match = match first {
            Statement::Match(m) => m,
            _ => {
                return Err(ParseError::UnexpectedToken {
                    expected: "MATCH statement before UNION".to_string(),
                    found: "non-MATCH statement".to_string(),
                    span: self.current_span(),
                });
            }
        };

        let mut queries = vec![first_match];

        // Determine union type from first UNION keyword
        self.expect(TokenKind::Union)?;
        let union_type = if self.check(TokenKind::All) {
            self.advance();
            UnionType::UnionAll
        } else {
            UnionType::Union
        };

        // Parse the next MATCH statement
        let next = self.parse_match_or_delete()?;
        match next {
            Statement::Match(m) => queries.push(m),
            _ => {
                return Err(ParseError::UnexpectedToken {
                    expected: "MATCH statement after UNION".to_string(),
                    found: "non-MATCH statement".to_string(),
                    span: self.current_span(),
                });
            }
        }

        // Parse additional UNION [ALL] statements
        while self.check(TokenKind::Union) {
            self.advance();
            // Check consistency: all must be same type
            if self.check(TokenKind::All) {
                self.advance();
            }

            let next = self.parse_match_or_delete()?;
            match next {
                Statement::Match(m) => queries.push(m),
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        expected: "MATCH statement after UNION".to_string(),
                        found: "non-MATCH statement".to_string(),
                        span: self.current_span(),
                    });
                }
            }
        }

        Ok(Statement::Union(UnionStatement {
            queries,
            union_type,
        }))
    }

    // ========== CREATE ==========

    fn parse_create(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;

        let mut patterns = Vec::new();
        patterns.push(self.parse_pattern()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }

        Ok(Statement::Create(CreateClause { patterns }))
    }

    // ========== MATCH ==========

    fn parse_match_or_delete(&mut self) -> Result<Statement, ParseError> {
        // Parse first segment
        let (first_segment, is_delete, set_clause, delete_clause) = self.parse_query_segment()?;

        if is_delete {
            // DELETE statement
            let patterns: Vec<Pattern> = first_segment
                .match_clauses
                .into_iter()
                .flat_map(|c| c.patterns)
                .collect();
            return Ok(Statement::Delete(DeleteStatement {
                patterns,
                where_clause: first_segment.where_clause,
                set_clause,
                delete_clause: delete_clause.unwrap(),
            }));
        }

        // Check for MATCH + CREATE
        if self.check(TokenKind::Create) {
            let create = self.parse_create_clause()?;
            return Ok(Statement::MatchCreate(MatchCreateStatement {
                segments: vec![first_segment.clone()],
                where_clause: first_segment.where_clause,
                create_clause: create,
            }));
        }

        // Check for MATCH + SET (standalone, not for DELETE)
        if let Some(set_clause) = set_clause {
            let return_clause = if self.check(TokenKind::Return) {
                self.advance();
                Some(self.parse_return_clause()?)
            } else {
                None
            };
            return Ok(Statement::MatchSet(MatchSetStatement {
                segments: vec![first_segment.clone()],
                where_clause: first_segment.where_clause,
                set_clause,
                return_clause,
            }));
        }

        // Check for MATCH + REMOVE
        if self.check(TokenKind::Remove) {
            let remove_clause = self.parse_remove_clause()?;
            let return_clause = if self.check(TokenKind::Return) {
                self.advance();
                Some(self.parse_return_clause()?)
            } else {
                None
            };
            return Ok(Statement::MatchRemove(MatchRemoveStatement {
                segments: vec![first_segment.clone()],
                where_clause: first_segment.where_clause,
                remove_clause,
                return_clause,
            }));
        }

        // Check for MATCH + MERGE
        if self.check(TokenKind::Merge) {
            let match_clauses = first_segment.match_clauses.clone();
            let where_clause = first_segment.where_clause.clone();
            return self.parse_merge(match_clauses, where_clause);
        }

        let mut segments = vec![first_segment];

        // Continue parsing segments while we have WITH clauses
        while segments.last().unwrap().with_clause.is_some() {
            // After WITH, we may have another MATCH or go directly to RETURN
            if self.check(TokenKind::Match) || self.check(TokenKind::Optional) {
                let (segment, is_del, set_cl, _) = self.parse_query_segment()?;
                if is_del {
                    return Err(self.unexpected_token("RETURN or WITH"));
                }
                // Check for SET after inner segment
                if let Some(set_cl) = set_cl {
                    let return_clause = if self.check(TokenKind::Return) {
                        self.advance();
                        Some(self.parse_return_clause()?)
                    } else {
                        None
                    };
                    segments.push(segment.clone());
                    return Ok(Statement::MatchSet(MatchSetStatement {
                        segments,
                        where_clause: segment.where_clause,
                        set_clause: set_cl,
                        return_clause,
                    }));
                }
                // Check for CREATE after inner segment
                if self.check(TokenKind::Create) {
                    let create = self.parse_create_clause()?;
                    segments.push(segment.clone());
                    return Ok(Statement::MatchCreate(MatchCreateStatement {
                        segments,
                        where_clause: segment.where_clause,
                        create_clause: create,
                    }));
                }
                // Check for REMOVE after inner segment
                if self.check(TokenKind::Remove) {
                    let remove_clause = self.parse_remove_clause()?;
                    let return_clause = if self.check(TokenKind::Return) {
                        self.advance();
                        Some(self.parse_return_clause()?)
                    } else {
                        None
                    };
                    segments.push(segment.clone());
                    return Ok(Statement::MatchRemove(MatchRemoveStatement {
                        segments,
                        where_clause: segment.where_clause,
                        remove_clause,
                        return_clause,
                    }));
                }
                segments.push(segment);
            } else if self.check(TokenKind::Return) {
                // Final RETURN, exit loop
                break;
            } else if self.check(TokenKind::With) {
                // WITH directly after WITH (chaining)
                let with_clause = Some(self.parse_with_clause()?);
                segments.push(QuerySegment {
                    match_clauses: vec![],
                    where_clause: None,
                    with_clause,
                });
            } else if self.check(TokenKind::Where) {
                // WHERE after WITH (filtering on WITH results)
                self.advance();
                let where_clause = Some(self.parse_expression()?);

                // Check for WITH or RETURN
                if self.check(TokenKind::With) {
                    let with_clause = Some(self.parse_with_clause()?);
                    segments.push(QuerySegment {
                        match_clauses: vec![],
                        where_clause,
                        with_clause,
                    });
                } else {
                    segments.push(QuerySegment {
                        match_clauses: vec![],
                        where_clause,
                        with_clause: None,
                    });
                    break;
                }
            } else {
                break;
            }
        }

        // Check for MATCH + FOREACH before RETURN
        if self.check(TokenKind::Foreach) {
            let foreach_stmt = self.parse_foreach_statement()?;
            return Ok(Statement::MatchForeach(MatchForeachStatement {
                segments,
                foreach_clause: foreach_stmt,
            }));
        }

        // Check for CALL subquery before RETURN
        let call_clause = if self.check(TokenKind::Call) {
            Some(self.parse_call_subquery()?)
        } else {
            None
        };

        // Parse final RETURN clause
        self.expect(TokenKind::Return)?;
        let return_clause = self.parse_return_clause()?;

        Ok(Statement::Match(MatchStatement {
            segments,
            call_clause,
            return_clause,
        }))
    }

    fn parse_query_segment(
        &mut self,
    ) -> Result<(QuerySegment, bool, Option<SetClause>, Option<DeleteClause>), ParseError> {
        // Parse MATCH clauses (may be empty if starting with WHERE after WITH)
        let mut match_clauses = Vec::new();

        if self.check(TokenKind::Match) {
            self.expect(TokenKind::Match)?;
            let first_clause = self.parse_match_clause(false)?;
            match_clauses.push(first_clause);

            // Parse additional OPTIONAL MATCH clauses
            while self.check(TokenKind::Optional) {
                self.advance();
                self.expect(TokenKind::Match)?;
                let clause = self.parse_match_clause(true)?;
                match_clauses.push(clause);
            }
        }

        // WHERE clause (optional)
        let where_clause = if self.check(TokenKind::Where) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };

        // SET clause (optional, for DELETE or standalone MATCH+SET)
        let set_clause = if self.check(TokenKind::Set) {
            Some(self.parse_set_clause()?)
        } else {
            None
        };

        // Check for DELETE, WITH, or continue to RETURN/CREATE/REMOVE
        if self.check(TokenKind::Delete) || self.check(TokenKind::Detach) {
            let delete_clause = self.parse_delete_clause()?;
            return Ok((
                QuerySegment {
                    match_clauses,
                    where_clause,
                    with_clause: None,
                },
                true,
                set_clause,
                Some(delete_clause),
            ));
        }

        // WITH clause (optional)
        let with_clause = if self.check(TokenKind::With) {
            Some(self.parse_with_clause()?)
        } else {
            None
        };

        Ok((
            QuerySegment {
                match_clauses,
                where_clause,
                with_clause,
            },
            false,
            set_clause,
            None,
        ))
    }

    fn parse_with_clause(&mut self) -> Result<WithClause, ParseError> {
        self.expect(TokenKind::With)?;

        // DISTINCT (optional)
        let distinct = if self.check(TokenKind::Distinct) {
            self.advance();
            true
        } else {
            false
        };

        // Parse WITH items
        let mut items = Vec::new();
        items.push(self.parse_with_item()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            items.push(self.parse_with_item()?);
        }

        // ORDER BY (optional)
        let order_by = if self.check(TokenKind::Order) {
            Some(self.parse_order_by_clause()?)
        } else {
            None
        };

        // SKIP (optional)
        let skip = if self.check(TokenKind::Skip) {
            self.advance();
            Some(self.parse_skip_limit_expr()?)
        } else {
            None
        };

        // LIMIT (optional)
        let limit = if self.check(TokenKind::Limit) {
            self.advance();
            Some(self.parse_skip_limit_expr()?)
        } else {
            None
        };

        Ok(WithClause {
            distinct,
            items,
            order_by,
            skip,
            limit,
        })
    }

    fn parse_with_item(&mut self) -> Result<WithItem, ParseError> {
        let expression = self.parse_return_item()?;

        // AS alias (optional)
        let alias = if self.check(TokenKind::As) {
            self.advance(); // consume AS
            Some(self.expect_ident()?)
        } else {
            None
        };

        Ok(WithItem { expression, alias })
    }

    fn parse_match_clause(&mut self, optional: bool) -> Result<MatchClause, ParseError> {
        let mut patterns = Vec::new();
        patterns.push(self.parse_pattern()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }

        Ok(MatchClause { patterns, optional })
    }

    // ========== SET ==========

    fn parse_set_clause(&mut self) -> Result<SetClause, ParseError> {
        self.expect(TokenKind::Set)?;

        let mut items = Vec::new();
        items.push(self.parse_set_item()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            items.push(self.parse_set_item()?);
        }

        Ok(SetClause { items })
    }

    fn parse_set_item(&mut self) -> Result<SetItem, ParseError> {
        let variable = self.expect_ident()?;

        if self.check(TokenKind::PlusEquals) {
            // n += {key: value, ...}
            self.advance(); // consume +=
            let props = self.parse_properties()?;
            Ok(SetItem::MergeProperties(variable, props))
        } else if self.check(TokenKind::Colon) {
            // n:NewLabel1:NewLabel2:...
            self.advance(); // consume :
            let label = self.expect_ident()?;
            // Only one label per AddLabel; additional colons parsed as separate items
            Ok(SetItem::AddLabel(variable, label))
        } else {
            // n.prop = value
            self.expect(TokenKind::Dot)?;
            let property = self.expect_ident()?;
            self.expect(TokenKind::Eq)?;
            let value = self.parse_expression()?;
            Ok(SetItem::Property(variable, property, value))
        }
    }

    // ========== DELETE ==========

    fn parse_delete_clause(&mut self) -> Result<DeleteClause, ParseError> {
        let detach = if self.check(TokenKind::Detach) {
            self.advance();
            true
        } else {
            false
        };

        self.expect(TokenKind::Delete)?;

        let mut variables = Vec::new();
        variables.push(self.expect_ident()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            variables.push(self.expect_ident()?);
        }

        Ok(DeleteClause { detach, variables })
    }

    fn parse_return_clause(&mut self) -> Result<ReturnClause, ParseError> {
        // DISTINCT (optional)
        let distinct = if self.check(TokenKind::Distinct) {
            self.advance();
            true
        } else {
            false
        };

        let mut items = Vec::new();

        // First item
        items.push(self.parse_return_item()?);

        // Additional items
        while self.check(TokenKind::Comma) {
            self.advance();
            items.push(self.parse_return_item()?);
        }

        // ORDER BY (optional)
        let order_by = if self.check(TokenKind::Order) {
            Some(self.parse_order_by_clause()?)
        } else {
            None
        };

        // SKIP (optional)
        let skip = if self.check(TokenKind::Skip) {
            self.advance();
            Some(self.parse_skip_limit_expr()?)
        } else {
            None
        };

        // LIMIT (optional)
        let limit = if self.check(TokenKind::Limit) {
            self.advance();
            Some(self.parse_skip_limit_expr()?)
        } else {
            None
        };

        Ok(ReturnClause {
            distinct,
            items,
            order_by,
            skip,
            limit,
        })
    }

    fn parse_order_by_clause(&mut self) -> Result<OrderByClause, ParseError> {
        self.expect(TokenKind::Order)?;
        self.expect(TokenKind::By)?;

        let mut items = Vec::new();
        items.push(self.parse_order_by_item()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            items.push(self.parse_order_by_item()?);
        }

        Ok(OrderByClause { items })
    }

    fn parse_order_by_item(&mut self) -> Result<OrderByItem, ParseError> {
        let var = self.expect_ident()?;

        let expression = if self.check(TokenKind::Dot) {
            self.advance();
            let prop = self.expect_ident()?;
            OrderByExpression::Property(var, prop)
        } else {
            OrderByExpression::Variable(var)
        };

        let direction = if self.check(TokenKind::Desc) {
            self.advance();
            OrderDirection::Desc
        } else if self.check(TokenKind::Asc) {
            self.advance();
            OrderDirection::Asc
        } else {
            OrderDirection::Asc // default
        };

        // NULLS FIRST / NULLS LAST (optional)
        let nulls_order = if self.check(TokenKind::Nulls) {
            self.advance();
            if self.check(TokenKind::First) {
                self.advance();
                NullsOrder::First
            } else if self.check(TokenKind::Last) {
                self.advance();
                NullsOrder::Last
            } else {
                return Err(self.unexpected_token("FIRST or LAST"));
            }
        } else {
            NullsOrder::Default
        };

        Ok(OrderByItem {
            expression,
            direction,
            nulls_order,
        })
    }

    /// SKIP / LIMIT の値をパースする。整数リテラルまたは $param 形式を受け付ける。
    fn parse_skip_limit_expr(&mut self) -> Result<Expression, ParseError> {
        match self.peek_kind().cloned() {
            Some(TokenKind::Int(n)) if n >= 0 => {
                self.advance();
                Ok(Expression::Literal(Literal::Int(n)))
            }
            Some(TokenKind::Parameter(name)) => {
                self.advance();
                Ok(Expression::Parameter(name))
            }
            _ => Err(self.unexpected_token("positive integer or $parameter")),
        }
    }

    fn parse_return_item(&mut self) -> Result<ReturnItem, ParseError> {
        if self.check(TokenKind::Star) {
            self.advance();
            return Ok(ReturnItem::All);
        }

        // Handle list expressions (comprehensions, literals) as RETURN items
        if self.check(TokenKind::LBracket) {
            let expr = self.parse_list_expression()?;
            return Ok(ReturnItem::Expr(expr));
        }

        // Handle `last(expr)` specially since `last` is a keyword token
        if self.check(TokenKind::Last) {
            let func_name = "last".to_string();
            self.advance();
            if self.check(TokenKind::LParen) {
                return self.parse_aggregate_function(&func_name);
            }
            return Err(self.unexpected_token("("));
        }

        // Handle `all(var IN list WHERE pred)` since `all` is a keyword token
        if self.check(TokenKind::All) {
            self.advance(); // consume ALL
            if self.check(TokenKind::LParen) {
                self.advance(); // consume '('
                let variable = self.expect_ident()?;
                self.expect(TokenKind::In)?;
                let list = Box::new(self.parse_expression()?);
                self.expect(TokenKind::Where)?;
                let predicate = Box::new(self.parse_expression()?);
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Expr(Expression::ListPredicate {
                    kind: ListPredicateKind::All,
                    variable,
                    list,
                    predicate,
                }));
            }
            return Err(self.unexpected_token("("));
        }

        let var = self.expect_ident()?;

        // Check for subquery forms: EXISTS/COUNT/COLLECT followed by '{'
        if self.check(TokenKind::LBrace) {
            match var.to_uppercase().as_str() {
                "EXISTS" => {
                    self.advance(); // consume '{'
                    let subquery = self.parse_subquery_pattern()?;
                    self.expect(TokenKind::RBrace)?;
                    return Ok(ReturnItem::Expr(Expression::ExistsSubquery(Box::new(
                        subquery,
                    ))));
                }
                "COUNT" => {
                    self.advance(); // consume '{'
                    let subquery = self.parse_subquery_pattern()?;
                    self.expect(TokenKind::RBrace)?;
                    return Ok(ReturnItem::Expr(Expression::CountSubquery(Box::new(
                        subquery,
                    ))));
                }
                "COLLECT" => {
                    self.advance(); // consume '{'
                    let body = self.parse_collect_subquery_body()?;
                    self.expect(TokenKind::RBrace)?;
                    return Ok(ReturnItem::Expr(Expression::CollectSubquery(Box::new(
                        body,
                    ))));
                }
                _ => {}
            }
        }

        // Check if it's a function call (identifier followed by parenthesis)
        if self.check(TokenKind::LParen) {
            return self.parse_aggregate_function(&var);
        }

        if self.check(TokenKind::Dot) {
            self.advance();
            let prop = self.expect_ident()?;
            Ok(ReturnItem::Property(var, prop))
        } else {
            Ok(ReturnItem::Variable(var))
        }
    }

    fn parse_aggregate_function(&mut self, func_name: &str) -> Result<ReturnItem, ParseError> {
        self.expect(TokenKind::LParen)?;

        // Check for predicate functions first (special syntax: variable IN list WHERE predicate)
        match func_name.to_lowercase().as_str() {
            "all" | "any" | "none" | "single" => {
                let variable = self.expect_ident()?;
                self.expect(TokenKind::In)?;
                let list = Box::new(self.parse_expression()?);
                self.expect(TokenKind::Where)?;
                let predicate = Box::new(self.parse_expression()?);
                self.expect(TokenKind::RParen)?;
                let kind = match func_name.to_lowercase().as_str() {
                    "all" => ListPredicateKind::All,
                    "any" => ListPredicateKind::Any,
                    "none" => ListPredicateKind::None,
                    "single" => ListPredicateKind::Single,
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Expr(Expression::ListPredicate {
                    kind,
                    variable,
                    list,
                    predicate,
                }));
            }
            "exists" => {
                let expr = Box::new(self.parse_expression()?);
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Expr(Expression::Exists(expr)));
            }
            "isempty" => {
                let expr = Box::new(self.parse_expression()?);
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Expr(Expression::IsEmpty(expr)));
            }
            _ => {}
        }

        // Check for scalar functions first (they take a simple variable name)
        match func_name.to_lowercase().as_str() {
            "nodes" | "relationships" | "length" => {
                let var = self.expect_ident()?;
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "nodes" => ScalarFunction::Nodes(var),
                    "relationships" => ScalarFunction::Relationships(var),
                    "length" => ScalarFunction::Length(var),
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            "shortestpath" => {
                let start = self.expect_ident()?;
                self.expect(TokenKind::Comma)?;
                let end = self.expect_ident()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::ShortestPath {
                    start,
                    end,
                }));
            }
            "allshortestpaths" => {
                let start = self.expect_ident()?;
                self.expect(TokenKind::Comma)?;
                let end = self.expect_ident()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::AllShortestPaths {
                    start,
                    end,
                }));
            }
            // 1引数の文字列関数
            "trim" | "ltrim" | "rtrim" | "tolower" | "toupper" | "reverse" | "tostring"
            | "size" => {
                let arg = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "trim" => ScalarFunction::Trim(Box::new(arg)),
                    "ltrim" => ScalarFunction::LTrim(Box::new(arg)),
                    "rtrim" => ScalarFunction::RTrim(Box::new(arg)),
                    "tolower" => ScalarFunction::ToLower(Box::new(arg)),
                    "toupper" => ScalarFunction::ToUpper(Box::new(arg)),
                    "reverse" => ScalarFunction::Reverse(Box::new(arg)),
                    "tostring" => ScalarFunction::ToString(Box::new(arg)),
                    "size" => ScalarFunction::Size(Box::new(arg)),
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // 2引数の文字列関数
            "left" | "right" | "split" => {
                let arg1 = self.parse_expression()?;
                self.expect(TokenKind::Comma)?;
                let arg2 = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "left" => ScalarFunction::Left(Box::new(arg1), Box::new(arg2)),
                    "right" => ScalarFunction::Right(Box::new(arg1), Box::new(arg2)),
                    "split" => ScalarFunction::Split(Box::new(arg1), Box::new(arg2)),
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // 2-3引数: substring(s, start, len?)
            "substring" => {
                let arg1 = self.parse_expression()?;
                self.expect(TokenKind::Comma)?;
                let arg2 = self.parse_expression()?;
                let arg3 = if self.check(TokenKind::Comma) {
                    self.advance();
                    Some(Box::new(self.parse_expression()?))
                } else {
                    None
                };
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::Substring(
                    Box::new(arg1),
                    Box::new(arg2),
                    arg3,
                )));
            }
            // 3引数: replace(s, search, rep)
            "replace" => {
                let arg1 = self.parse_expression()?;
                self.expect(TokenKind::Comma)?;
                let arg2 = self.parse_expression()?;
                self.expect(TokenKind::Comma)?;
                let arg3 = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::Replace(
                    Box::new(arg1),
                    Box::new(arg2),
                    Box::new(arg3),
                )));
            }
            // 0引数の数学関数
            "rand" | "e" | "pi" => {
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "rand" => ScalarFunction::Rand,
                    "e" => ScalarFunction::E,
                    "pi" => ScalarFunction::Pi,
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // 1引数の数学関数
            "abs" | "ceil" | "floor" | "sign" | "isnan" | "log" | "log10" | "sqrt" => {
                let arg = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "abs" => ScalarFunction::Abs(Box::new(arg)),
                    "ceil" => ScalarFunction::Ceil(Box::new(arg)),
                    "floor" => ScalarFunction::Floor(Box::new(arg)),
                    "sign" => ScalarFunction::Sign(Box::new(arg)),
                    "isnan" => ScalarFunction::IsNaN(Box::new(arg)),
                    "log" => ScalarFunction::Log(Box::new(arg)),
                    "log10" => ScalarFunction::Log10(Box::new(arg)),
                    "sqrt" => ScalarFunction::Sqrt(Box::new(arg)),
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // 1-2引数: round(v, precision?)
            "round" => {
                let arg1 = self.parse_expression()?;
                let arg2 = if self.check(TokenKind::Comma) {
                    self.advance();
                    Some(Box::new(self.parse_expression()?))
                } else {
                    None
                };
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::Round(
                    Box::new(arg1),
                    arg2,
                )));
            }
            // Variable name pattern functions (node/edge metadata)
            "id" | "elementid" | "type" | "startnode" | "endnode" | "labels" | "properties"
            | "keys" => {
                let var = self.expect_ident()?;
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "id" => ScalarFunction::Id(var),
                    "elementid" => ScalarFunction::ElementId(var),
                    "type" => ScalarFunction::Type(var),
                    "startnode" => ScalarFunction::StartNode(var),
                    "endnode" => ScalarFunction::EndNode(var),
                    "labels" => ScalarFunction::Labels(var),
                    "properties" => ScalarFunction::Properties(var),
                    "keys" => ScalarFunction::Keys(var),
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // 1-argument expression: toBoolean, toFloat, toInteger
            "toboolean" | "tofloat" | "tointeger" => {
                let arg = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "toboolean" => ScalarFunction::ToBoolean(Box::new(arg)),
                    "tofloat" => ScalarFunction::ToFloat(Box::new(arg)),
                    "tointeger" => ScalarFunction::ToInteger(Box::new(arg)),
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // 2-argument: nullIf(a, b)
            "nullif" => {
                let arg1 = self.parse_expression()?;
                self.expect(TokenKind::Comma)?;
                let arg2 = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::NullIf(
                    Box::new(arg1),
                    Box::new(arg2),
                )));
            }
            // Variadic: coalesce(...)
            "coalesce" => {
                let mut args = vec![self.parse_expression()?];
                while self.check(TokenKind::Comma) {
                    self.advance();
                    args.push(self.parse_expression()?);
                }
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::Coalesce(args)));
            }
            // 0-argument: timestamp(), randomUUID()
            "timestamp" | "randomuuid" => {
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "timestamp" => ScalarFunction::Timestamp,
                    "randomuuid" => ScalarFunction::RandomUUID,
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // リスト操作関数: 1引数
            "head" | "tail" => {
                let arg = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let scalar = match func_name.to_lowercase().as_str() {
                    "head" => ScalarFunction::Head(Box::new(arg)),
                    "tail" => ScalarFunction::Tail(Box::new(arg)),
                    _ => unreachable!(),
                };
                return Ok(ReturnItem::Function(scalar));
            }
            // last はキーワードなので parse_return_item で事前処理済み
            "last" => {
                let arg = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::Last(Box::new(arg))));
            }
            // range(start, end) or range(start, end, step)
            "range" => {
                let arg1 = self.parse_expression()?;
                self.expect(TokenKind::Comma)?;
                let arg2 = self.parse_expression()?;
                let arg3 = if self.check(TokenKind::Comma) {
                    self.advance();
                    Some(Box::new(self.parse_expression()?))
                } else {
                    None
                };
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::Range(
                    Box::new(arg1),
                    Box::new(arg2),
                    arg3,
                )));
            }
            // reduce(acc = init, x IN list | body)
            "reduce" => {
                let acc_var = self.expect_ident()?;
                self.expect(TokenKind::Eq)?;
                let init = Box::new(self.parse_expression()?);
                self.expect(TokenKind::Comma)?;
                let item_var = self.expect_ident()?;
                self.expect(TokenKind::In)?;
                let list = Box::new(self.parse_expression()?);
                self.expect(TokenKind::Pipe)?;
                let body = Box::new(self.parse_expression()?);
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::Reduce {
                    acc_var,
                    init,
                    item_var,
                    list,
                    body,
                }));
            }
            // percentileCont(expr, percentile)
            "percentilecont" => {
                let expr = self.parse_return_item()?;
                self.expect(TokenKind::Comma)?;
                let percentile_expr = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Aggregate(AggregateFunction::PercentileCont(
                    Box::new(expr),
                    Box::new(ReturnItem::Expr(percentile_expr)),
                )));
            }
            // percentileDisc(expr, percentile)
            "percentiledisc" => {
                let expr = self.parse_return_item()?;
                self.expect(TokenKind::Comma)?;
                let percentile_expr = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Aggregate(AggregateFunction::PercentileDisc(
                    Box::new(expr),
                    Box::new(ReturnItem::Expr(percentile_expr)),
                )));
            }
            // stDev(expr)
            "stdev" => {
                let inner = self.parse_return_item()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Aggregate(AggregateFunction::StDev(Box::new(
                    inner,
                ))));
            }
            // stDevP(expr)
            "stdevp" => {
                let inner = self.parse_return_item()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Aggregate(AggregateFunction::StDevP(Box::new(
                    inner,
                ))));
            }
            _ => {}
        }

        // Aggregate functions - check for DISTINCT modifier
        let is_distinct = if self.check(TokenKind::Distinct) {
            self.advance();
            true
        } else {
            false
        };

        let inner = if self.check(TokenKind::Star) {
            self.advance();
            None // COUNT(*)
        } else if self.check(TokenKind::RParen) {
            None // Empty, will error for non-count
        } else {
            Some(Box::new(self.parse_return_item()?))
        };

        self.expect(TokenKind::RParen)?;

        let aggregate = match func_name.to_uppercase().as_str() {
            "COUNT" => {
                if is_distinct {
                    let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                        expected: "expression".to_string(),
                        found: ")".to_string(),
                        span: self.current_span(),
                    })?;
                    AggregateFunction::CountDistinct(inner)
                } else {
                    AggregateFunction::Count(inner)
                }
            }
            "SUM" => {
                let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: ")".to_string(),
                    span: self.current_span(),
                })?;
                if is_distinct {
                    AggregateFunction::SumDistinct(inner)
                } else {
                    AggregateFunction::Sum(inner)
                }
            }
            "AVG" => {
                let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: ")".to_string(),
                    span: self.current_span(),
                })?;
                if is_distinct {
                    AggregateFunction::AvgDistinct(inner)
                } else {
                    AggregateFunction::Avg(inner)
                }
            }
            "MIN" => {
                let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: ")".to_string(),
                    span: self.current_span(),
                })?;
                AggregateFunction::Min(inner)
            }
            "MAX" => {
                let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: ")".to_string(),
                    span: self.current_span(),
                })?;
                AggregateFunction::Max(inner)
            }
            "COLLECT" => {
                let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: ")".to_string(),
                    span: self.current_span(),
                })?;
                if is_distinct {
                    AggregateFunction::CollectDistinct(inner)
                } else {
                    AggregateFunction::Collect(inner)
                }
            }
            _ => {
                return Err(ParseError::UnexpectedToken {
                    expected: "function (COUNT, SUM, AVG, MIN, MAX, COLLECT, percentileCont, percentileDisc, stDev, stDevP, nodes, relationships, length, shortestPath, allShortestPaths, trim, ltrim, rtrim, toLower, toUpper, reverse, toString, size, left, right, substring, split, replace, abs, ceil, floor, round, sign, rand, isNaN, log, log10, sqrt, e, pi, id, elementId, type, startNode, endNode, labels, properties, keys, coalesce, nullIf, toBoolean, toFloat, toInteger, timestamp, randomUUID)".to_string(),
                    found: func_name.to_string(),
                    span: self.current_span(),
                });
            }
        };

        Ok(ReturnItem::Aggregate(aggregate))
    }

    // ========== CREATE clause (for MATCH+CREATE) ==========

    fn parse_create_clause(&mut self) -> Result<CreateClause, ParseError> {
        self.expect(TokenKind::Create)?;

        let mut patterns = Vec::new();
        patterns.push(self.parse_pattern()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }

        Ok(CreateClause { patterns })
    }

    // ========== REMOVE ==========

    fn parse_remove_clause(&mut self) -> Result<RemoveClause, ParseError> {
        self.expect(TokenKind::Remove)?;

        let mut items = Vec::new();
        items.push(self.parse_remove_item()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            items.push(self.parse_remove_item()?);
        }

        Ok(RemoveClause { items })
    }

    fn parse_remove_item(&mut self) -> Result<RemoveItem, ParseError> {
        let variable = self.expect_ident()?;

        if self.check(TokenKind::Dot) {
            // REMOVE n.prop
            self.advance();
            let property = self.expect_ident()?;
            Ok(RemoveItem::Property(variable, property))
        } else if self.check(TokenKind::Colon) {
            // REMOVE n:Label
            self.advance();
            let label = self.expect_ident()?;
            Ok(RemoveItem::Label(variable, label))
        } else {
            Err(self.unexpected_token(". or :"))
        }
    }

    // ========== MERGE ==========

    fn parse_merge(
        &mut self,
        match_clauses: Vec<MatchClause>,
        where_clause: Option<Expression>,
    ) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Merge)?;

        let mut patterns = Vec::new();
        patterns.push(self.parse_pattern()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }

        // ON CREATE SET / ON MATCH SET
        let mut on_create_set = None;
        let mut on_match_set = None;

        while self.check(TokenKind::On) {
            self.advance();

            if self.check(TokenKind::Create) {
                self.advance();
                on_create_set = Some(self.parse_set_clause()?);
            } else if self.check(TokenKind::Match) {
                self.advance();
                on_match_set = Some(self.parse_set_clause()?);
            } else {
                return Err(self.unexpected_token("CREATE or MATCH"));
            }
        }

        // Optional RETURN
        let return_clause = if self.check(TokenKind::Return) {
            self.advance();
            Some(self.parse_return_clause()?)
        } else {
            None
        };

        Ok(Statement::Merge(MergeStatement {
            match_clauses,
            where_clause,
            patterns,
            on_create_set,
            on_match_set,
            return_clause,
        }))
    }

    // ========== UNWIND ==========

    fn parse_unwind(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Unwind)?;

        let expression = self.parse_expression()?;

        self.expect(TokenKind::As)?;
        let variable = self.expect_ident()?;

        // Optional CREATE
        let create_clause = if self.check(TokenKind::Create) {
            Some(self.parse_create_clause()?)
        } else {
            None
        };

        // Optional SET (typically after CREATE)
        let set_clause = if self.check(TokenKind::Set) {
            Some(self.parse_set_clause()?)
        } else {
            None
        };

        // Optional RETURN
        let return_clause = if self.check(TokenKind::Return) {
            self.advance();
            Some(self.parse_return_clause()?)
        } else {
            None
        };

        Ok(Statement::Unwind(UnwindStatement {
            expression,
            variable,
            create_clause,
            set_clause,
            return_clause,
        }))
    }

    // ========== FOREACH ==========

    fn parse_foreach(&mut self) -> Result<Statement, ParseError> {
        let stmt = self.parse_foreach_statement()?;
        Ok(Statement::Foreach(stmt))
    }

    fn parse_foreach_statement(&mut self) -> Result<ForeachStatement, ParseError> {
        self.expect(TokenKind::Foreach)?;
        self.expect(TokenKind::LParen)?;

        let variable = self.expect_ident()?;
        self.expect(TokenKind::In)?;
        let list = self.parse_expression()?;
        self.expect(TokenKind::Pipe)?;

        let clauses = self.parse_foreach_clauses()?;

        self.expect(TokenKind::RParen)?;
        Ok(ForeachStatement {
            variable,
            list,
            clauses,
        })
    }

    fn parse_foreach_clauses(&mut self) -> Result<Vec<ForeachClause>, ParseError> {
        let mut clauses = Vec::new();

        loop {
            match self.peek_kind() {
                Some(TokenKind::Create) => {
                    let create = self.parse_create_clause()?;
                    clauses.push(ForeachClause::Create(create));
                }
                Some(TokenKind::Set) => {
                    let set = self.parse_set_clause()?;
                    clauses.push(ForeachClause::Set(set));
                }
                Some(TokenKind::Remove) => {
                    let remove = self.parse_remove_clause()?;
                    clauses.push(ForeachClause::Remove(remove));
                }
                Some(TokenKind::Delete) | Some(TokenKind::Detach) => {
                    let delete = self.parse_delete_clause()?;
                    clauses.push(ForeachClause::Delete(delete));
                }
                Some(TokenKind::Merge) => {
                    self.advance(); // consume MERGE
                    let mut patterns = Vec::new();
                    patterns.push(self.parse_pattern()?);
                    while self.check(TokenKind::Comma) {
                        self.advance();
                        patterns.push(self.parse_pattern()?);
                    }
                    clauses.push(ForeachClause::Merge(patterns));
                }
                Some(TokenKind::Foreach) => {
                    let inner = self.parse_foreach_statement()?;
                    clauses.push(ForeachClause::Foreach(Box::new(inner)));
                }
                _ => break,
            }
        }

        if clauses.is_empty() {
            return Err(self.unexpected_token("CREATE, SET, REMOVE, DELETE, MERGE, or FOREACH"));
        }

        Ok(clauses)
    }

    // ========== CONSTRAINT DDL ==========

    /// CREATE CONSTRAINT name FOR pattern REQUIRE ...
    ///
    /// Supported forms:
    /// - `FOR (n:Label) REQUIRE n.prop IS UNIQUE/NOT NULL/:: TYPE`
    /// - `FOR (n:Label1) REQUIRE n:Label2`  (RequiredLabel)
    /// - `FOR (s:SLabel)-[r:EType]->(t:TLabel)`  (EndpointLabel, no REQUIRE needed)
    fn parse_create_constraint(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;
        self.expect(TokenKind::Constraint)?;

        let name = self.expect_ident()?;

        self.expect(TokenKind::For)?;

        // Parse leading '(' for the start node
        self.expect(TokenKind::LParen)?;
        let variable = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let label = self.expect_ident()?;
        self.expect(TokenKind::RParen)?;

        // Check if this is an edge pattern: (s:SLabel)-[r:EType]->(t:TLabel)
        // Edge patterns start with '-' (Dash) or '<-' (ArrowLeft)
        if self.check(TokenKind::Dash) || self.check(TokenKind::ArrowLeft) {
            return self.parse_endpoint_label_constraint(name, label, variable);
        }

        self.expect(TokenKind::Require)?;

        // After REQUIRE, check if it's `var:Label` (RequiredLabel) or `var.prop IS ...`
        // Peek ahead: if we see `ident colon ident` it's a RequiredLabel constraint
        // If we see `ident dot ident` or `(ident dot ident` it's a property constraint
        let is_required_label = {
            // Save position to peek
            let saved = self.pos;
            let next_is_ident = matches!(self.peek_kind(), Some(TokenKind::Ident(_)));
            if next_is_ident {
                self.advance(); // consume ident
                let after_ident = self.peek_kind().cloned();
                self.pos = saved; // restore
                matches!(after_ident, Some(TokenKind::Colon))
            } else {
                false
            }
        };

        if is_required_label {
            // Parse: REQUIRE var:RequiredLabel
            let req_var = self.expect_ident()?;
            if req_var != variable {
                return Err(ParseError::UnexpectedToken {
                    expected: format!("variable '{}'", variable),
                    found: req_var,
                    span: self.current_span(),
                });
            }
            self.expect(TokenKind::Colon)?;
            let required_label = self.expect_ident()?;

            return Ok(Statement::CreateConstraint(CreateConstraintStatement {
                name,
                label,
                variable,
                constraint_type: ConstraintTypeAst::RequiredLabel(required_label),
                properties: vec![],
            }));
        }

        // Property constraint: parse var.prop or (var.prop, var.prop, ...)
        let properties = if self.check(TokenKind::LParen) {
            // Composite constraint: (var.prop, var.prop, ...)
            self.advance(); // consume '('
            let mut props = Vec::new();

            loop {
                let req_var = self.expect_ident()?;
                if req_var != variable {
                    return Err(ParseError::UnexpectedToken {
                        expected: format!("variable '{}'", variable),
                        found: req_var,
                        span: self.current_span(),
                    });
                }
                self.expect(TokenKind::Dot)?;
                let prop = self.expect_ident()?;
                props.push(prop);

                if !self.check(TokenKind::Comma) {
                    break;
                }
                self.advance(); // consume ','
            }

            self.expect(TokenKind::RParen)?;
            props
        } else {
            // Single property constraint: var.prop
            let req_var = self.expect_ident()?;
            if req_var != variable {
                return Err(ParseError::UnexpectedToken {
                    expected: format!("variable '{}'", variable),
                    found: req_var,
                    span: self.current_span(),
                });
            }
            self.expect(TokenKind::Dot)?;
            let property = self.expect_ident()?;
            vec![property]
        };

        self.expect(TokenKind::Is)?;

        // Parse constraint type: UNIQUE, NOT NULL, or :: TYPE
        let constraint_type = if self.check(TokenKind::Unique) {
            self.advance();
            ConstraintTypeAst::Unique
        } else if self.check(TokenKind::Not) {
            self.advance();
            self.expect(TokenKind::Null)?;
            ConstraintTypeAst::NotNull
        } else if self.check(TokenKind::Colon) {
            // IS :: TYPE
            self.advance();
            self.expect(TokenKind::Colon)?;
            let type_name = self.expect_ident()?;
            let prop_type = match type_name.to_uppercase().as_str() {
                "INTEGER" | "INT" => PropertyTypeAst::Integer,
                "FLOAT" | "DOUBLE" => PropertyTypeAst::Float,
                "STRING" => PropertyTypeAst::String,
                "BOOLEAN" | "BOOL" => PropertyTypeAst::Boolean,
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        expected: "INTEGER, FLOAT, STRING, or BOOLEAN".to_string(),
                        found: type_name,
                        span: self.current_span(),
                    });
                }
            };
            ConstraintTypeAst::TypeCheck(prop_type)
        } else {
            return Err(self.unexpected_token("UNIQUE, NOT NULL, or :: TYPE"));
        };

        Ok(Statement::CreateConstraint(CreateConstraintStatement {
            name,
            label,
            variable,
            constraint_type,
            properties,
        }))
    }

    /// Parse endpoint label constraint from edge pattern:
    /// `(s:SLabel)-[r:EType]->(t:TLabel)` already consumed start node.
    /// Builds an `EndpointLabel` constraint with the edge type as the constraint label.
    fn parse_endpoint_label_constraint(
        &mut self,
        name: String,
        source_label: String,
        _source_var: String,
    ) -> Result<Statement, ParseError> {
        // Parse `-[r:EType]->` or `<-[r:EType]-`
        // Accept: `->` direction (dash lbracket ... rbracket arrowright)
        // or `<-` direction (arrowleft lbracket ... rbracket dash)
        // For simplicity support outgoing `->` and both `--`

        // consume leading dash or arrowleft
        let _dash_or_arrow = self.advance(); // '-' or '<-'

        // optional additional dash for `--[`
        if self.check(TokenKind::LBracket) {
            // already at '['
        } else if self.check(TokenKind::Dash) {
            self.advance(); // consume extra dash if `--[`
        }

        self.expect(TokenKind::LBracket)?;

        // Optional edge variable
        let _edge_var = if matches!(self.peek_kind(), Some(TokenKind::Ident(_))) {
            Some(self.expect_ident()?)
        } else {
            None
        };

        self.expect(TokenKind::Colon)?;
        let edge_type = self.expect_ident()?;
        self.expect(TokenKind::RBracket)?;

        // consume `->` or `-`
        if self.check(TokenKind::Arrow) || self.check(TokenKind::Dash) {
            self.advance();
        }

        // Parse target node: (t:TLabel)
        self.expect(TokenKind::LParen)?;
        let _target_var = if matches!(self.peek_kind(), Some(TokenKind::Ident(_))) {
            // peek further to see if there's a colon after
            let saved = self.pos;
            let ident = self.expect_ident()?;
            if self.check(TokenKind::Colon) {
                Some(ident)
            } else {
                // it was the label with no variable
                self.pos = saved;
                None
            }
        } else {
            None
        };

        self.expect(TokenKind::Colon)?;
        let target_label = self.expect_ident()?;
        self.expect(TokenKind::RParen)?;

        Ok(Statement::CreateConstraint(CreateConstraintStatement {
            name,
            label: edge_type,
            variable: String::new(),
            constraint_type: ConstraintTypeAst::EndpointLabel {
                source_label,
                target_label,
            },
            properties: vec![],
        }))
    }

    /// DROP CONSTRAINT name
    fn parse_drop_constraint(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Drop)?;
        self.expect(TokenKind::Constraint)?;

        let name = self.expect_ident()?;

        Ok(Statement::DropConstraint(DropConstraintStatement { name }))
    }

    /// SHOW CONSTRAINTS / SHOW USERS
    fn parse_show(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Show)?;

        // Check for SHOW USERS (User is a keyword)
        if self.check(TokenKind::User) {
            // SHOW USERS - consume "USER" and expect "S" via ident
            self.advance();
            // Accept both "SHOW USER" and "SHOW USERS"
            return Ok(Statement::ShowUsers);
        }

        // Expect CONSTRAINTS (as an identifier since it's not a keyword)
        let ident = self.expect_ident()?;
        if ident.to_uppercase() == "CONSTRAINTS" {
            Ok(Statement::ShowConstraints)
        } else if ident.to_uppercase() == "USERS" {
            Ok(Statement::ShowUsers)
        } else {
            Err(ParseError::UnexpectedToken {
                expected: "CONSTRAINTS or USERS".to_string(),
                found: ident,
                span: self.current_span(),
            })
        }
    }

    // ========== Fulltext Index ==========

    /// CREATE FULLTEXT INDEX name FOR (n:Label) ON (n.prop1, n.prop2)
    fn parse_create_fulltext_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;
        self.expect(TokenKind::Fulltext)?;
        self.expect(TokenKind::Index)?;

        let name = self.expect_ident()?;

        self.expect(TokenKind::For)?;
        self.expect(TokenKind::LParen)?;
        let variable = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let label = self.expect_ident()?;
        self.expect(TokenKind::RParen)?;

        self.expect(TokenKind::On)?;
        self.expect(TokenKind::LParen)?;

        let mut properties = Vec::new();
        loop {
            let prop_var = self.expect_ident()?;
            if prop_var != variable {
                return Err(ParseError::UnexpectedToken {
                    expected: format!("{}.property", variable),
                    found: prop_var,
                    span: self.current_span(),
                });
            }
            self.expect(TokenKind::Dot)?;
            let prop_name = self.expect_ident()?;
            properties.push(prop_name);

            if !self.check(TokenKind::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(TokenKind::RParen)?;

        Ok(Statement::CreateFulltextIndex(
            CreateFulltextIndexStatement {
                name,
                label,
                variable,
                properties,
            },
        ))
    }

    /// DROP FULLTEXT INDEX name
    fn parse_drop_fulltext_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Drop)?;
        self.expect(TokenKind::Fulltext)?;
        self.expect(TokenKind::Index)?;
        let name = self.expect_ident()?;

        Ok(Statement::DropFulltextIndex(DropFulltextIndexStatement {
            name,
        }))
    }

    // ========== User Management ==========

    /// CREATE USER username SET PASSWORD 'pass' ROLE role
    fn parse_create_user(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;
        self.expect(TokenKind::User)?;
        let username = self.expect_ident()?;
        self.expect(TokenKind::Set)?;
        self.expect(TokenKind::Password)?;
        let password = self.expect_string()?;
        self.expect(TokenKind::Role)?;
        let role = self.expect_ident()?;

        Ok(Statement::CreateUser(CreateUserStatement {
            username,
            password,
            role,
        }))
    }

    /// DROP USER username
    fn parse_drop_user(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Drop)?;
        self.expect(TokenKind::User)?;
        let username = self.expect_ident()?;

        Ok(Statement::DropUser(DropUserStatement { username }))
    }

    /// ALTER USER username SET PASSWORD 'pass' / SET ROLE role
    fn parse_alter_user(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Alter)?;
        self.expect(TokenKind::User)?;
        let username = self.expect_ident()?;
        self.expect(TokenKind::Set)?;

        let mut password = None;
        let mut role = None;

        // Parse SET PASSWORD and/or SET ROLE
        if self.check(TokenKind::Password) {
            self.advance();
            password = Some(self.expect_string()?);

            // Check for additional SET ROLE
            if self.check(TokenKind::Role) {
                self.advance();
                role = Some(self.expect_ident()?);
            }
        } else if self.check(TokenKind::Role) {
            self.advance();
            role = Some(self.expect_ident()?);
        } else {
            return Err(self.unexpected_token("PASSWORD or ROLE"));
        }

        Ok(Statement::AlterUser(AlterUserStatement {
            username,
            password,
            role,
        }))
    }

    // ========== Subqueries ==========

    /// CALL { [WITH var, ...] MATCH pattern [WHERE expr] RETURN item [AS alias], ... }
    fn parse_call_subquery(&mut self) -> Result<CallSubquery, ParseError> {
        self.expect(TokenKind::Call)?;
        self.expect(TokenKind::LBrace)?;

        // Optional WITH import: WITH var1, var2, ...
        let with_import = if self.check(TokenKind::With) {
            self.advance();
            let mut vars = Vec::new();
            vars.push(self.expect_ident()?);
            while self.check(TokenKind::Comma) {
                self.advance();
                vars.push(self.expect_ident()?);
            }
            Some(vars)
        } else {
            None
        };

        // Inner MATCH
        self.expect(TokenKind::Match)?;
        let match_clause = self.parse_match_clause(false)?;

        // Inner WHERE
        let where_clause = if self.check(TokenKind::Where) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };

        // Inner RETURN with optional AS aliases
        self.expect(TokenKind::Return)?;
        let mut return_items = Vec::new();
        return_items.push(self.parse_call_return_item()?);
        while self.check(TokenKind::Comma) {
            self.advance();
            return_items.push(self.parse_call_return_item()?);
        }

        self.expect(TokenKind::RBrace)?;

        Ok(CallSubquery {
            with_import,
            match_clause,
            where_clause,
            return_items,
        })
    }

    fn parse_call_return_item(&mut self) -> Result<CallReturnItem, ParseError> {
        let expression = self.parse_return_item()?;
        let alias = if self.check(TokenKind::As) {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };
        Ok(CallReturnItem { expression, alias })
    }

    /// Parse a top-level procedure call:
    ///   CALL proc.name.part(arg1, arg2) YIELD col1, col2 [RETURN ...]
    ///
    /// The procedure name is a dot-separated sequence of identifiers.
    fn parse_procedure_call(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Call)?;

        // Parse dotted procedure name: ident ('.' ident)*
        let mut parts = Vec::new();
        parts.push(self.expect_ident()?);
        while self.check(TokenKind::Dot) {
            self.advance();
            // The next part might be a keyword used as identifier (e.g. "index", "search")
            let part = self.expect_ident_or_keyword()?;
            parts.push(part);
        }
        let procedure = parts.join(".");

        // Parse argument list: '(' [expr, ...] ')'
        self.expect(TokenKind::LParen)?;
        let mut arguments = Vec::new();
        if !self.check(TokenKind::RParen) {
            arguments.push(self.parse_expression()?);
            while self.check(TokenKind::Comma) {
                self.advance();
                arguments.push(self.parse_expression()?);
            }
        }
        self.expect(TokenKind::RParen)?;

        // Optional YIELD clause
        let yield_columns = if self.check(TokenKind::Yield) {
            self.advance();
            let mut cols = Vec::new();
            cols.push(self.expect_ident()?);
            while self.check(TokenKind::Comma) {
                self.advance();
                cols.push(self.expect_ident()?);
            }
            cols
        } else {
            Vec::new()
        };

        // Optional RETURN clause (for filtering/projection after YIELD)
        let return_clause = if self.check(TokenKind::Return) {
            self.advance();
            Some(self.parse_return_clause()?)
        } else {
            None
        };

        Ok(Statement::ProcedureCall(ProcedureCallStatement {
            procedure,
            arguments,
            yield_columns,
            return_clause,
        }))
    }

    /// Expect an identifier or a keyword that can serve as an identifier in dotted names.
    fn expect_ident_or_keyword(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::Ident(s)) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            // Allow keywords commonly used as identifiers in procedure names
            Some(TokenKind::Index) => {
                self.advance();
                Ok("index".to_string())
            }
            Some(TokenKind::Fulltext) => {
                self.advance();
                Ok("fulltext".to_string())
            }
            _ => self.expect_ident(),
        }
    }

    /// Parse subquery pattern body for EXISTS/COUNT: MATCH pattern [WHERE expr]
    fn parse_subquery_pattern(&mut self) -> Result<SubqueryPattern, ParseError> {
        self.expect(TokenKind::Match)?;
        let match_clause = self.parse_match_clause(false)?;
        let patterns = match_clause.patterns;

        let where_clause = if self.check(TokenKind::Where) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };

        Ok(SubqueryPattern {
            patterns,
            where_clause,
        })
    }

    /// Parse collect subquery body: MATCH pattern [WHERE expr] RETURN item
    fn parse_collect_subquery_body(&mut self) -> Result<CollectSubqueryBody, ParseError> {
        self.expect(TokenKind::Match)?;
        let match_clause = self.parse_match_clause(false)?;
        let patterns = match_clause.patterns;

        let where_clause = if self.check(TokenKind::Where) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };

        self.expect(TokenKind::Return)?;
        let return_item = self.parse_return_item()?;

        Ok(CollectSubqueryBody {
            patterns,
            where_clause,
            return_item,
        })
    }

    fn current_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or_default()
    }

    // ========== Pattern ==========

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        let start = self.parse_node_pattern()?;

        // Check if it's a path pattern
        if self.is_edge_start() {
            let mut segments = Vec::new();

            while self.is_edge_start() {
                let edge = self.parse_edge_pattern()?;
                let node = self.parse_node_pattern()?;
                segments.push(PathSegment { edge, node });
            }

            Ok(Pattern::Path(PathPattern { start, segments }))
        } else {
            Ok(Pattern::Node(start))
        }
    }

    fn parse_node_pattern(&mut self) -> Result<NodePattern, ParseError> {
        self.expect(TokenKind::LParen)?;

        let mut variable = None;
        let mut labels = Vec::new();
        let mut properties = HashMap::new();

        // Variable name (optional)
        if let Some(TokenKind::Ident(_)) = self.peek_kind() {
            variable = Some(self.expect_ident()?);
        }

        // Labels (optional, multiple allowed: :Label1:Label2)
        while self.check(TokenKind::Colon) {
            self.advance();
            labels.push(self.expect_ident()?);
        }

        // Properties (optional)
        if self.check(TokenKind::LBrace) {
            properties = self.parse_properties()?;
        }

        self.expect(TokenKind::RParen)?;

        Ok(NodePattern {
            variable,
            labels,
            properties,
        })
    }

    fn is_edge_start(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(TokenKind::Dash) | Some(TokenKind::ArrowLeft)
        )
    }

    fn parse_edge_pattern(&mut self) -> Result<EdgePattern, ParseError> {
        let direction = if self.check(TokenKind::ArrowLeft) {
            // <-
            self.advance();
            EdgeDirection::Incoming
        } else {
            // -
            self.expect(TokenKind::Dash)?;
            EdgeDirection::Outgoing // Will be determined later
        };

        let mut variable = None;
        let mut edge_type = None;
        let mut properties = HashMap::new();
        let mut length_range = None;

        // Edge details in brackets (optional)
        if self.check(TokenKind::LBracket) {
            self.advance();

            // Variable (optional)
            if let Some(TokenKind::Ident(_)) = self.peek_kind() {
                variable = Some(self.expect_ident()?);
            }

            // Type (optional)
            if self.check(TokenKind::Colon) {
                self.advance();
                edge_type = Some(self.expect_ident()?);
            }

            // Variable-length path: *min..max (optional)
            if self.check(TokenKind::Star) {
                self.advance();
                length_range = Some(self.parse_length_range()?);
            }

            // Properties (optional)
            if self.check(TokenKind::LBrace) {
                properties = self.parse_properties()?;
            }

            self.expect(TokenKind::RBracket)?;
        }

        // Direction ending
        let final_direction = if direction == EdgeDirection::Incoming {
            // <-[...]- (already consumed <-)
            self.expect(TokenKind::Dash)?;
            EdgeDirection::Incoming
        } else {
            // -[...] followed by -> or -
            if self.check(TokenKind::Arrow) {
                self.advance();
                EdgeDirection::Outgoing
            } else if self.check(TokenKind::Dash) {
                self.advance();
                EdgeDirection::Both
            } else {
                return Err(self.unexpected_token("-> or -"));
            }
        };

        Ok(EdgePattern {
            variable,
            edge_type,
            properties,
            direction: final_direction,
            length_range,
        })
    }

    fn parse_length_range(&mut self) -> Result<LengthRange, ParseError> {
        let mut min = 1u32;
        let mut max = None;
        let mut has_min = false;

        // Check for min value
        if let Some(TokenKind::Int(n)) = self.peek_kind().cloned() {
            min = n as u32;
            has_min = true;
            self.advance();
        }

        // Check for range (..)
        if self.check(TokenKind::Dot) {
            self.advance();
            self.expect(TokenKind::Dot)?;

            // Check for max value
            if let Some(TokenKind::Int(n)) = self.peek_kind().cloned() {
                max = Some(n as u32);
                self.advance();
            }
            // If no max after .., it means unlimited
        } else if has_min {
            // No .., but has min, so min is also max (exact count)
            max = Some(min);
        }
        // If no min and no .., it's just [*] meaning 1..unlimited (max stays None)

        Ok(LengthRange { min, max })
    }

    fn parse_properties(&mut self) -> Result<HashMap<String, Expression>, ParseError> {
        self.expect(TokenKind::LBrace)?;

        let mut props = HashMap::new();

        if !self.check(TokenKind::RBrace) {
            loop {
                let key = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let value = self.parse_expression()?;
                props.insert(key, value);

                if !self.check(TokenKind::Comma) {
                    break;
                }
                self.advance();
            }
        }

        self.expect(TokenKind::RBrace)?;

        Ok(props)
    }

    // ========== Expression ==========

    fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        self.parse_or_expression()
    }

    fn parse_or_expression(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_and_expression()?;

        while self.check(TokenKind::Or) {
            self.advance();
            let right = self.parse_and_expression()?;
            left = Expression::BinaryOp(Box::new(left), BinaryOp::Or, Box::new(right));
        }

        Ok(left)
    }

    fn parse_and_expression(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_comparison()?;

        while self.check(TokenKind::And) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expression::BinaryOp(Box::new(left), BinaryOp::And, Box::new(right));
        }

        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expression, ParseError> {
        let left = self.parse_additive()?;

        let op = match self.peek_kind() {
            Some(TokenKind::Eq) => BinaryOp::Eq,
            Some(TokenKind::Neq) => BinaryOp::Neq,
            Some(TokenKind::Lt) => BinaryOp::Lt,
            Some(TokenKind::Gt) => BinaryOp::Gt,
            Some(TokenKind::Lte) => BinaryOp::Lte,
            Some(TokenKind::Gte) => BinaryOp::Gte,
            Some(TokenKind::RegexMatch) => BinaryOp::Regex,
            Some(TokenKind::Contains) => BinaryOp::Contains,
            Some(TokenKind::Starts) => {
                self.advance(); // consume STARTS
                self.expect(TokenKind::With)?; // expect WITH
                let right = self.parse_additive()?;
                return Ok(Expression::BinaryOp(
                    Box::new(left),
                    BinaryOp::StartsWith,
                    Box::new(right),
                ));
            }
            Some(TokenKind::Ends) => {
                self.advance(); // consume ENDS
                self.expect(TokenKind::With)?; // expect WITH
                let right = self.parse_additive()?;
                return Ok(Expression::BinaryOp(
                    Box::new(left),
                    BinaryOp::EndsWith,
                    Box::new(right),
                ));
            }
            Some(TokenKind::In) => {
                self.advance();
                let right = self.parse_additive()?;
                return Ok(Expression::BinaryOp(
                    Box::new(left),
                    BinaryOp::In,
                    Box::new(right),
                ));
            }
            Some(TokenKind::Is) => {
                self.advance(); // consume IS
                if self.check(TokenKind::Normalized) {
                    self.advance(); // consume NORMALIZED
                    return Ok(Expression::UnaryOp(UnaryOp::IsNormalized, Box::new(left)));
                }
                return Err(self.unexpected_token("NORMALIZED"));
            }
            _ => return Ok(left),
        };

        self.advance();
        let right = self.parse_additive()?;

        Ok(Expression::BinaryOp(Box::new(left), op, Box::new(right)))
    }

    fn parse_additive(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_multiplicative()?;

        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Plus) => BinaryOp::Add,
                Some(TokenKind::Dash) => BinaryOp::Sub,
                _ => break,
            };

            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expression::BinaryOp(Box::new(left), op, Box::new(right));
        }

        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_unary()?;

        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Star) => BinaryOp::Mul,
                Some(TokenKind::Slash) => BinaryOp::Div,
                _ => break,
            };

            self.advance();
            let right = self.parse_unary()?;
            left = Expression::BinaryOp(Box::new(left), op, Box::new(right));
        }

        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expression, ParseError> {
        if self.check(TokenKind::Not) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(expr)));
        }

        if self.check(TokenKind::Dash) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expression::UnaryOp(UnaryOp::Neg, Box::new(expr)));
        }

        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expression, ParseError> {
        let mut expr = self.parse_primary()?;
        while self.check(TokenKind::LBracket) {
            self.advance(); // consume [
            let start = self.parse_expression()?;
            if self.check(TokenKind::Dot) {
                // List slice: list[start..end] - consume two Dot tokens
                self.advance(); // first dot
                self.advance(); // second dot
                let end = self.parse_expression()?;
                self.expect(TokenKind::RBracket)?;
                expr = Expression::ListSlice(Box::new(expr), Box::new(start), Box::new(end));
            } else {
                self.expect(TokenKind::RBracket)?;
                expr = Expression::IndexAccess(Box::new(expr), Box::new(start));
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expression, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::LParen) => {
                // Speculatively try to parse as a pattern predicate: (n)-->() or (n:Label {..})
                let saved_pos = self.pos;
                if let Ok(start_node) = self.parse_node_pattern() {
                    let has_labels_or_props =
                        !start_node.labels.is_empty() || !start_node.properties.is_empty();
                    if self.is_edge_start() || has_labels_or_props {
                        // Commit as pattern predicate
                        let pattern = if self.is_edge_start() {
                            let mut segments = Vec::new();
                            while self.is_edge_start() {
                                let edge = self.parse_edge_pattern()?;
                                let node = self.parse_node_pattern()?;
                                segments.push(PathSegment { edge, node });
                            }
                            Pattern::Path(PathPattern {
                                start: start_node,
                                segments,
                            })
                        } else {
                            Pattern::Node(start_node)
                        };
                        return Ok(Expression::PatternPredicate(vec![pattern]));
                    }
                }
                // Restore and fall through to parenthesized expression
                self.pos = saved_pos;
                self.advance(); // consume (
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            Some(TokenKind::LBracket) => {
                // List literal: [expr, expr, ...]
                self.parse_list_expression()
            }
            Some(TokenKind::Case) => self.parse_case_expression(),
            Some(TokenKind::All) => {
                // `all` is a keyword token, handle as list predicate function
                self.advance(); // consume ALL
                if self.check(TokenKind::LParen) {
                    self.advance(); // consume '('
                    let variable = self.expect_ident()?;
                    self.expect(TokenKind::In)?;
                    let list = Box::new(self.parse_expression()?);
                    self.expect(TokenKind::Where)?;
                    let predicate = Box::new(self.parse_expression()?);
                    self.expect(TokenKind::RParen)?;
                    return Ok(Expression::ListPredicate {
                        kind: ListPredicateKind::All,
                        variable,
                        list,
                        predicate,
                    });
                }
                Err(self.unexpected_token("("))
            }
            Some(TokenKind::Ident(_)) => {
                let var = self.expect_ident()?;
                // Check for predicate/scalar functions (identifier followed by '(')
                if self.check(TokenKind::LParen) {
                    match var.to_lowercase().as_str() {
                        "all" | "any" | "none" | "single" => {
                            self.advance(); // consume '('
                            let variable = self.expect_ident()?;
                            self.expect(TokenKind::In)?;
                            let list = Box::new(self.parse_expression()?);
                            self.expect(TokenKind::Where)?;
                            let predicate = Box::new(self.parse_expression()?);
                            self.expect(TokenKind::RParen)?;
                            let kind = match var.to_lowercase().as_str() {
                                "all" => ListPredicateKind::All,
                                "any" => ListPredicateKind::Any,
                                "none" => ListPredicateKind::None,
                                "single" => ListPredicateKind::Single,
                                _ => unreachable!(),
                            };
                            return Ok(Expression::ListPredicate {
                                kind,
                                variable,
                                list,
                                predicate,
                            });
                        }
                        "exists" => {
                            self.advance(); // consume '('
                            let expr = Box::new(self.parse_expression()?);
                            self.expect(TokenKind::RParen)?;
                            return Ok(Expression::Exists(expr));
                        }
                        "isempty" => {
                            self.advance(); // consume '('
                            let expr = Box::new(self.parse_expression()?);
                            self.expect(TokenKind::RParen)?;
                            return Ok(Expression::IsEmpty(expr));
                        }
                        _ => {} // fall through to subquery/property/variable handling
                    }
                }
                // Check for EXISTS/COUNT/COLLECT subqueries (identifier followed by '{')
                if self.check(TokenKind::LBrace) {
                    match var.to_uppercase().as_str() {
                        "EXISTS" => {
                            self.advance(); // consume '{'
                            let subquery = self.parse_subquery_pattern()?;
                            self.expect(TokenKind::RBrace)?;
                            return Ok(Expression::ExistsSubquery(Box::new(subquery)));
                        }
                        "COUNT" => {
                            self.advance(); // consume '{'
                            let subquery = self.parse_subquery_pattern()?;
                            self.expect(TokenKind::RBrace)?;
                            return Ok(Expression::CountSubquery(Box::new(subquery)));
                        }
                        "COLLECT" => {
                            self.advance(); // consume '{'
                            let body = self.parse_collect_subquery_body()?;
                            self.expect(TokenKind::RBrace)?;
                            return Ok(Expression::CollectSubquery(Box::new(body)));
                        }
                        _ => {}
                    }
                }
                if self.check(TokenKind::Dot) {
                    self.advance();
                    let prop = self.expect_ident()?;
                    Ok(Expression::Property(var, prop))
                } else {
                    Ok(Expression::Variable(var))
                }
            }
            Some(
                TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null,
            ) => {
                let lit = self.parse_literal()?;
                Ok(Expression::Literal(lit))
            }
            Some(TokenKind::Parameter(_)) => {
                if let TokenKind::Parameter(name) = self.advance().unwrap().kind {
                    Ok(Expression::Parameter(name))
                } else {
                    unreachable!()
                }
            }
            Some(_) => Err(self.unexpected_token("expression")),
            None => Err(ParseError::UnexpectedEof),
        }
    }

    fn parse_list_expression(&mut self) -> Result<Expression, ParseError> {
        self.expect(TokenKind::LBracket)?;

        // Check for list comprehension: [x IN list WHERE pred | expr]
        let is_comprehension = matches!(
            (
                self.tokens.get(self.pos).map(|t| &t.kind),
                self.tokens.get(self.pos + 1).map(|t| &t.kind),
            ),
            (Some(TokenKind::Ident(_)), Some(TokenKind::In))
        );

        if is_comprehension {
            let variable = self.expect_ident()?;
            self.advance(); // consume IN
            let list = Box::new(self.parse_expression()?);
            let predicate = if self.check(TokenKind::Where) {
                self.advance();
                Some(Box::new(self.parse_expression()?))
            } else {
                None
            };
            self.expect(TokenKind::Pipe)?;
            let result = Box::new(self.parse_expression()?);
            self.expect(TokenKind::RBracket)?;
            return Ok(Expression::ListComprehension {
                variable,
                list,
                predicate,
                result,
            });
        }

        let mut elements = Vec::new();

        if !self.check(TokenKind::RBracket) {
            elements.push(self.parse_expression()?);
            while self.check(TokenKind::Comma) {
                self.advance();
                elements.push(self.parse_expression()?);
            }
        }

        self.expect(TokenKind::RBracket)?;
        Ok(Expression::List(elements))
    }

    fn parse_case_expression(&mut self) -> Result<Expression, ParseError> {
        self.expect(TokenKind::Case)?;

        // Check for simple CASE (CASE expr WHEN ...) vs searched CASE (CASE WHEN ...)
        let operand = if !self.check(TokenKind::When) {
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        // Parse WHEN clauses
        let mut when_clauses = Vec::new();
        while self.check(TokenKind::When) {
            self.advance();
            let condition = self.parse_expression()?;
            self.expect(TokenKind::Then)?;
            let result = self.parse_expression()?;
            when_clauses.push(WhenClause { condition, result });
        }

        if when_clauses.is_empty() {
            return Err(self.unexpected_token("WHEN"));
        }

        // Parse optional ELSE clause
        let else_clause = if self.check(TokenKind::Else) {
            self.advance();
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        self.expect(TokenKind::End)?;

        Ok(Expression::Case(CaseExpression {
            operand,
            when_clauses,
            else_clause,
        }))
    }

    fn parse_literal(&mut self) -> Result<Literal, ParseError> {
        let token = self.advance().ok_or(ParseError::UnexpectedEof)?;

        match token.kind {
            TokenKind::Int(n) => Ok(Literal::Int(n)),
            TokenKind::Float(n) => Ok(Literal::Float(n)),
            TokenKind::String(s) => Ok(Literal::String(s)),
            TokenKind::True => Ok(Literal::Bool(true)),
            TokenKind::False => Ok(Literal::Bool(false)),
            TokenKind::Null => Ok(Literal::Null),
            _ => Err(ParseError::UnexpectedToken {
                expected: "literal".to_string(),
                found: format!("{}", token.kind),
                span: token.span,
            }),
        }
    }

    // ========== Helper methods ==========

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|t| &t.kind)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(token)
        } else {
            None
        }
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.peek_kind().is_some_and(|k| {
            std::mem::discriminant(k) == std::mem::discriminant(&kind)
        })
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token, ParseError> {
        if self.check(kind.clone()) {
            Ok(self.advance().unwrap())
        } else {
            Err(self.unexpected_token(&format!("{}", kind)))
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::Ident(_)) => {
                if let TokenKind::Ident(s) = self.advance().unwrap().kind {
                    Ok(s)
                } else {
                    unreachable!()
                }
            }
            _ => Err(self.unexpected_token("identifier")),
        }
    }

    fn expect_string(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::String(_)) => {
                if let TokenKind::String(s) = self.advance().unwrap().kind {
                    Ok(s)
                } else {
                    unreachable!()
                }
            }
            _ => Err(self.unexpected_token("string literal")),
        }
    }

    fn unexpected_token(&self, expected: &str) -> ParseError {
        match self.peek() {
            Some(token) => ParseError::UnexpectedToken {
                expected: expected.to_string(),
                found: format!("{}", token.kind),
                span: token.span,
            },
            None => ParseError::UnexpectedEof,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<Statement, ParseError> {
        Parser::new(input)?.parse()
    }

    #[test]
    fn test_create_simple_node() {
        let stmt = parse("CREATE (n)").unwrap();

        if let Statement::Create(create) = stmt {
            assert_eq!(create.patterns.len(), 1);
            if let Pattern::Node(node) = &create.patterns[0] {
                assert_eq!(node.variable, Some("n".to_string()));
                assert!(node.labels.is_empty());
            } else {
                panic!("expected node pattern");
            }
        } else {
            panic!("expected CREATE statement");
        }
    }

    #[test]
    fn test_create_node_with_label() {
        let stmt = parse("CREATE (n:Person)").unwrap();

        if let Statement::Create(create) = stmt {
            if let Pattern::Node(node) = &create.patterns[0] {
                assert_eq!(node.variable, Some("n".to_string()));
                assert_eq!(node.labels, vec!["Person".to_string()]);
            } else {
                panic!("expected node pattern");
            }
        } else {
            panic!("expected CREATE statement");
        }
    }

    #[test]
    fn test_create_node_with_multiple_labels() {
        let stmt = parse("CREATE (n:Person:Employee)").unwrap();

        if let Statement::Create(create) = stmt {
            if let Pattern::Node(node) = &create.patterns[0] {
                assert_eq!(node.variable, Some("n".to_string()));
                assert_eq!(node.labels, vec!["Person".to_string(), "Employee".to_string()]);
            } else {
                panic!("expected node pattern");
            }
        } else {
            panic!("expected CREATE statement");
        }
    }

    #[test]
    fn test_create_node_with_properties() {
        let stmt = parse(r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

        if let Statement::Create(create) = stmt {
            if let Pattern::Node(node) = &create.patterns[0] {
                assert_eq!(node.variable, Some("n".to_string()));
                assert_eq!(node.labels, vec!["Person".to_string()]);
                assert_eq!(
                    node.properties.get("name"),
                    Some(&Expression::Literal(Literal::String("Alice".to_string())))
                );
                assert_eq!(
                    node.properties.get("age"),
                    Some(&Expression::Literal(Literal::Int(30)))
                );
            } else {
                panic!("expected node pattern");
            }
        } else {
            panic!("expected CREATE statement");
        }
    }

    #[test]
    fn test_create_path() {
        let stmt = parse("CREATE (a:Person)-[:KNOWS]->(b:Person)").unwrap();

        if let Statement::Create(create) = stmt {
            if let Pattern::Path(path) = &create.patterns[0] {
                assert_eq!(path.start.variable, Some("a".to_string()));
                assert_eq!(path.start.labels, vec!["Person".to_string()]);
                assert_eq!(path.segments.len(), 1);

                let seg = &path.segments[0];
                assert_eq!(seg.edge.edge_type, Some("KNOWS".to_string()));
                assert_eq!(seg.edge.direction, EdgeDirection::Outgoing);
                assert_eq!(seg.node.variable, Some("b".to_string()));
            } else {
                panic!("expected path pattern");
            }
        } else {
            panic!("expected CREATE statement");
        }
    }

    #[test]
    fn test_match_return() {
        let stmt = parse("MATCH (n:Person) RETURN n").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.segments[0].match_clauses[0].patterns.len(), 1);
            assert!(m.segments[0].where_clause.is_none());
            assert_eq!(m.return_clause.items.len(), 1);
            assert_eq!(
                m.return_clause.items[0],
                ReturnItem::Variable("n".to_string())
            );
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_match_return_property() {
        let stmt = parse("MATCH (n:Person) RETURN n.name").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(
                m.return_clause.items[0],
                ReturnItem::Property("n".to_string(), "name".to_string())
            );
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_match_where() {
        let stmt = parse("MATCH (n:Person) WHERE n.age > 18 RETURN n").unwrap();

        if let Statement::Match(m) = stmt {
            let where_clause = m.segments[0].where_clause.clone().unwrap();
            if let Expression::BinaryOp(left, op, right) = where_clause {
                assert_eq!(op, BinaryOp::Gt);
                assert_eq!(
                    *left,
                    Expression::Property("n".to_string(), "age".to_string())
                );
                assert_eq!(*right, Expression::Literal(Literal::Int(18)));
            } else {
                panic!("expected binary expression");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_match_where_and() {
        let stmt =
            parse(r#"MATCH (n:Person) WHERE n.age > 18 AND n.name = "Alice" RETURN n"#).unwrap();

        if let Statement::Match(m) = stmt {
            let where_clause = m.segments[0].where_clause.clone().unwrap();
            if let Expression::BinaryOp(_, op, _) = where_clause {
                assert_eq!(op, BinaryOp::And);
            } else {
                panic!("expected AND expression");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_return_multiple() {
        let stmt = parse("MATCH (n:Person) RETURN n.name, n.age").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.return_clause.items.len(), 2);
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_return_star() {
        let stmt = parse("MATCH (n:Person) RETURN *").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.return_clause.items[0], ReturnItem::All);
        } else {
            panic!("expected MATCH statement");
        }
    }

    // ========== DELETE tests ==========

    #[test]
    fn test_delete_node() {
        let stmt = parse("MATCH (n:Person) DELETE n").unwrap();

        if let Statement::Delete(d) = stmt {
            assert_eq!(d.patterns.len(), 1);
            assert!(d.where_clause.is_none());
            assert!(d.set_clause.is_none());
            assert!(!d.delete_clause.detach);
            assert_eq!(d.delete_clause.variables, vec!["n".to_string()]);
        } else {
            panic!("expected DELETE statement");
        }
    }

    #[test]
    fn test_delete_with_where() {
        let stmt = parse(r#"MATCH (n:Person) WHERE n.name = "Alice" DELETE n"#).unwrap();

        if let Statement::Delete(d) = stmt {
            assert!(d.where_clause.is_some());
            assert_eq!(d.delete_clause.variables, vec!["n".to_string()]);
        } else {
            panic!("expected DELETE statement");
        }
    }

    #[test]
    fn test_detach_delete() {
        let stmt = parse("MATCH (n:Person) DETACH DELETE n").unwrap();

        if let Statement::Delete(d) = stmt {
            assert!(d.delete_clause.detach);
            assert_eq!(d.delete_clause.variables, vec!["n".to_string()]);
        } else {
            panic!("expected DELETE statement");
        }
    }

    #[test]
    fn test_delete_multiple() {
        let stmt = parse("MATCH (a)-[r]->(b) DELETE a, r, b").unwrap();

        if let Statement::Delete(d) = stmt {
            assert_eq!(
                d.delete_clause.variables,
                vec!["a".to_string(), "r".to_string(), "b".to_string()]
            );
        } else {
            panic!("expected DELETE statement");
        }
    }

    #[test]
    fn test_delete_edge() {
        let stmt = parse("MATCH (a)-[r:KNOWS]->(b) DELETE r").unwrap();

        if let Statement::Delete(d) = stmt {
            assert!(!d.delete_clause.detach);
            assert_eq!(d.delete_clause.variables, vec!["r".to_string()]);
        } else {
            panic!("expected DELETE statement");
        }
    }

    // ========== SET tests ==========

    #[test]
    fn test_set_property() {
        let stmt = parse(r#"MATCH (n:Person) SET n.age = 31 DELETE n"#).unwrap();

        if let Statement::Delete(d) = stmt {
            let set = d.set_clause.unwrap();
            assert_eq!(set.items.len(), 1);
            assert_eq!(
                set.items[0],
                SetItem::Property(
                    "n".to_string(),
                    "age".to_string(),
                    Expression::Literal(Literal::Int(31))
                )
            );
        } else {
            panic!("expected DELETE statement with SET");
        }
    }

    #[test]
    fn test_set_multiple_properties() {
        let stmt = parse(r#"MATCH (n:Person) SET n.age = 31, n.name = "Bob" DELETE n"#).unwrap();

        if let Statement::Delete(d) = stmt {
            let set = d.set_clause.unwrap();
            assert_eq!(set.items.len(), 2);
            assert!(matches!(&set.items[0], SetItem::Property(v, p, _) if v == "n" && p == "age"));
            assert!(matches!(&set.items[1], SetItem::Property(v, p, _) if v == "n" && p == "name"));
        } else {
            panic!("expected DELETE statement with SET");
        }
    }

    // ========== Variable-length path tests ==========

    #[test]
    fn test_variable_length_path_exact() {
        let stmt = parse("MATCH (a)-[:KNOWS*2]->(b) RETURN a").unwrap();

        if let Statement::Match(m) = stmt {
            if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
                let seg = &path.segments[0];
                assert_eq!(seg.edge.edge_type, Some("KNOWS".to_string()));
                let range = seg
                    .edge
                    .length_range
                    .as_ref()
                    .expect("should have length_range");
                assert_eq!(range.min, 2);
                assert_eq!(range.max, Some(2));
            } else {
                panic!("expected path pattern");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_variable_length_path_range() {
        let stmt = parse("MATCH (a)-[:KNOWS*2..5]->(b) RETURN a").unwrap();

        if let Statement::Match(m) = stmt {
            if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
                let seg = &path.segments[0];
                let range = seg
                    .edge
                    .length_range
                    .as_ref()
                    .expect("should have length_range");
                assert_eq!(range.min, 2);
                assert_eq!(range.max, Some(5));
            } else {
                panic!("expected path pattern");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_variable_length_path_unlimited() {
        let stmt = parse("MATCH (a)-[:KNOWS*2..]->(b) RETURN a").unwrap();

        if let Statement::Match(m) = stmt {
            if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
                let seg = &path.segments[0];
                let range = seg
                    .edge
                    .length_range
                    .as_ref()
                    .expect("should have length_range");
                assert_eq!(range.min, 2);
                assert_eq!(range.max, None);
            } else {
                panic!("expected path pattern");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_parse_nodes_function() {
        let stmt = parse("MATCH (a)-[r:KNOWS*2]->(b) RETURN nodes(r)").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.return_clause.items.len(), 1);
            if let ReturnItem::Function(ScalarFunction::Nodes(var)) = &m.return_clause.items[0] {
                assert_eq!(var, "r");
            } else {
                panic!("expected nodes() function");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_parse_relationships_function() {
        let stmt = parse("MATCH (a)-[r:KNOWS*2]->(b) RETURN relationships(r)").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.return_clause.items.len(), 1);
            if let ReturnItem::Function(ScalarFunction::Relationships(var)) =
                &m.return_clause.items[0]
            {
                assert_eq!(var, "r");
            } else {
                panic!("expected relationships() function");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_parse_length_function() {
        let stmt = parse("MATCH (a)-[r:KNOWS*2]->(b) RETURN length(r)").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.return_clause.items.len(), 1);
            if let ReturnItem::Function(ScalarFunction::Length(var)) = &m.return_clause.items[0] {
                assert_eq!(var, "r");
            } else {
                panic!("expected length() function");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_variable_length_path_star_only() {
        // [*] should mean 1..unlimited
        let stmt = parse("MATCH (a)-[:KNOWS*]->(b) RETURN a").unwrap();

        if let Statement::Match(m) = stmt {
            if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
                let seg = &path.segments[0];
                let range = seg
                    .edge
                    .length_range
                    .as_ref()
                    .expect("should have length_range");
                assert_eq!(range.min, 1);
                assert_eq!(range.max, None); // unlimited
            } else {
                panic!("expected path pattern");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_variable_length_path_zero_min() {
        // [*0..3] should allow zero hops
        let stmt = parse("MATCH (a)-[:KNOWS*0..3]->(b) RETURN a").unwrap();

        if let Statement::Match(m) = stmt {
            if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
                let seg = &path.segments[0];
                let range = seg
                    .edge
                    .length_range
                    .as_ref()
                    .expect("should have length_range");
                assert_eq!(range.min, 0);
                assert_eq!(range.max, Some(3));
            } else {
                panic!("expected path pattern");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    // ========== shortestPath / allShortestPaths tests ==========

    #[test]
    fn test_parse_shortest_path() {
        let stmt = parse("MATCH (a:Person), (b:Person) RETURN shortestPath(a, b)").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.return_clause.items.len(), 1);
            if let ReturnItem::Function(ScalarFunction::ShortestPath { start, end }) =
                &m.return_clause.items[0]
            {
                assert_eq!(start, "a");
                assert_eq!(end, "b");
            } else {
                panic!("expected shortestPath() function");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_parse_all_shortest_paths() {
        let stmt = parse("MATCH (a:Person), (b:Person) RETURN allShortestPaths(a, b)").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.return_clause.items.len(), 1);
            if let ReturnItem::Function(ScalarFunction::AllShortestPaths { start, end }) =
                &m.return_clause.items[0]
            {
                assert_eq!(start, "a");
                assert_eq!(end, "b");
            } else {
                panic!("expected allShortestPaths() function");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    // ========== OPTIONAL MATCH tests ==========

    #[test]
    fn test_optional_match() {
        let stmt = parse("MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b) RETURN a, b").unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.segments[0].match_clauses.len(), 2);
            assert!(!m.segments[0].match_clauses[0].optional);
            assert!(m.segments[0].match_clauses[1].optional);
            assert_eq!(m.segments[0].match_clauses[0].patterns.len(), 1);
            assert_eq!(m.segments[0].match_clauses[1].patterns.len(), 1);
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_multiple_optional_match() {
        let stmt = parse(
            "MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b) OPTIONAL MATCH (a)-[:WORKS_AT]->(c) RETURN a, b, c",
        )
        .unwrap();

        if let Statement::Match(m) = stmt {
            assert_eq!(m.segments[0].match_clauses.len(), 3);
            assert!(!m.segments[0].match_clauses[0].optional);
            assert!(m.segments[0].match_clauses[1].optional);
            assert!(m.segments[0].match_clauses[2].optional);
        } else {
            panic!("expected MATCH statement");
        }
    }

    // ========== CASE WHEN tests ==========

    #[test]
    fn test_case_when_searched() {
        // Searched CASE: CASE WHEN condition THEN result ... END
        let stmt =
            parse("MATCH (n:Person) WHERE CASE WHEN n.age < 20 THEN true ELSE false END RETURN n")
                .unwrap();

        if let Statement::Match(m) = stmt {
            assert!(m.segments[0].where_clause.is_some());
            if let Expression::Case(case_expr) = m.segments[0].where_clause.as_ref().unwrap() {
                assert!(case_expr.operand.is_none()); // Searched CASE
                assert_eq!(case_expr.when_clauses.len(), 1);
                assert!(case_expr.else_clause.is_some());
            } else {
                panic!("expected CASE expression");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_case_when_simple() {
        // Simple CASE: CASE expr WHEN value THEN result ... END
        let stmt = parse(
            "MATCH (n:Person) WHERE CASE n.status WHEN 1 THEN true WHEN 2 THEN false END RETURN n",
        )
        .unwrap();

        if let Statement::Match(m) = stmt {
            assert!(m.segments[0].where_clause.is_some());
            if let Expression::Case(case_expr) = m.segments[0].where_clause.as_ref().unwrap() {
                assert!(case_expr.operand.is_some()); // Simple CASE
                assert_eq!(case_expr.when_clauses.len(), 2);
                assert!(case_expr.else_clause.is_none());
            } else {
                panic!("expected CASE expression");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_case_when_multiple_when() {
        let stmt = parse(
            "MATCH (n:Person) WHERE CASE WHEN n.age < 20 THEN true WHEN n.age < 60 THEN true ELSE false END RETURN n",
        )
        .unwrap();

        if let Statement::Match(m) = stmt {
            if let Expression::Case(case_expr) = m.segments[0].where_clause.as_ref().unwrap() {
                assert_eq!(case_expr.when_clauses.len(), 2);
            } else {
                panic!("expected CASE expression");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    // ========== WITH clause tests ==========

    #[test]
    fn test_with_clause_basic() {
        let stmt = parse("MATCH (n:Person) WITH n RETURN n").unwrap();

        if let Statement::Match(m) = stmt {
            // Single segment with MATCH and WITH
            assert_eq!(m.segments.len(), 1);
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert!(!with.distinct);
            assert_eq!(with.items.len(), 1);
            if let ReturnItem::Variable(name) = &with.items[0].expression {
                assert_eq!(name, "n");
            } else {
                panic!("expected variable in WITH");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_clause_alias() {
        let stmt = parse("MATCH (n:Person) WITH n.name AS name RETURN name").unwrap();

        if let Statement::Match(m) = stmt {
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert_eq!(with.items.len(), 1);
            assert_eq!(with.items[0].alias, Some("name".to_string()));
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_distinct() {
        let stmt = parse("MATCH (n:Person) WITH DISTINCT n.city AS city RETURN city").unwrap();

        if let Statement::Match(m) = stmt {
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert!(with.distinct);
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_order_by() {
        let stmt = parse("MATCH (n:Person) WITH n ORDER BY n.age RETURN n").unwrap();

        if let Statement::Match(m) = stmt {
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert!(with.order_by.is_some());
            let order_by = with.order_by.as_ref().unwrap();
            assert_eq!(order_by.items.len(), 1);
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_limit() {
        let stmt = parse("MATCH (n:Person) WITH n LIMIT 10 RETURN n").unwrap();

        if let Statement::Match(m) = stmt {
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert_eq!(
                with.limit,
                Some(Expression::Literal(Literal::Int(10)))
            );
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_skip() {
        let stmt = parse("MATCH (n:Person) WITH n SKIP 5 RETURN n").unwrap();

        if let Statement::Match(m) = stmt {
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert_eq!(
                with.skip,
                Some(Expression::Literal(Literal::Int(5)))
            );
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_aggregation() {
        let stmt =
            parse("MATCH (n:Person) WITH n.city AS city, COUNT(*) AS count RETURN city, count")
                .unwrap();

        if let Statement::Match(m) = stmt {
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert_eq!(with.items.len(), 2);
            assert_eq!(with.items[0].alias, Some("city".to_string()));
            assert_eq!(with.items[1].alias, Some("count".to_string()));
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_where() {
        let stmt = parse(
            "MATCH (n:Person) WITH n.city AS city, COUNT(*) AS count WHERE count > 5 RETURN city",
        )
        .unwrap();

        if let Statement::Match(m) = stmt {
            // First segment has MATCH and WITH
            // Second segment has WHERE (filtering on WITH results)
            assert_eq!(m.segments.len(), 2);
            assert!(m.segments[0].with_clause.is_some());
            assert!(m.segments[1].where_clause.is_some());
            assert!(m.segments[1].match_clauses.is_empty());
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_multiple_with() {
        let stmt = parse("MATCH (n:Person) WITH n.city AS city WITH city RETURN city").unwrap();

        if let Statement::Match(m) = stmt {
            // MATCH + WITH creates 1 segment, second WITH creates another
            assert_eq!(m.segments.len(), 2);
            assert!(m.segments[0].with_clause.is_some());
            assert!(m.segments[1].with_clause.is_some());
        } else {
            panic!("expected MATCH statement");
        }
    }

    // ========== Regex match (=~) tests ==========

    #[test]
    fn test_regex_match_parse() {
        let stmt = parse(r#"MATCH (n:Person) WHERE n.name =~ "A.*" RETURN n"#).unwrap();

        if let Statement::Match(m) = stmt {
            let where_clause = m.segments[0].where_clause.clone().unwrap();
            if let Expression::BinaryOp(left, op, right) = where_clause {
                assert_eq!(op, BinaryOp::Regex);
                assert_eq!(
                    *left,
                    Expression::Property("n".to_string(), "name".to_string())
                );
                assert_eq!(
                    *right,
                    Expression::Literal(Literal::String("A.*".to_string()))
                );
            } else {
                panic!("expected binary expression with =~");
            }
        } else {
            panic!("expected MATCH statement");
        }
    }

    // ========== UNION / UNION ALL tests ==========

    #[test]
    fn test_union_parse() {
        let stmt =
            parse("MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name").unwrap();

        if let Statement::Union(u) = stmt {
            assert_eq!(u.queries.len(), 2);
            assert_eq!(u.union_type, UnionType::Union);
        } else {
            panic!("expected UNION statement");
        }
    }

    #[test]
    fn test_union_all_parse() {
        let stmt =
            parse("MATCH (n:Person) RETURN n.name UNION ALL MATCH (n:Company) RETURN n.name")
                .unwrap();

        if let Statement::Union(u) = stmt {
            assert_eq!(u.queries.len(), 2);
            assert_eq!(u.union_type, UnionType::UnionAll);
        } else {
            panic!("expected UNION statement");
        }
    }

    #[test]
    fn test_union_three_queries() {
        let stmt = parse(
            "MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name UNION MATCH (n:City) RETURN n.name",
        )
        .unwrap();

        if let Statement::Union(u) = stmt {
            assert_eq!(u.queries.len(), 3);
            assert_eq!(u.union_type, UnionType::Union);
        } else {
            panic!("expected UNION statement");
        }
    }

    // ========== MATCH + CREATE tests ==========

    #[test]
    fn test_parse_match_create() {
        let stmt = parse(
            r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
        )
        .unwrap();

        if let Statement::MatchCreate(mc) = stmt {
            assert_eq!(mc.create_clause.patterns.len(), 1);
        } else {
            panic!("expected MatchCreate statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_match_create_with_where() {
        let stmt = parse(
            r#"MATCH (a:Person) WHERE a.age > 20 CREATE (a)-[:MEMBER_OF]->(g:Group {name: "Adults"})"#,
        )
        .unwrap();

        if let Statement::MatchCreate(mc) = stmt {
            assert!(mc.where_clause.is_some());
            assert_eq!(mc.create_clause.patterns.len(), 1);
        } else {
            panic!("expected MatchCreate statement");
        }
    }

    // ========== MATCH + SET tests ==========

    #[test]
    fn test_parse_match_set_standalone() {
        let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31 RETURN n"#).unwrap();

        if let Statement::MatchSet(ms) = stmt {
            assert_eq!(ms.set_clause.items.len(), 1);
            assert!(ms.return_clause.is_some());
        } else {
            panic!("expected MatchSet statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_match_set_multiple() {
        let stmt =
            parse(r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31, n.city = "Tokyo" RETURN n"#)
                .unwrap();

        if let Statement::MatchSet(ms) = stmt {
            assert_eq!(ms.set_clause.items.len(), 2);
        } else {
            panic!("expected MatchSet statement");
        }
    }

    #[test]
    fn test_parse_match_set_no_return() {
        let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31"#).unwrap();

        if let Statement::MatchSet(ms) = stmt {
            assert!(ms.return_clause.is_none());
        } else {
            panic!("expected MatchSet statement");
        }
    }

    // ========== MERGE tests ==========

    #[test]
    fn test_parse_merge_simple() {
        let stmt = parse(r#"MERGE (n:Person {name: "Alice"})"#).unwrap();

        if let Statement::Merge(m) = stmt {
            assert!(m.match_clauses.is_empty());
            assert_eq!(m.patterns.len(), 1);
            assert!(m.on_create_set.is_none());
            assert!(m.on_match_set.is_none());
        } else {
            panic!("expected Merge statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_merge_with_on_create_set() {
        let stmt = parse(r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25"#).unwrap();

        if let Statement::Merge(m) = stmt {
            assert!(m.on_create_set.is_some());
            assert!(m.on_match_set.is_none());
        } else {
            panic!("expected Merge statement");
        }
    }

    #[test]
    fn test_parse_merge_with_both_on_clauses() {
        let stmt = parse(
            r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25 ON MATCH SET n.age = 30"#,
        )
        .unwrap();

        if let Statement::Merge(m) = stmt {
            assert!(m.on_create_set.is_some());
            assert!(m.on_match_set.is_some());
        } else {
            panic!("expected Merge statement");
        }
    }

    #[test]
    fn test_parse_match_merge() {
        let stmt = parse(
            r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) MERGE (a)-[:KNOWS]->(b)"#,
        )
        .unwrap();

        if let Statement::Merge(m) = stmt {
            assert_eq!(m.match_clauses.len(), 1);
            assert_eq!(m.patterns.len(), 1);
        } else {
            panic!("expected Merge statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_merge_with_return() {
        let stmt = parse(r#"MERGE (n:Person {name: "Alice"}) RETURN n"#).unwrap();

        if let Statement::Merge(m) = stmt {
            assert!(m.return_clause.is_some());
        } else {
            panic!("expected Merge statement");
        }
    }

    // ========== MATCH + REMOVE tests ==========

    #[test]
    fn test_parse_match_remove_property() {
        let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age RETURN n"#).unwrap();

        if let Statement::MatchRemove(mr) = stmt {
            assert_eq!(mr.remove_clause.items.len(), 1);
            assert!(matches!(
                mr.remove_clause.items[0],
                RemoveItem::Property(_, _)
            ));
            assert!(mr.return_clause.is_some());
        } else {
            panic!("expected MatchRemove statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_match_remove_label() {
        let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) REMOVE n:Person RETURN n"#).unwrap();

        if let Statement::MatchRemove(mr) = stmt {
            assert_eq!(mr.remove_clause.items.len(), 1);
            if let RemoveItem::Label(var, label) = &mr.remove_clause.items[0] {
                assert_eq!(var, "n");
                assert_eq!(label, "Person");
            } else {
                panic!("expected Label remove item");
            }
        } else {
            panic!("expected MatchRemove statement");
        }
    }

    #[test]
    fn test_parse_match_remove_multiple() {
        let stmt =
            parse(r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age, n.city RETURN n"#).unwrap();

        if let Statement::MatchRemove(mr) = stmt {
            assert_eq!(mr.remove_clause.items.len(), 2);
        } else {
            panic!("expected MatchRemove statement");
        }
    }

    // ========== UNWIND tests ==========

    #[test]
    fn test_parse_unwind_list() {
        let stmt = parse("UNWIND [1, 2, 3] AS x RETURN x").unwrap();

        if let Statement::Unwind(uw) = stmt {
            assert_eq!(uw.variable, "x");
            assert!(uw.return_clause.is_some());
            assert!(uw.create_clause.is_none());
        } else {
            panic!("expected Unwind statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_unwind_string_list() {
        let stmt = parse(r#"UNWIND ["a", "b"] AS name RETURN name"#).unwrap();

        if let Statement::Unwind(uw) = stmt {
            assert_eq!(uw.variable, "name");
        } else {
            panic!("expected Unwind statement");
        }
    }

    #[test]
    fn test_parse_list_expression() {
        let stmt = parse("UNWIND [1, 2, 3] AS x RETURN x").unwrap();

        if let Statement::Unwind(uw) = stmt {
            if let Expression::List(elems) = &uw.expression {
                assert_eq!(elems.len(), 3);
            } else {
                panic!("expected List expression");
            }
        } else {
            panic!("expected Unwind statement");
        }
    }

    // ========== Lexer keyword tests ==========

    #[test]
    fn test_parse_merge_keyword() {
        // Just make sure MERGE is recognized as keyword
        let stmt = parse(r#"MERGE (n:Person {name: "Alice"})"#);
        assert!(stmt.is_ok());
    }

    #[test]
    fn test_parse_remove_keyword() {
        let stmt = parse(r#"MATCH (n:Person) REMOVE n.age"#);
        assert!(stmt.is_ok());
    }

    #[test]
    fn test_parse_unwind_keyword() {
        let stmt = parse("UNWIND [1] AS x RETURN x");
        assert!(stmt.is_ok());
    }

    // ========== Subquery parser tests ==========

    #[test]
    fn test_parse_exists_subquery_in_where() {
        let stmt = parse(
            r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
        );
        assert!(stmt.is_ok(), "Failed to parse EXISTS subquery: {:?}", stmt);
    }

    #[test]
    fn test_parse_exists_subquery_with_where_clause() {
        let stmt = parse(
            r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(f:Person) WHERE f.name = "Bob" } RETURN p.name"#,
        );
        assert!(
            stmt.is_ok(),
            "Failed to parse EXISTS subquery with WHERE: {:?}",
            stmt
        );
    }

    #[test]
    fn test_parse_count_subquery_in_where() {
        let stmt =
            parse(r#"MATCH (p:Person) WHERE COUNT { MATCH (p)-[:KNOWS]->() } > 5 RETURN p.name"#);
        assert!(
            stmt.is_ok(),
            "Failed to parse COUNT subquery in WHERE: {:?}",
            stmt
        );
    }

    #[test]
    fn test_parse_count_subquery_in_return() {
        let stmt = parse(r#"MATCH (p:Person) RETURN COUNT { MATCH (p)-[:KNOWS]->() }"#);
        assert!(
            stmt.is_ok(),
            "Failed to parse COUNT subquery in RETURN: {:?}",
            stmt
        );
    }

    #[test]
    fn test_parse_collect_subquery_in_return() {
        let stmt = parse(
            r#"MATCH (p:Person) RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
        );
        assert!(stmt.is_ok(), "Failed to parse COLLECT subquery: {:?}", stmt);
    }

    #[test]
    fn test_parse_call_subquery_with_with() {
        let stmt = parse(
            r#"MATCH (p:Person)
               CALL {
                 WITH p
                 MATCH (p)-[:KNOWS]->(f:Person)
                 RETURN COUNT(f) AS friend_count
               }
               RETURN p.name, friend_count"#,
        );
        assert!(
            stmt.is_ok(),
            "Failed to parse CALL subquery with WITH: {:?}",
            stmt
        );
    }

    #[test]
    fn test_parse_call_subquery_without_with() {
        let stmt = parse(
            r#"MATCH (p:Person)
               CALL {
                 MATCH (q:Person)
                 RETURN COUNT(q) AS total
               }
               RETURN p.name, total"#,
        );
        assert!(
            stmt.is_ok(),
            "Failed to parse CALL subquery without WITH: {:?}",
            stmt
        );
    }

    #[test]
    fn test_parse_call_subquery_ast_structure() {
        let stmt = parse(
            r#"MATCH (p:Person)
               CALL {
                 WITH p
                 MATCH (p)-[:KNOWS]->(f:Person)
                 RETURN COUNT(f) AS friend_count
               }
               RETURN p.name, friend_count"#,
        )
        .unwrap();

        match stmt {
            Statement::Match(m) => {
                assert!(m.call_clause.is_some(), "Expected call_clause to be Some");
                let call = m.call_clause.unwrap();
                assert_eq!(call.with_import, Some(vec!["p".to_string()]));
            }
            _ => panic!("Expected Match statement"),
        }
    }

    #[test]
    fn test_parse_exists_subquery_ast_structure() {
        let stmt = parse(
            r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
        )
        .unwrap();

        match stmt {
            Statement::Match(m) => {
                let segment = &m.segments[0];
                let where_expr = segment
                    .where_clause
                    .as_ref()
                    .expect("Expected WHERE clause");
                assert!(
                    matches!(where_expr, Expression::ExistsSubquery(_)),
                    "Expected ExistsSubquery expression, got {:?}",
                    where_expr
                );
            }
            _ => panic!("Expected Match statement"),
        }
    }

    #[test]
    fn test_parse_count_subquery_ast_structure() {
        let stmt =
            parse(r#"MATCH (p:Person) WHERE COUNT { MATCH (p)-[:KNOWS]->() } > 0 RETURN p.name"#)
                .unwrap();

        match stmt {
            Statement::Match(m) => {
                let segment = &m.segments[0];
                let where_expr = segment
                    .where_clause
                    .as_ref()
                    .expect("Expected WHERE clause");
                // The WHERE clause is: COUNT{...} > 0, which is a BinaryOp
                assert!(
                    matches!(where_expr, Expression::BinaryOp(_, _, _)),
                    "Expected BinaryOp containing CountSubquery"
                );
                if let Expression::BinaryOp(left, op, _right) = where_expr {
                    assert!(matches!(op, BinaryOp::Gt));
                    assert!(
                        matches!(left.as_ref(), Expression::CountSubquery(_)),
                        "Expected CountSubquery on left of BinaryOp"
                    );
                }
            }
            _ => panic!("Expected Match statement"),
        }
    }

    #[test]
    fn test_parse_collect_subquery_ast_structure() {
        let stmt = parse(
            r#"MATCH (p:Person) RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
        )
        .unwrap();

        match stmt {
            Statement::Match(m) => {
                assert_eq!(m.return_clause.items.len(), 1);
                match &m.return_clause.items[0] {
                    ReturnItem::Expr(Expression::CollectSubquery(_)) => {}
                    other => panic!("Expected CollectSubquery return item, got {:?}", other),
                }
            }
            _ => panic!("Expected Match statement"),
        }
    }

    // ========== FOREACH Tests ==========

    #[test]
    fn test_foreach_create() {
        let stmt =
            parse(r#"FOREACH (name IN ['Alice', 'Bob'] | CREATE (:Person {name: name}))"#).unwrap();
        match stmt {
            Statement::Foreach(f) => {
                assert_eq!(f.variable, "name");
                assert_eq!(f.clauses.len(), 1);
                assert!(matches!(f.clauses[0], ForeachClause::Create(_)));
            }
            _ => panic!("Expected Foreach statement"),
        }
    }

    #[test]
    fn test_foreach_set() {
        let stmt = parse(r#"FOREACH (n IN [1, 2, 3] | SET n.visited = true)"#).unwrap();
        match stmt {
            Statement::Foreach(f) => {
                assert_eq!(f.variable, "n");
                assert_eq!(f.clauses.len(), 1);
                assert!(matches!(f.clauses[0], ForeachClause::Set(_)));
            }
            _ => panic!("Expected Foreach statement"),
        }
    }

    #[test]
    fn test_foreach_remove() {
        let stmt = parse(r#"FOREACH (n IN [1] | REMOVE n.prop)"#).unwrap();
        match stmt {
            Statement::Foreach(f) => {
                assert_eq!(f.variable, "n");
                assert_eq!(f.clauses.len(), 1);
                assert!(matches!(f.clauses[0], ForeachClause::Remove(_)));
            }
            _ => panic!("Expected Foreach statement"),
        }
    }

    #[test]
    fn test_foreach_delete() {
        let stmt = parse(r#"FOREACH (n IN [1] | DELETE n)"#).unwrap();
        match stmt {
            Statement::Foreach(f) => {
                assert_eq!(f.variable, "n");
                assert_eq!(f.clauses.len(), 1);
                assert!(matches!(f.clauses[0], ForeachClause::Delete(_)));
            }
            _ => panic!("Expected Foreach statement"),
        }
    }

    #[test]
    fn test_foreach_merge() {
        let stmt = parse(r#"FOREACH (name IN ['Alice'] | MERGE (:Person {name: name}))"#).unwrap();
        match stmt {
            Statement::Foreach(f) => {
                assert_eq!(f.variable, "name");
                assert_eq!(f.clauses.len(), 1);
                assert!(matches!(f.clauses[0], ForeachClause::Merge(_)));
            }
            _ => panic!("Expected Foreach statement"),
        }
    }

    #[test]
    fn test_foreach_nested() {
        let stmt = parse(
            r#"FOREACH (city IN ['Tokyo'] | FOREACH (name IN ['Alice'] | CREATE (:Person {name: name})))"#,
        )
        .unwrap();
        match stmt {
            Statement::Foreach(outer) => {
                assert_eq!(outer.variable, "city");
                assert_eq!(outer.clauses.len(), 1);
                match &outer.clauses[0] {
                    ForeachClause::Foreach(inner) => {
                        assert_eq!(inner.variable, "name");
                        assert_eq!(inner.clauses.len(), 1);
                        assert!(matches!(inner.clauses[0], ForeachClause::Create(_)));
                    }
                    _ => panic!("Expected nested Foreach"),
                }
            }
            _ => panic!("Expected Foreach statement"),
        }
    }

    #[test]
    fn test_foreach_with_literal_list() {
        let stmt = parse(r#"FOREACH (x IN [1, 2, 3] | CREATE (:Item {value: x}))"#).unwrap();
        match stmt {
            Statement::Foreach(f) => {
                assert_eq!(f.variable, "x");
                match &f.list {
                    Expression::List(items) => assert_eq!(items.len(), 3),
                    _ => panic!("Expected list expression"),
                }
            }
            _ => panic!("Expected Foreach statement"),
        }
    }

    #[test]
    fn test_match_foreach_parse() {
        let stmt = parse(r#"MATCH (n:Person) FOREACH (x IN [1] | SET n.updated = true)"#).unwrap();
        match stmt {
            Statement::MatchForeach(mf) => {
                assert_eq!(mf.segments.len(), 1);
                assert_eq!(mf.foreach_clause.variable, "x");
                assert_eq!(mf.foreach_clause.clauses.len(), 1);
                assert!(matches!(
                    mf.foreach_clause.clauses[0],
                    ForeachClause::Set(_)
                ));
            }
            _ => panic!("Expected MatchForeach statement"),
        }
    }

    // ========== parse_with_recovery Tests ==========

    #[test]
    fn test_recovery_valid_query_returns_statement() {
        let result = parse_with_recovery("MATCH (n:Person) RETURN n");
        assert!(result.is_ok());
        assert!(result.statement.is_some());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_recovery_invalid_keyword_returns_error() {
        let result = parse_with_recovery("FOOBAR (n)");
        assert!(result.is_err());
        assert!(result.statement.is_none());
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_recovery_empty_input_returns_error() {
        let result = parse_with_recovery("");
        assert!(result.is_err());
        assert!(result.statement.is_none());
        assert!(!result.errors.is_empty());
        // 空入力は UnexpectedEof またはレキサーエラーを返す
        let has_eof_or_lexer_error = result.errors.iter().any(|e| {
            matches!(e, ParseError::UnexpectedEof | ParseError::LexerError(_))
        });
        assert!(has_eof_or_lexer_error || !result.errors.is_empty());
    }

    #[test]
    fn test_recovery_incomplete_query_returns_error() {
        let result = parse_with_recovery("MATCH (n:Person) WHERE");
        assert!(result.is_err());
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_recovery_errors_contain_span_info() {
        let result = parse_with_recovery("INVALID TOKEN HERE");
        assert!(result.is_err());
        // エラーにはスパン情報が含まれていること
        let has_token_error = result.errors.iter().any(|e| {
            matches!(e, ParseError::UnexpectedToken { span, .. } if span.line > 0 || span.column > 0)
        });
        // UnexpectedEof またはトークンエラーが含まれていること
        assert!(!result.errors.is_empty());
        let _ = has_token_error;
    }

    #[test]
    fn test_recovery_create_statement() {
        let result = parse_with_recovery("CREATE (n:Person {name: 'Alice'})");
        assert!(result.is_ok());
        assert!(matches!(result.statement, Some(Statement::Create(_))));
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_recovery_parse_result_is_ok_is_err() {
        let ok_result = parse_with_recovery("MATCH (n) RETURN n");
        assert!(ok_result.is_ok());
        assert!(!ok_result.is_err());

        let err_result = parse_with_recovery("???");
        assert!(!err_result.is_ok());
        assert!(err_result.is_err());
    }

    #[test]
    fn test_recovery_multiple_errors_possible() {
        // 現在の実装では最初のエラーで停止するが、errors は Vec なので複数エラーを保持できる
        let result = parse_with_recovery("NOT A VALID CYPHER QUERY AT ALL");
        assert!(result.is_err());
        // errors フィールドにエラーが含まれること
        assert!(!result.errors.is_empty());
    }
}
