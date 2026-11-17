//! CALL サブクエリ、プロシージャ呼び出し（YIELD とその後に続く句）、EXISTS / COUNT / COLLECT サブクエリ。

use super::*;

impl Parser {
    // ========== Subqueries ==========

    /// CALL { [WITH var, ...] MATCH pattern [WHERE expr] RETURN item [AS alias], ... }
    pub(super) fn parse_call_subquery(&mut self) -> Result<CallSubquery, ParseError> {
        self.expect(TokenKind::Call)?;
        self.expect(TokenKind::LBrace)?;
        let subquery = self.parse_call_subquery_branch()?;
        self.expect(TokenKind::RBrace)?;
        Ok(subquery)
    }

    /// One branch of a CALL subquery body: `[WITH vars] MATCH … [WHERE …] RETURN …`,
    /// followed by any `UNION [ALL]` branches.
    pub(super) fn parse_call_subquery_branch(&mut self) -> Result<CallSubquery, ParseError> {
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

    pub(super) fn parse_call_return_item(&mut self) -> Result<CallReturnItem, ParseError> {
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
    pub(super) fn parse_procedure_call(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_procedure_continuation(
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
    pub(super) fn try_consume_dot_property(&mut self) -> Result<Option<String>, ParseError> {
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
    pub(super) fn expect_dot_property(&mut self) -> Result<String, ParseError> {
        self.expect(TokenKind::Dot)?;
        self.expect_ident_or_keyword()
    }

    /// プロパティ名 / プロシージャ名パート / マップキーなど、
    /// コンテキスト上は識別子として扱える位置で識別子またはキーワードを読み取る。
    ///
    /// Cypher 標準ではキーワードは予約語ではなく、識別子位置に現れたときは
    /// 識別子として解釈される。lexer 段階で大文字小文字を区別しないキーワード化を
    /// 行っているため、ここで再び識別子に戻す。
    pub(super) fn expect_ident_or_keyword(&mut self) -> Result<String, ParseError> {
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
    pub(super) fn parse_subquery_pattern(&mut self) -> Result<SubqueryPattern, ParseError> {
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
    pub(super) fn parse_collect_subquery_body(
        &mut self,
    ) -> Result<CollectSubqueryBody, ParseError> {
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

    pub(super) fn current_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or_default()
    }
}
