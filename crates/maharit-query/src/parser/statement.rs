//! 文と句（CREATE / MATCH / WITH / RETURN / ORDER BY / SET / DELETE / REMOVE / MERGE /
//! UNWIND / FOREACH）の構文解析と、句を連ねた文・UNION の組み立て。

use super::*;

impl Parser {
    /// 次のトークンが句の始まりか（決まった形の文の後に続いていたら句の列として読み直す）
    pub(super) fn at_pipeline_clause(&self) -> bool {
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
    pub(super) fn parse_pipeline(&mut self) -> Result<Statement, ParseError> {
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

    pub(super) fn parse_fixed_statement(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_union(&mut self, first: Statement) -> Result<Statement, ParseError> {
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

    pub(super) fn parse_create(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_optional_return(&mut self) -> Result<Option<ReturnClause>, ParseError> {
        if self.check(TokenKind::Return) {
            self.advance();
            Ok(Some(self.parse_return_clause()?))
        } else {
            Ok(None)
        }
    }

    // ========== MATCH ==========

    pub(super) fn parse_match_or_delete(&mut self) -> Result<Statement, ParseError> {
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

    pub(super) fn parse_query_segment(
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
    pub(super) fn parse_unwind_clause(&mut self) -> Result<UnwindClause, ParseError> {
        self.expect(TokenKind::Unwind)?;
        let expression = self.parse_expression()?;
        self.expect(TokenKind::As)?;
        let variable = self.expect_ident()?;
        Ok(UnwindClause {
            expression,
            variable,
        })
    }

    pub(super) fn parse_with_clause(&mut self) -> Result<WithClause, ParseError> {
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

    pub(super) fn parse_with_item(&mut self) -> Result<WithItem, ParseError> {
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

    pub(super) fn parse_match_clause(&mut self, optional: bool) -> Result<MatchClause, ParseError> {
        let mut patterns = Vec::new();
        patterns.push(self.parse_pattern()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }

        Ok(MatchClause { patterns, optional })
    }

    // ========== SET ==========

    pub(super) fn parse_set_clause(&mut self) -> Result<SetClause, ParseError> {
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

    pub(super) fn parse_set_item(&mut self) -> Result<SetItem, ParseError> {
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

    pub(super) fn parse_delete_clause(&mut self) -> Result<DeleteClause, ParseError> {
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

    pub(super) fn parse_return_clause(&mut self) -> Result<ReturnClause, ParseError> {
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

    pub(super) fn parse_order_by_clause(&mut self) -> Result<OrderByClause, ParseError> {
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

    pub(super) fn parse_order_by_item(&mut self) -> Result<OrderByItem, ParseError> {
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
    pub(super) fn parse_skip_limit_expr(&mut self) -> Result<Expression, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::Int(n)) if *n < 0 => Err(self.unexpected_token("non-negative integer")),
            _ => self.parse_expression(),
        }
    }

    pub(super) fn parse_return_item(&mut self) -> Result<ReturnItem, ParseError> {
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
    pub(super) fn at_expression_operator(&self) -> bool {
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

    pub(super) fn parse_simple_return_item(&mut self) -> Result<ReturnItem, ParseError> {
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

    // ========== CREATE clause (for MATCH+CREATE) ==========

    pub(super) fn parse_create_clause(&mut self) -> Result<CreateClause, ParseError> {
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
    pub(super) fn parse_trailing_updates(&mut self) -> Result<Vec<UpdateClause>, ParseError> {
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

    pub(super) fn parse_remove_clause(&mut self) -> Result<RemoveClause, ParseError> {
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

    pub(super) fn parse_remove_item(&mut self) -> Result<RemoveItem, ParseError> {
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

    pub(super) fn parse_merge(
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

    pub(super) fn parse_standalone_return(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Return)?;
        let return_clause = self.parse_return_clause()?;
        Ok(Statement::Return(return_clause))
    }

    // ========== UNWIND ==========

    pub(super) fn parse_unwind(&mut self) -> Result<Statement, ParseError> {
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

    pub(super) fn parse_foreach(&mut self) -> Result<Statement, ParseError> {
        let stmt = self.parse_foreach_statement()?;
        Ok(Statement::Foreach(stmt))
    }

    pub(super) fn parse_foreach_statement(&mut self) -> Result<ForeachStatement, ParseError> {
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

    pub(super) fn parse_foreach_clauses(&mut self) -> Result<Vec<ForeachClause>, ParseError> {
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
}
