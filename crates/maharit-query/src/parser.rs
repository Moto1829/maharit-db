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
    ///
    /// 入力全体を 1 文として解釈する。文の後ろに解釈できないトークンが残る場合は
    /// エラーにする（以前は黙って捨てていたため、例えば
    /// `MATCH … SET … REMOVE …` の REMOVE が無視されても成功扱いになっていた）。
    pub fn parse(&mut self) -> Result<Statement, ParseError> {
        let stmt = self.parse_statement()?;
        match self.peek_kind() {
            None | Some(TokenKind::Eof) => Ok(stmt),
            Some(_) => Err(self.unexpected_token("end of query")),
        }
    }

    /// 決まった形の文として読み、書き込み句の後に句が続いていたり読めなかったりしたら、
    /// 先頭から句の列（[`Statement::Pipeline`]）として読み直す（task171）。
    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.pos;
        let data_statement = matches!(
            self.peek_kind(),
            Some(
                TokenKind::Create
                    | TokenKind::Match
                    | TokenKind::Merge
                    | TokenKind::Unwind
                    | TokenKind::Optional
                    | TokenKind::With
                    | TokenKind::Foreach
            )
        );
        let result = self.parse_fixed_statement();
        if !data_statement || (result.is_ok() && !self.at_pipeline_clause()) {
            return result;
        }
        // DDL（CREATE INDEX など）は句の列にならない
        if let Ok(stmt) = &result
            && !matches!(
                stmt,
                Statement::Create(_)
                    | Statement::CreateReturn(..)
                    | Statement::Match(_)
                    | Statement::MatchCreate(_)
                    | Statement::MatchSet(_)
                    | Statement::MatchRemove(_)
                    | Statement::Merge(_)
                    | Statement::Delete(_)
                    | Statement::Unwind(_)
                    | Statement::Foreach(_)
                    | Statement::MatchForeach(_)
            )
        {
            return result;
        }
        let end = self.pos;
        self.pos = start;
        match self.parse_pipeline() {
            Ok(stmt) if matches!(self.peek_kind(), None | Some(TokenKind::Eof)) => Ok(stmt),
            _ => {
                self.pos = end;
                result
            }
        }
    }

    /// 次のトークンが句の始まりか（決まった形の文の後に続いていたら句の列として読み直す）
    fn at_pipeline_clause(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(
                TokenKind::Create
                    | TokenKind::Match
                    | TokenKind::Optional
                    | TokenKind::Merge
                    | TokenKind::Unwind
                    | TokenKind::With
                    | TokenKind::Set
                    | TokenKind::Remove
                    | TokenKind::Delete
                    | TokenKind::Detach
                    | TokenKind::Foreach
            )
        )
    }

    /// 句の列: 任意の順の読み取り・書き込み句と、最後の省略可能な RETURN
    fn parse_pipeline(&mut self) -> Result<Statement, ParseError> {
        let mut clauses = Vec::new();
        let return_clause = loop {
            match self.peek_kind() {
                Some(
                    TokenKind::Match | TokenKind::Optional | TokenKind::Unwind | TokenKind::With,
                ) => {
                    let (segment, _, set_clause, delete_clause) = self.parse_query_segment()?;
                    clauses.push(PipelineClause::Read(segment));
                    if let Some(set) = set_clause {
                        clauses.push(PipelineClause::Update(UpdateClause::Set(set)));
                    }
                    if let Some(delete) = delete_clause {
                        clauses.push(PipelineClause::Delete(delete));
                    }
                }
                Some(TokenKind::Where)
                    if matches!(
                        clauses.last(),
                        Some(PipelineClause::Read(QuerySegment {
                            with_clause: Some(_),
                            ..
                        }))
                    ) =>
                {
                    // WITH … WHERE
                    self.advance();
                    clauses.push(PipelineClause::Read(QuerySegment {
                        where_clause: Some(self.parse_expression()?),
                        ..Default::default()
                    }));
                }
                Some(TokenKind::Create) => {
                    clauses.push(PipelineClause::Create(self.parse_create_clause()?))
                }
                Some(TokenKind::Merge) => match self.parse_merge(vec![], None)? {
                    Statement::Merge(m) if m.return_clause.is_some() => {
                        let mut m = m;
                        let rc = m.return_clause.take();
                        clauses.push(PipelineClause::Merge(m));
                        break rc;
                    }
                    Statement::Merge(m) => clauses.push(PipelineClause::Merge(m)),
                    _ => unreachable!("parse_merge returns Statement::Merge"),
                },
                Some(TokenKind::Set) => clauses.push(PipelineClause::Update(UpdateClause::Set(
                    self.parse_set_clause()?,
                ))),
                Some(TokenKind::Remove) => clauses.push(PipelineClause::Update(
                    UpdateClause::Remove(self.parse_remove_clause()?),
                )),
                Some(TokenKind::Delete | TokenKind::Detach) => {
                    clauses.push(PipelineClause::Delete(self.parse_delete_clause()?))
                }
                Some(TokenKind::Foreach) => {
                    clauses.push(PipelineClause::Foreach(self.parse_foreach_statement()?))
                }
                Some(TokenKind::Return) => {
                    self.advance();
                    break Some(self.parse_return_clause()?);
                }
                _ => break None,
            }
        };
        if clauses.is_empty() {
            return Err(self.unexpected_token("a clause"));
        }
        Ok(Statement::Pipeline(PipelineStatement {
            clauses,
            return_clause,
        }))
    }

    fn parse_fixed_statement(&mut self) -> Result<Statement, ParseError> {
        // Handle EXPLAIN / PROFILE prefixes
        if self.check(TokenKind::Explain) {
            self.advance();
            let inner = self.parse_statement()?;
            return Ok(Statement::Explain(Box::new(inner)));
        }
        if self.check(TokenKind::Profile) {
            self.advance();
            let inner = self.parse_statement()?;
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
                // Peek ahead to check for CREATE INDEX ON
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Index) {
                    return self.parse_create_index();
                }
                // Peek ahead to check for CREATE USER
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::User) {
                    return self.parse_create_user();
                }
                self.parse_create()?
            }
            Some(TokenKind::Match) => self.parse_match_or_delete()?,
            Some(TokenKind::Merge) => return self.parse_merge(vec![], None),
            Some(TokenKind::Unwind) => {
                // `UNWIND … AS x` の後に MATCH / UNWIND / WITH が続くなら読み取りのパイプライン
                let start = self.pos;
                self.parse_unwind_clause()?;
                let chained = self.check(TokenKind::Match)
                    || self.check(TokenKind::Optional)
                    || self.check(TokenKind::Unwind)
                    || self.check(TokenKind::With);
                self.pos = start;
                if chained {
                    self.parse_match_or_delete()?
                } else {
                    return self.parse_unwind();
                }
            }
            Some(TokenKind::Optional) | Some(TokenKind::With) => self.parse_match_or_delete()?,
            Some(TokenKind::Foreach) => return self.parse_foreach(),
            Some(TokenKind::Return) => return self.parse_standalone_return(),
            Some(TokenKind::Drop) => {
                // Peek ahead to check for DROP FULLTEXT INDEX vs DROP CONSTRAINT
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Fulltext) {
                    return self.parse_drop_fulltext_index();
                }
                // Peek ahead to check for DROP INDEX
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Index) {
                    return self.parse_drop_index();
                }
                // Peek ahead to check for DROP USER
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::User) {
                    return self.parse_drop_user();
                }
                return self.parse_drop_constraint();
            }
            Some(TokenKind::Alter) => return self.parse_alter_user(),
            Some(TokenKind::Show) => return self.parse_show(),
            Some(TokenKind::Ident(word))
                if ["GRANT", "DENY", "REVOKE"].contains(&word.to_uppercase().as_str()) =>
            {
                return self.parse_acl();
            }
            Some(TokenKind::Call) => {
                // If followed by identifier (not '{'), it's a top-level procedure call
                let next = self.tokens.get(self.pos + 1).map(|t| &t.kind);
                if matches!(next, Some(TokenKind::Ident(_))) {
                    return self.parse_procedure_call();
                }
                if next != Some(&TokenKind::LBrace) {
                    return Err(self.unexpected_token("procedure name or { after CALL"));
                }
                // Standalone `CALL { … } RETURN …`: a MATCH statement with no
                // segments, so the subquery runs once against an empty row.
                let call_clause = Some(self.parse_call_subquery()?);
                self.expect(TokenKind::Return)?;
                let return_clause = self.parse_return_clause()?;
                Statement::Match(MatchStatement {
                    segments: vec![],
                    call_clause,
                    return_clause,
                })
            }
            Some(_) => {
                return Err(self.unexpected_token(
                    "CREATE, MATCH, MERGE, UNWIND, FOREACH, RETURN, DROP, DROP INDEX, SHOW, ALTER, CALL, EXPLAIN, or PROFILE",
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

        let create = CreateClause { patterns };
        if self.check(TokenKind::Return) {
            self.advance();
            let return_clause = self.parse_return_clause()?;
            return Ok(Statement::CreateReturn(create, return_clause));
        }
        Ok(Statement::Create(create))
    }

    /// 省略可能な RETURN 句
    fn parse_optional_return(&mut self) -> Result<Option<ReturnClause>, ParseError> {
        if self.check(TokenKind::Return) {
            self.advance();
            Ok(Some(self.parse_return_clause()?))
        } else {
            Ok(None)
        }
    }

    // ========== MATCH ==========

    fn parse_match_or_delete(&mut self) -> Result<Statement, ParseError> {
        // Parse first segment
        let (first_segment, is_delete, set_clause, delete_clause) = self.parse_query_segment()?;

        // UNWIND・WITH を含む DELETE / MERGE は決まった形の文で表せないので、
        // エラーにして句の列（parse_pipeline）で読み直させる
        if (!first_segment.unwinds.is_empty() && (is_delete || self.check(TokenKind::Merge)))
            || (first_segment.with_clause.is_some() && self.check(TokenKind::Merge))
        {
            return Err(
                self.unexpected_token("MATCH, WITH, CREATE, SET, REMOVE or RETURN after UNWIND")
            );
        }

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
            let return_clause = self.parse_optional_return()?;
            return Ok(Statement::MatchCreate(MatchCreateStatement {
                segments: vec![first_segment.clone()],
                where_clause: first_segment.where_clause,
                create_clause: create,
                return_clause,
            }));
        }

        // Check for MATCH + SET (standalone, not for DELETE)
        if let Some(set_clause) = set_clause {
            let more_updates = self.parse_trailing_updates()?;
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
                more_updates,
                return_clause,
            }));
        }

        // Check for MATCH + REMOVE
        if self.check(TokenKind::Remove) {
            let remove_clause = self.parse_remove_clause()?;
            let more_updates = self.parse_trailing_updates()?;
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
                more_updates,
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

        // 後続のセグメント: MATCH / OPTIONAL MATCH / UNWIND / WITH / （WITH の後の）WHERE
        loop {
            if self.check(TokenKind::Match)
                || self.check(TokenKind::Optional)
                || self.check(TokenKind::Unwind)
            {
                let (segment, is_del, set_cl, _) = self.parse_query_segment()?;
                if is_del {
                    return Err(self.unexpected_token("RETURN or WITH"));
                }
                // Check for SET after inner segment
                if let Some(set_cl) = set_cl {
                    let more_updates = self.parse_trailing_updates()?;
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
                        more_updates,
                        return_clause,
                    }));
                }
                // Check for CREATE after inner segment
                if self.check(TokenKind::Create) {
                    let create = self.parse_create_clause()?;
                    let return_clause = self.parse_optional_return()?;
                    segments.push(segment.clone());
                    return Ok(Statement::MatchCreate(MatchCreateStatement {
                        segments,
                        where_clause: segment.where_clause,
                        create_clause: create,
                        return_clause,
                    }));
                }
                // Check for REMOVE after inner segment
                if self.check(TokenKind::Remove) {
                    let remove_clause = self.parse_remove_clause()?;
                    let more_updates = self.parse_trailing_updates()?;
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
                        more_updates,
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
                    unwinds: vec![],
                    match_clauses: vec![],
                    where_clause: None,
                    with_clause,
                });
            } else if self.check(TokenKind::Where)
                && segments.last().is_some_and(|seg| seg.with_clause.is_some())
            {
                // WHERE after WITH (filtering on WITH results)
                self.advance();
                let where_clause = Some(self.parse_expression()?);

                // Check for WITH or RETURN
                if self.check(TokenKind::With) {
                    let with_clause = Some(self.parse_with_clause()?);
                    segments.push(QuerySegment {
                        unwinds: vec![],
                        match_clauses: vec![],
                        where_clause,
                        with_clause,
                    });
                } else {
                    segments.push(QuerySegment {
                        unwinds: vec![],
                        match_clauses: vec![],
                        where_clause,
                        with_clause: None,
                    });
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
        // 先頭の UNWIND 句（task163: `UNWIND … MATCH …` / `UNWIND … UNWIND …`）
        let mut unwinds = Vec::new();
        while self.check(TokenKind::Unwind) {
            unwinds.push(self.parse_unwind_clause()?);
        }

        // MATCH / OPTIONAL MATCH を任意の順・個数で（WITH の後の WHERE だけのときは空）
        let mut match_clauses = Vec::new();
        loop {
            if self.check(TokenKind::Match) {
                self.advance();
                match_clauses.push(self.parse_match_clause(false)?);
            } else if self.check(TokenKind::Optional) {
                self.advance();
                self.expect(TokenKind::Match)?;
                match_clauses.push(self.parse_match_clause(true)?);
            } else {
                break;
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
                    unwinds,
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
                unwinds,
                match_clauses,
                where_clause,
                with_clause,
            },
            false,
            set_clause,
            None,
        ))
    }

    /// `UNWIND expr AS var`（読み取りの文の中の UNWIND 句）
    fn parse_unwind_clause(&mut self) -> Result<UnwindClause, ParseError> {
        self.expect(TokenKind::Unwind)?;
        let expression = self.parse_expression()?;
        self.expect(TokenKind::As)?;
        let variable = self.expect_ident()?;
        Ok(UnwindClause {
            expression,
            variable,
        })
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
        loop {
            let item = self.parse_set_item()?;
            // `SET n:A:B` は AddLabel を 2 つに分ける
            if let SetItem::AddLabel(var, _) = &item {
                let var = var.clone();
                items.push(item);
                while self.check(TokenKind::Colon) {
                    self.advance();
                    items.push(SetItem::AddLabel(
                        var.clone(),
                        self.expect_ident_or_keyword()?,
                    ));
                }
            } else {
                items.push(item);
            }
            if !self.check(TokenKind::Comma) {
                break;
            }
            self.advance();
        }

        Ok(SetClause { items })
    }

    fn parse_set_item(&mut self) -> Result<SetItem, ParseError> {
        let variable = self.expect_ident()?;

        if self.check(TokenKind::PlusEquals) {
            self.advance(); // consume +=
            if self.check(TokenKind::LBrace) {
                // n += {key: value, ...}
                let props = self.parse_properties()?;
                Ok(SetItem::MergeProperties(variable, props))
            } else {
                // n += $props
                Ok(SetItem::MergeMap(variable, self.parse_expression()?))
            }
        } else if self.check(TokenKind::Eq) {
            // n = {key: value} / n = $props
            self.advance();
            Ok(SetItem::ReplaceProperties(
                variable,
                self.parse_expression()?,
            ))
        } else if self.check(TokenKind::Colon) {
            // n:NewLabel1:NewLabel2:...
            self.advance(); // consume :
            let label = self.expect_ident_or_keyword()?;
            // Only one label per AddLabel; additional colons parsed as separate items
            Ok(SetItem::AddLabel(variable, label))
        } else {
            // n.prop = value
            let property = self.expect_dot_property()?;
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

        // First item (with optional AS alias)
        {
            let mut item = self.parse_return_item()?;
            if self.check(TokenKind::As) {
                self.advance();
                let alias = self.expect_ident()?;
                item = ReturnItem::Alias(Box::new(item), alias);
            }
            items.push(item);
        }

        // Additional items (with optional AS alias)
        while self.check(TokenKind::Comma) {
            self.advance();
            let mut item = self.parse_return_item()?;
            if self.check(TokenKind::As) {
                self.advance();
                let alias = self.expect_ident()?;
                item = ReturnItem::Alias(Box::new(item), alias);
            }
            items.push(item);
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
        let expression = self.parse_return_item()?;

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

    /// SKIP / LIMIT の値をパースする。変数を含まない式（整数リテラル、`$param`、
    /// `$size * ($page - 1)` など）を受け付け、実行時に 0 以上の整数か検査する。
    fn parse_skip_limit_expr(&mut self) -> Result<Expression, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::Int(n)) if *n < 0 => Err(self.unexpected_token("non-negative integer")),
            _ => self.parse_expression(),
        }
    }

    fn parse_return_item(&mut self) -> Result<ReturnItem, ParseError> {
        if self.check(TokenKind::Star) {
            self.advance();
            return Ok(ReturnItem::All);
        }

        // List literals and comprehensions, including what follows them
        // (`[1, 2, 3][0]`, `[1] + [2]`)
        if self.check(TokenKind::LBracket) {
            return Ok(ReturnItem::Expr(self.parse_expression()?));
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

        // 非identifierトークンの場合はexpressionとして解析
        match self.peek_kind() {
            Some(TokenKind::Int(_))
            | Some(TokenKind::Float(_))
            | Some(TokenKind::String(_))
            | Some(TokenKind::True)
            | Some(TokenKind::False)
            | Some(TokenKind::Null)
            | Some(TokenKind::Not)
            | Some(TokenKind::Minus)
            | Some(TokenKind::Dash)
            | Some(TokenKind::Case)
            | Some(TokenKind::Parameter(_))
            | Some(TokenKind::LParen)
            | Some(TokenKind::LBrace) => {
                let expr = self.parse_expression()?;
                return Ok(ReturnItem::Expr(expr));
            }
            _ => {}
        }

        let saved_pos = self.pos;
        let item = self.parse_simple_return_item()?;
        // `n.age + 1`, `toUpper(n.name) = 'X'`, `n.list[0]`…: an operator after a
        // simple item means the whole item is an expression. Re-parse it as one
        // (aggregates cannot appear inside expressions, so they stay as they are).
        if !matches!(item, ReturnItem::Aggregate(_)) && self.at_expression_operator() {
            self.pos = saved_pos;
            return Ok(ReturnItem::Expr(self.parse_expression()?));
        }
        Ok(item)
    }

    /// Whether the next token continues an expression (binary operator,
    /// comparison, predicate keyword or index access).
    fn at_expression_operator(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(
                TokenKind::Plus
                    | TokenKind::Dash
                    | TokenKind::Star
                    | TokenKind::Slash
                    | TokenKind::Percent
                    | TokenKind::Colon
                    | TokenKind::Eq
                    | TokenKind::Neq
                    | TokenKind::Lt
                    | TokenKind::Gt
                    | TokenKind::Lte
                    | TokenKind::Gte
                    | TokenKind::RegexMatch
                    | TokenKind::Contains
                    | TokenKind::Starts
                    | TokenKind::Ends
                    | TokenKind::In
                    | TokenKind::Is
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::Xor
                    | TokenKind::LBracket
            )
        )
    }

    fn parse_simple_return_item(&mut self) -> Result<ReturnItem, ParseError> {
        let saved_pos = self.pos;
        let var = self.expect_ident()?;

        // テンポラル関数 (date/datetime/duration) は BinaryOp に参加できるため
        // parse_expression に委譲する
        if matches!(
            var.to_lowercase().as_str(),
            "date" | "datetime" | "duration"
        ) && self.check(TokenKind::LParen)
        {
            self.pos = saved_pos;
            let expr = self.parse_expression()?;
            return Ok(ReturnItem::Expr(expr));
        }

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

        match self.try_consume_dot_property()? {
            Some(prop) => Ok(ReturnItem::Property(var, prop)),
            None => Ok(ReturnItem::Variable(var)),
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
            // 数学関数: sin(x) など 1 引数 / pow(a, b)・atan2(y, x)
            name if MathFunction::from_name(name).is_some() => {
                let arg = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let f = MathFunction::from_name(name).expect("checked above");
                return Ok(ReturnItem::Function(ScalarFunction::Math(f, Box::new(arg))));
            }
            "pow" | "atan2" => {
                let arg1 = self.parse_expression()?;
                self.expect(TokenKind::Comma)?;
                let arg2 = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                let f = if func_name.eq_ignore_ascii_case("pow") {
                    MathFunction2::Pow
                } else {
                    MathFunction2::Atan2
                };
                return Ok(ReturnItem::Function(ScalarFunction::Math2(
                    f,
                    Box::new(arg1),
                    Box::new(arg2),
                )));
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
            // テンポラル型関数
            "date" => {
                let arg = if self.check(TokenKind::RParen) {
                    None
                } else {
                    Some(Box::new(self.parse_expression()?))
                };
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::DateFunc(arg)));
            }
            "datetime" => {
                let arg = if self.check(TokenKind::RParen) {
                    None
                } else {
                    Some(Box::new(self.parse_expression()?))
                };
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::DateTimeFunc(arg)));
            }
            "duration" => {
                let arg = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                return Ok(ReturnItem::Function(ScalarFunction::DurationFunc(
                    Box::new(arg),
                )));
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
                    expected: "function (COUNT, SUM, AVG, MIN, MAX, COLLECT, percentileCont, percentileDisc, stDev, stDevP, nodes, relationships, length, shortestPath, allShortestPaths, trim, ltrim, rtrim, toLower, toUpper, reverse, toString, size, left, right, substring, split, replace, abs, ceil, floor, round, sign, rand, isNaN, log, log10, sqrt, exp, pow, sin, cos, tan, cot, asin, acos, atan, atan2, degrees, radians, e, pi, id, elementId, type, startNode, endNode, labels, properties, keys, coalesce, nullIf, toBoolean, toFloat, toInteger, timestamp, randomUUID)".to_string(),
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

    /// SET / REMOVE 句の後に続く SET / REMOVE 句を記述順にパースする。
    fn parse_trailing_updates(&mut self) -> Result<Vec<UpdateClause>, ParseError> {
        let mut updates = Vec::new();
        loop {
            if self.check(TokenKind::Set) {
                updates.push(UpdateClause::Set(self.parse_set_clause()?));
            } else if self.check(TokenKind::Remove) {
                updates.push(UpdateClause::Remove(self.parse_remove_clause()?));
            } else {
                return Ok(updates);
            }
        }
    }

    fn parse_remove_clause(&mut self) -> Result<RemoveClause, ParseError> {
        self.expect(TokenKind::Remove)?;

        let mut items = Vec::new();
        loop {
            let item = self.parse_remove_item()?;
            // `REMOVE n:A:B` は Label を 2 つに分ける
            if let RemoveItem::Label(var, _) = &item {
                let var = var.clone();
                items.push(item);
                while self.check(TokenKind::Colon) {
                    self.advance();
                    items.push(RemoveItem::Label(
                        var.clone(),
                        self.expect_ident_or_keyword()?,
                    ));
                }
            } else {
                items.push(item);
            }
            if !self.check(TokenKind::Comma) {
                break;
            }
            self.advance();
        }

        Ok(RemoveClause { items })
    }

    fn parse_remove_item(&mut self) -> Result<RemoveItem, ParseError> {
        let variable = self.expect_ident()?;

        if let Some(property) = self.try_consume_dot_property()? {
            // REMOVE n.prop
            Ok(RemoveItem::Property(variable, property))
        } else if self.check(TokenKind::Colon) {
            // REMOVE n:Label (Task 108: キーワードもラベル名として許容)
            self.advance();
            let label = self.expect_ident_or_keyword()?;
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

    // ========== Standalone RETURN ==========

    fn parse_standalone_return(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Return)?;
        let return_clause = self.parse_return_clause()?;
        Ok(Statement::Return(return_clause))
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
        let label = self.expect_ident_or_keyword()?;
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
            let required_label = self.expect_ident_or_keyword()?;

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
                let prop = self.expect_dot_property()?;
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
            let property = self.expect_dot_property()?;
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
        let edge_type = self.expect_ident_or_keyword()?;
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
        let target_label = self.expect_ident_or_keyword()?;
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

        // SHOW INDEXES — INDEX is a keyword, so check it directly before falling
        // back to the identifier path used for CONSTRAINTS/USERS.
        if self.check(TokenKind::Index) {
            self.advance();
            return Ok(Statement::ShowIndexes);
        }

        // Expect CONSTRAINTS or USERS (as identifiers since they're not keywords)
        let ident = self.expect_ident()?;
        if ident.to_uppercase() == "ACL" {
            return Ok(Statement::ShowAcl);
        }
        if ident.to_uppercase() == "CONSTRAINTS" {
            Ok(Statement::ShowConstraints)
        } else if ident.to_uppercase() == "USERS" {
            Ok(Statement::ShowUsers)
        } else if ident.to_uppercase() == "INDEXES" {
            // Fallback in case INDEXES is tokenised as an identifier
            Ok(Statement::ShowIndexes)
        } else {
            Err(ParseError::UnexpectedToken {
                expected: "CONSTRAINTS, INDEXES, or USERS".to_string(),
                found: ident,
                span: self.current_span(),
            })
        }
    }

    // ========== Fulltext Index ==========

    /// CREATE FULLTEXT INDEX name FOR (n:Label) ON (n.prop1, n.prop2)
    /// CREATE FULLTEXT INDEX name FOR (n:Label) ON EACH [n.prop1, n.prop2]
    fn parse_create_fulltext_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;
        self.expect(TokenKind::Fulltext)?;
        self.expect(TokenKind::Index)?;

        let name = self.expect_ident()?;

        self.expect(TokenKind::For)?;
        self.expect(TokenKind::LParen)?;
        let variable = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let label = self.expect_ident_or_keyword()?;
        self.expect(TokenKind::RParen)?;

        // `ON (n.a, n.b)` or the Neo4j form `ON EACH [n.a, n.b]`
        self.expect(TokenKind::On)?;
        let each =
            matches!(self.peek_kind(), Some(TokenKind::Ident(w)) if w.eq_ignore_ascii_case("each"));
        let close = if each {
            self.advance();
            self.expect(TokenKind::LBracket)?;
            TokenKind::RBracket
        } else {
            self.expect(TokenKind::LParen)?;
            TokenKind::RParen
        };

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
            let prop_name = self.expect_dot_property()?;
            properties.push(prop_name);

            if !self.check(TokenKind::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(close)?;

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

    // ========== Property Index ==========

    /// CREATE INDEX ON :Label(property)
    /// CREATE INDEX ON :Label(property)
    /// CREATE INDEX FOR (n:Label) ON (n.property)（Neo4j 4.x 以降の形）
    fn parse_create_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;
        self.expect(TokenKind::Index)?;
        if self.check(TokenKind::For) {
            self.advance();
            self.expect(TokenKind::LParen)?;
            let var = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let label = self.expect_ident_or_keyword()?;
            self.expect(TokenKind::RParen)?;
            self.expect(TokenKind::On)?;
            self.expect(TokenKind::LParen)?;
            let span = self.current_span();
            let owner = self.expect_ident()?;
            if owner != var {
                return Err(ParseError::UnexpectedToken {
                    expected: format!("{var}.<property>"),
                    found: owner,
                    span,
                });
            }
            let property = self.expect_dot_property()?;
            if self.check(TokenKind::Comma) {
                return Err(self.unexpected_token(") (composite indexes are not supported)"));
            }
            self.expect(TokenKind::RParen)?;
            return Ok(Statement::CreateIndex(CreateIndexStatement {
                label,
                property,
            }));
        }
        self.expect(TokenKind::On)?;
        self.expect(TokenKind::Colon)?;
        let label = self.expect_ident_or_keyword()?;
        self.expect(TokenKind::LParen)?;
        let property = self.expect_ident_or_keyword()?;
        self.expect(TokenKind::RParen)?;
        Ok(Statement::CreateIndex(CreateIndexStatement {
            label,
            property,
        }))
    }

    /// DROP INDEX ON :Label(property)
    fn parse_drop_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Drop)?;
        self.expect(TokenKind::Index)?;
        self.expect(TokenKind::On)?;
        self.expect(TokenKind::Colon)?;
        let label = self.expect_ident_or_keyword()?;
        self.expect(TokenKind::LParen)?;
        let property = self.expect_ident_or_keyword()?;
        self.expect(TokenKind::RParen)?;
        Ok(Statement::DropIndex(DropIndexStatement { label, property }))
    }

    // ========== User Management ==========

    /// CREATE USER username SET PASSWORD 'pass' ROLE role
    /// ロール値を識別子（`reader`）または文字列リテラル（`'reader'`）として受理する。
    fn parse_role_value(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::String(_)) => self.expect_string(),
            _ => self.expect_ident(),
        }
    }

    fn parse_create_user(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;
        self.expect(TokenKind::User)?;
        let username = self.expect_ident()?;
        self.expect(TokenKind::Set)?;
        self.expect(TokenKind::Password)?;
        let password = self.expect_string()?;
        // ROLE の前の `SET` は省略可（`SET ROLE 'x'` と `ROLE x` の両方を受理）。
        if self.check(TokenKind::Set) {
            self.advance();
        }
        self.expect(TokenKind::Role)?;
        let role = self.parse_role_value()?;

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

        // Parse SET PASSWORD and/or [SET] ROLE
        if self.check(TokenKind::Password) {
            self.advance();
            password = Some(self.expect_string()?);

            // Check for additional [SET] ROLE
            if self.check(TokenKind::Set) {
                self.advance();
            }
            if self.check(TokenKind::Role) {
                self.advance();
                role = Some(self.parse_role_value()?);
            }
        } else if self.check(TokenKind::Role) {
            self.advance();
            role = Some(self.parse_role_value()?);
        } else {
            return Err(self.unexpected_token("PASSWORD or ROLE"));
        }

        Ok(Statement::AlterUser(AlterUserStatement {
            username,
            password,
            role,
        }))
    }

    // ========== ACL ==========

    /// 次のトークンが（大文字小文字を問わず）識別子 `word` なら読み進める。
    fn expect_word(&mut self, word: &str) -> Result<(), ParseError> {
        match self.peek_kind() {
            Some(TokenKind::Ident(w)) if w.eq_ignore_ascii_case(word) => {
                self.advance();
                Ok(())
            }
            _ => Err(self.unexpected_token(word)),
        }
    }

    /// `GRANT|DENY READ|WRITE ON :Label[(prop)] TO USER name | ROLE role`
    /// `REVOKE READ|WRITE ON :Label[(prop)] FROM USER name | ROLE role`
    /// （`:*` は全ラベル）
    fn parse_acl(&mut self) -> Result<Statement, ParseError> {
        let command = match self.peek_kind() {
            Some(TokenKind::Ident(w)) => match w.to_uppercase().as_str() {
                "GRANT" => AclCommand::Grant,
                "DENY" => AclCommand::Deny,
                _ => AclCommand::Revoke,
            },
            _ => return Err(self.unexpected_token("GRANT, DENY, or REVOKE")),
        };
        self.advance();
        let action = match self.peek_kind() {
            Some(TokenKind::Ident(w)) if w.eq_ignore_ascii_case("READ") => AclAction::Read,
            Some(TokenKind::Ident(w)) if w.eq_ignore_ascii_case("WRITE") => AclAction::Write,
            _ => return Err(self.unexpected_token("READ or WRITE")),
        };
        self.advance();
        self.expect(TokenKind::On)?;
        self.expect(TokenKind::Colon)?;
        let target = if self.check(TokenKind::Star) {
            self.advance();
            AclTarget::AllLabels
        } else {
            let label = self.expect_ident_or_keyword()?;
            if self.check(TokenKind::LParen) {
                self.advance();
                let property = self.expect_ident_or_keyword()?;
                self.expect(TokenKind::RParen)?;
                AclTarget::Property(label, property)
            } else {
                AclTarget::Label(label)
            }
        };
        if command == AclCommand::Revoke {
            self.expect_word("FROM")?;
        } else {
            self.expect_word("TO")?;
        }
        let grantee = if self.check(TokenKind::User) {
            self.advance();
            AclGrantee::User(self.expect_ident()?)
        } else if self.check(TokenKind::Role) {
            self.advance();
            AclGrantee::Role(self.parse_role_value()?)
        } else {
            return Err(self.unexpected_token("USER or ROLE"));
        };
        Ok(Statement::Acl(AclStatement {
            command,
            action,
            target,
            grantee,
        }))
    }

    // ========== Subqueries ==========

    /// CALL { [WITH var, ...] MATCH pattern [WHERE expr] RETURN item [AS alias], ... }
    fn parse_call_subquery(&mut self) -> Result<CallSubquery, ParseError> {
        self.expect(TokenKind::Call)?;
        self.expect(TokenKind::LBrace)?;
        let subquery = self.parse_call_subquery_branch()?;
        self.expect(TokenKind::RBrace)?;
        Ok(subquery)
    }

    /// One branch of a CALL subquery body: `[WITH vars] MATCH … [WHERE …] RETURN …`,
    /// followed by any `UNION [ALL]` branches.
    fn parse_call_subquery_branch(&mut self) -> Result<CallSubquery, ParseError> {
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

        let union = if self.check(TokenKind::Union) {
            self.advance();
            let union_type = if self.check(TokenKind::All) {
                self.advance();
                UnionType::UnionAll
            } else {
                UnionType::Union
            };
            Some((union_type, Box::new(self.parse_call_subquery_branch()?)))
        } else {
            None
        };

        Ok(CallSubquery {
            with_import,
            match_clause,
            where_clause,
            return_items,
            union,
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

        // Optional YIELD clause: col [AS alias], ...
        let mut yield_items = Vec::new();
        if self.check(TokenKind::Yield) {
            self.advance();
            loop {
                let column = self.expect_ident_or_keyword()?;
                let alias = if self.check(TokenKind::As) {
                    self.advance();
                    Some(self.expect_ident()?)
                } else {
                    None
                };
                yield_items.push(YieldItem { column, alias });
                if !self.check(TokenKind::Comma) {
                    break;
                }
                self.advance();
            }
        }

        let segments = self.parse_procedure_continuation(!yield_items.is_empty())?;
        let updates = self.parse_trailing_updates()?;

        let foreach_clause = if self.check(TokenKind::Foreach) {
            Some(self.parse_foreach_statement()?)
        } else {
            None
        };

        // Optional RETURN clause (for filtering/projection after YIELD)
        let return_clause = if foreach_clause.is_none() && self.check(TokenKind::Return) {
            self.advance();
            Some(self.parse_return_clause()?)
        } else {
            None
        };

        // MATCH / WITH after YIELD only make sense when something consumes their rows.
        let has_reading_clause = segments
            .iter()
            .any(|seg| !seg.match_clauses.is_empty() || seg.with_clause.is_some());
        if has_reading_clause
            && updates.is_empty()
            && foreach_clause.is_none()
            && return_clause.is_none()
        {
            return Err(self.unexpected_token("RETURN or FOREACH"));
        }

        Ok(Statement::ProcedureCall(ProcedureCallStatement {
            procedure,
            arguments,
            yield_items,
            segments,
            updates,
            foreach_clause,
            return_clause,
        }))
    }

    /// Parse the clauses that may follow `CALL proc() YIELD ...`:
    /// an optional `WHERE`, then any number of `MATCH` / `OPTIONAL MATCH` /
    /// `WITH [WHERE]` segments. `WHERE` / `MATCH` / `WITH` require a YIELD.
    fn parse_procedure_continuation(
        &mut self,
        has_yield: bool,
    ) -> Result<Vec<QuerySegment>, ParseError> {
        let mut segments = Vec::new();
        let continues = self.check(TokenKind::Where)
            || self.check(TokenKind::Match)
            || self.check(TokenKind::Optional)
            || self.check(TokenKind::With);
        if !continues {
            return Ok(segments);
        }
        if !has_yield {
            return Err(self.unexpected_token("YIELD before WHERE, MATCH or WITH"));
        }

        if self.check(TokenKind::Where) {
            self.advance();
            segments.push(QuerySegment {
                unwinds: vec![],
                match_clauses: vec![],
                where_clause: Some(self.parse_expression()?),
                with_clause: None,
            });
        }
        loop {
            if self.check(TokenKind::Match) {
                let (segment, is_delete, set_clause, _) = self.parse_query_segment()?;
                if is_delete || set_clause.is_some() {
                    return Err(self.unexpected_token("RETURN, FOREACH, MATCH or WITH"));
                }
                segments.push(segment);
            } else if self.check(TokenKind::Optional) {
                // parse_query_segment only starts at a plain MATCH.
                self.advance();
                self.expect(TokenKind::Match)?;
                let clause = self.parse_match_clause(true)?;
                let where_clause = if self.check(TokenKind::Where) {
                    self.advance();
                    Some(self.parse_expression()?)
                } else {
                    None
                };
                segments.push(QuerySegment {
                    unwinds: vec![],
                    match_clauses: vec![clause],
                    where_clause,
                    with_clause: None,
                });
            } else if self.check(TokenKind::With) {
                segments.push(QuerySegment {
                    unwinds: vec![],
                    match_clauses: vec![],
                    where_clause: None,
                    with_clause: Some(self.parse_with_clause()?),
                });
            } else if self.check(TokenKind::Where)
                && segments.last().is_some_and(|seg| seg.with_clause.is_some())
            {
                self.advance();
                segments.push(QuerySegment {
                    unwinds: vec![],
                    match_clauses: vec![],
                    where_clause: Some(self.parse_expression()?),
                    with_clause: None,
                });
            } else {
                return Ok(segments);
            }
        }
    }

    /// Expect an identifier or a keyword that can serve as an identifier in dotted names.
    /// `.prop` 形式のドットプロパティアクセスを読み取る（オプショナル版）。
    /// 次のトークンが `.` でなければ `Ok(None)` を返し、ある場合は dot を
    /// 消費してプロパティ名（識別子またはキーワード）を返す。
    ///
    /// 利用箇所: `parse_order_by_item`, `parse_return_item`, `parse_remove_item`,
    /// `parse_primary` (Expression::Property).
    fn try_consume_dot_property(&mut self) -> Result<Option<String>, ParseError> {
        if self.check(TokenKind::Dot) {
            self.advance();
            Ok(Some(self.expect_ident_or_keyword()?))
        } else {
            Ok(None)
        }
    }

    /// `.prop` を必ず期待する版（Dot がなければパースエラー）。
    ///
    /// 利用箇所: `parse_set_item`, REQUIRE constraint (単一/複合), FULLTEXT INDEX.
    fn expect_dot_property(&mut self) -> Result<String, ParseError> {
        self.expect(TokenKind::Dot)?;
        self.expect_ident_or_keyword()
    }

    /// プロパティ名 / プロシージャ名パート / マップキーなど、
    /// コンテキスト上は識別子として扱える位置で識別子またはキーワードを読み取る。
    ///
    /// Cypher 標準ではキーワードは予約語ではなく、識別子位置に現れたときは
    /// 識別子として解釈される。lexer 段階で大文字小文字を区別しないキーワード化を
    /// 行っているため、ここで再び識別子に戻す。
    fn expect_ident_or_keyword(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::Ident(s)) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            Some(kind) => {
                if let Some(name) = crate::lexer::keyword_as_ident(kind) {
                    self.advance();
                    Ok(name.to_string())
                } else {
                    self.expect_ident()
                }
            }
            None => self.expect_ident(),
        }
    }

    /// Parse subquery pattern body for EXISTS/COUNT: MATCH pattern [WHERE expr]
    fn parse_subquery_pattern(&mut self) -> Result<SubqueryPattern, ParseError> {
        // `EXISTS { MATCH (n)-->() WHERE … }` or the short form `EXISTS { (n)-->() }`
        let patterns = if self.check(TokenKind::Match) {
            self.expect(TokenKind::Match)?;
            self.parse_match_clause(false)?.patterns
        } else {
            let mut patterns = vec![self.parse_pattern()?];
            while self.check(TokenKind::Comma) {
                self.advance();
                patterns.push(self.parse_pattern()?);
            }
            patterns
        };

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
        // Path variable: `p = (a)-[..]->(b)`
        let path_variable = match (
            self.tokens.get(self.pos).map(|t| &t.kind),
            self.tokens.get(self.pos + 1).map(|t| &t.kind),
        ) {
            (Some(TokenKind::Ident(_)), Some(TokenKind::Eq)) => {
                let name = self.expect_ident()?;
                self.advance(); // consume '='
                Some(name)
            }
            _ => None,
        };

        // `shortestPath((a)-[*]-(b))` / `allShortestPaths(...)`
        if let Some(kind) = self.peek_shortest_path_kind() {
            self.advance(); // function name
            self.expect(TokenKind::LParen)?;
            let inner = self.parse_pattern()?;
            self.expect(TokenKind::RParen)?;
            let mut path = match inner {
                Pattern::Path(p) if p.variable.is_none() && p.segments.len() == 1 => p,
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        expected: "a single-relationship pattern such as (a)-[:T*]-(b) inside shortestPath()".to_string(),
                        found: "other pattern".to_string(),
                        span: self.current_span(),
                    });
                }
            };
            path.variable = path_variable;
            path.shortest = Some(kind);
            return Ok(Pattern::Path(path));
        }

        let start = self.parse_node_pattern()?;

        // Check if it's a path pattern
        if self.is_edge_start() || path_variable.is_some() {
            let mut segments = Vec::new();

            while self.is_edge_start() {
                let edge = self.parse_edge_pattern()?;
                let node = self.parse_node_pattern()?;
                segments.push(PathSegment { edge, node });
            }

            Ok(Pattern::Path(PathPattern {
                variable: path_variable,
                start,
                segments,
                shortest: None,
            }))
        } else {
            Ok(Pattern::Node(start))
        }
    }

    /// パターン位置で `shortestPath(` / `allShortestPaths(` が始まるならその種別を返す。
    fn peek_shortest_path_kind(&self) -> Option<ShortestPathKind> {
        let name = match self.peek_kind() {
            Some(TokenKind::Ident(name)) => name,
            _ => return None,
        };
        if self.tokens.get(self.pos + 1).map(|t| &t.kind) != Some(&TokenKind::LParen) {
            return None;
        }
        match name.to_lowercase().as_str() {
            "shortestpath" => Some(ShortestPathKind::Single),
            "allshortestpaths" => Some(ShortestPathKind::All),
            _ => None,
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
        // Cypher 標準ではキーワードはコンテキスト依存に識別子扱いするので、
        // ラベル位置でも `User`/`Role`/`Index` などのキーワードを受け入れる (Task 108)。
        while self.check(TokenKind::Colon) {
            self.advance();
            labels.push(self.expect_ident_or_keyword()?);
        }

        // Properties (optional): `{k: v}` or `$props`
        let properties_param = self.parse_pattern_properties(&mut properties)?;

        self.expect(TokenKind::RParen)?;

        Ok(NodePattern {
            variable,
            labels,
            properties,
            properties_param,
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
        let mut edge_types = Vec::new();
        let mut properties = HashMap::new();
        let mut properties_param = None;
        let mut length_range = None;

        // Edge details in brackets (optional)
        if self.check(TokenKind::LBracket) {
            self.advance();

            // Variable (optional)
            if let Some(TokenKind::Ident(_)) = self.peek_kind() {
                variable = Some(self.expect_ident()?);
            }

            // Type (optional) — リレーション型もラベル同様キーワードを許容 (Task 108)
            if self.check(TokenKind::Colon) {
                self.advance();
                edge_types.push(self.expect_ident_or_keyword()?);
                // `[:A|B]`（旧来の `[:A|:B]` も許容）
                while self.check(TokenKind::Pipe) {
                    self.advance();
                    if self.check(TokenKind::Colon) {
                        self.advance();
                    }
                    edge_types.push(self.expect_ident_or_keyword()?);
                }
            }

            // Variable-length path: *min..max (optional)
            if self.check(TokenKind::Star) {
                self.advance();
                length_range = Some(self.parse_length_range()?);
            }

            // Properties (optional): `{k: v}` or `$props`
            properties_param = self.parse_pattern_properties(&mut properties)?;

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
            edge_types,
            properties,
            properties_param,
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

    /// パターンのプロパティ位置を読む。`{k: v}` なら `properties` に入れ、
    /// `$props` ならパラメータ名を返す。どちらも無ければ何もしない。
    fn parse_pattern_properties(
        &mut self,
        properties: &mut HashMap<String, Expression>,
    ) -> Result<Option<String>, ParseError> {
        if self.check(TokenKind::LBrace) {
            *properties = self.parse_properties()?;
            return Ok(None);
        }
        if let Some(TokenKind::Parameter(name)) = self.peek_kind() {
            let name = name.clone();
            self.advance();
            return Ok(Some(name));
        }
        Ok(None)
    }

    fn parse_properties(&mut self) -> Result<HashMap<String, Expression>, ParseError> {
        self.expect(TokenKind::LBrace)?;

        let mut props = HashMap::new();

        if !self.check(TokenKind::RBrace) {
            loop {
                let key = self.parse_property_key()?;
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

    /// マップリテラルのキー位置で識別子を読み取る。
    /// クォート付き文字列キー (`{"id": 1}`) も許容。
    /// 識別子位置で現れたキーワードも識別子として再解釈する。
    fn parse_property_key(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::String(_)) => self.expect_string(),
            _ => self.expect_ident_or_keyword(),
        }
    }

    // ========== Expression ==========

    fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        self.parse_or_expression()
    }

    fn parse_or_expression(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_xor_expression()?;

        while self.check(TokenKind::Or) {
            self.advance();
            let right = self.parse_xor_expression()?;
            left = Expression::BinaryOp(Box::new(left), BinaryOp::Or, Box::new(right));
        }

        Ok(left)
    }

    /// 優先順位は OR < XOR < AND（openCypher と同じ）
    fn parse_xor_expression(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_and_expression()?;

        while self.check(TokenKind::Xor) {
            self.advance();
            let right = self.parse_and_expression()?;
            left = Expression::BinaryOp(Box::new(left), BinaryOp::Xor, Box::new(right));
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
                let negated = if self.check(TokenKind::Not) {
                    self.advance();
                    true
                } else {
                    false
                };
                if self.check(TokenKind::Null) {
                    self.advance();
                    let op = if negated {
                        UnaryOp::IsNotNull
                    } else {
                        UnaryOp::IsNull
                    };
                    return Ok(Expression::UnaryOp(op, Box::new(left)));
                }
                if !negated && self.check(TokenKind::Normalized) {
                    self.advance(); // consume NORMALIZED
                    return Ok(Expression::UnaryOp(UnaryOp::IsNormalized, Box::new(left)));
                }
                return Err(self.unexpected_token("NULL, NOT NULL or NORMALIZED"));
            }
            _ => return Ok(left),
        };

        self.advance();
        let right = self.parse_additive()?;
        let mut result = Expression::BinaryOp(Box::new(left), op, Box::new(right.clone()));

        // 連続した比較 `a < b <= c` は `a < b AND b <= c`
        let mut prev = right;
        while let Some(op) = self.peek_kind().and_then(Self::chainable_comparison) {
            if !matches!(
                result,
                Expression::BinaryOp(
                    _,
                    BinaryOp::Eq
                        | BinaryOp::Neq
                        | BinaryOp::Lt
                        | BinaryOp::Gt
                        | BinaryOp::Lte
                        | BinaryOp::Gte,
                    _
                ) | Expression::BinaryOp(_, BinaryOp::And, _)
            ) {
                break;
            }
            self.advance();
            let next = self.parse_additive()?;
            let cmp = Expression::BinaryOp(Box::new(prev), op, Box::new(next.clone()));
            result = Expression::BinaryOp(Box::new(result), BinaryOp::And, Box::new(cmp));
            prev = next;
        }

        Ok(result)
    }

    fn chainable_comparison(kind: &TokenKind) -> Option<BinaryOp> {
        Some(match kind {
            TokenKind::Eq => BinaryOp::Eq,
            TokenKind::Neq => BinaryOp::Neq,
            TokenKind::Lt => BinaryOp::Lt,
            TokenKind::Gt => BinaryOp::Gt,
            TokenKind::Lte => BinaryOp::Lte,
            TokenKind::Gte => BinaryOp::Gte,
            _ => return None,
        })
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
                Some(TokenKind::Percent) => BinaryOp::Mod,
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
        loop {
            // `$param.key` / `$param['key'].nested`: パラメータ（とそのキー参照の結果）に
            // 続く `.key` はマップのキー参照として IndexAccess にする。`n.prop` は
            // parse_primary が Property として読むので、ここでは扱わない。
            if self.check(TokenKind::Dot)
                && matches!(expr, Expression::Parameter(_) | Expression::IndexAccess(..))
                && self.next_is_ident_or_keyword()
            {
                self.advance(); // consume .
                let key = self.expect_ident_or_keyword()?;
                expr = Expression::IndexAccess(
                    Box::new(expr),
                    Box::new(Expression::Literal(Literal::String(key))),
                );
                continue;
            }
            if !self.check(TokenKind::LBracket) {
                break;
            }
            self.advance(); // consume [
            // `list[..end]`: 開始を省略したスライス
            let start = if self.check(TokenKind::Dot) {
                None
            } else {
                Some(self.parse_expression()?)
            };
            if self.check(TokenKind::Dot) {
                // List slice: list[start..end] - consume two Dot tokens
                self.advance(); // first dot
                self.expect(TokenKind::Dot)?; // second dot
                // `list[start..]`: 終わりを省略したスライス（null は末尾まで）
                let end = if self.check(TokenKind::RBracket) {
                    Expression::Literal(Literal::Null)
                } else {
                    self.parse_expression()?
                };
                self.expect(TokenKind::RBracket)?;
                let start = start.unwrap_or(Expression::Literal(Literal::Int(0)));
                expr = Expression::ListSlice(Box::new(expr), Box::new(start), Box::new(end));
            } else {
                let start = start.ok_or_else(|| self.unexpected_token("index"))?;
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
                    let has_labels_or_props = !start_node.labels.is_empty()
                        || !start_node.properties.is_empty()
                        || start_node.properties_param.is_some();
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
                                variable: None,
                                start: start_node,
                                segments,
                                shortest: None,
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
            Some(TokenKind::LBrace) => {
                // Map literal: {key: expr, key: expr, ...}
                let props = self.parse_properties()?;
                Ok(Expression::Map(props))
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
                        "date" => {
                            self.advance(); // consume '('
                            let arg = if self.check(TokenKind::RParen) {
                                None
                            } else {
                                Some(Box::new(self.parse_expression()?))
                            };
                            self.expect(TokenKind::RParen)?;
                            return Ok(Expression::ScalarFn(ScalarFunction::DateFunc(arg)));
                        }
                        "datetime" => {
                            self.advance(); // consume '('
                            let arg = if self.check(TokenKind::RParen) {
                                None
                            } else {
                                Some(Box::new(self.parse_expression()?))
                            };
                            self.expect(TokenKind::RParen)?;
                            return Ok(Expression::ScalarFn(ScalarFunction::DateTimeFunc(arg)));
                        }
                        "duration" => {
                            self.advance(); // consume '('
                            let arg = self.parse_expression()?;
                            self.expect(TokenKind::RParen)?;
                            return Ok(Expression::ScalarFn(ScalarFunction::DurationFunc(
                                Box::new(arg),
                            )));
                        }
                        _ => {
                            // Any other function: reuse the RETURN-item function
                            // parser, which knows every scalar function.
                            return match self.parse_aggregate_function(&var)? {
                                ReturnItem::Function(f) => Ok(Expression::ScalarFn(f)),
                                ReturnItem::Expr(e) => Ok(e),
                                _ => Err(ParseError::UnexpectedToken {
                                    expected: "scalar function".to_string(),
                                    found: format!("aggregate function {var}()"),
                                    span: self.current_span(),
                                }),
                            };
                        }
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
                // ラベル述語: `n:Person`、`n:Person:Admin`
                if self.check(TokenKind::Colon) && self.next_is_ident_or_keyword() {
                    let mut labels = Vec::new();
                    while self.check(TokenKind::Colon) {
                        self.advance();
                        labels.push(self.expect_ident_or_keyword()?);
                    }
                    return Ok(Expression::HasLabels(var, labels));
                }
                match self.try_consume_dot_property()? {
                    Some(prop) => Ok(Expression::Property(var, prop)),
                    None => Ok(Expression::Variable(var)),
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

        // パターン内包 `[(a)-[:R]->(b) WHERE cond | expr]` は
        // `COLLECT { MATCH (a)-[:R]->(b) WHERE cond RETURN expr }` と同じ
        if self.check(TokenKind::LParen) {
            let saved = self.pos;
            if let Ok(Pattern::Path(path)) = self.parse_pattern()
                && (self.check(TokenKind::Where) || self.check(TokenKind::Pipe))
            {
                let where_clause = if self.check(TokenKind::Where) {
                    self.advance();
                    Some(self.parse_expression()?)
                } else {
                    None
                };
                self.expect(TokenKind::Pipe)?;
                let projection = self.parse_expression()?;
                self.expect(TokenKind::RBracket)?;
                return Ok(Expression::CollectSubquery(Box::new(CollectSubqueryBody {
                    patterns: vec![Pattern::Path(path)],
                    where_clause,
                    return_item: ReturnItem::Expr(projection),
                })));
            }
            self.pos = saved;
        }

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
            // `[x IN list WHERE pred]`: 射影を省略すると要素そのもの
            let result = if self.check(TokenKind::Pipe) {
                self.advance();
                Box::new(self.parse_expression()?)
            } else if predicate.is_some() {
                Box::new(Expression::Variable(variable.clone()))
            } else {
                return Err(self.unexpected_token("| or WHERE"));
            };
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

    /// 次の次のトークンが識別子（または識別子として読めるキーワード）か。
    fn next_is_ident_or_keyword(&self) -> bool {
        match self.tokens.get(self.pos + 1).map(|t| &t.kind) {
            Some(TokenKind::Ident(_)) => true,
            Some(kind) => crate::lexer::keyword_as_ident(kind).is_some(),
            None => false,
        }
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
        self.peek_kind()
            .is_some_and(|k| std::mem::discriminant(k) == std::mem::discriminant(&kind))
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
mod tests;
