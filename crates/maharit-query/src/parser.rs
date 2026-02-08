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
            Some(_) => {
                return Err(
                    self.unexpected_token("CREATE, MATCH, MERGE, UNWIND, DROP, SHOW, ALTER, EXPLAIN, or PROFILE"),
                )
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
        if set_clause.is_some() {
            let return_clause = if self.check(TokenKind::Return) {
                self.advance();
                Some(self.parse_return_clause()?)
            } else {
                None
            };
            return Ok(Statement::MatchSet(MatchSetStatement {
                segments: vec![first_segment.clone()],
                where_clause: first_segment.where_clause,
                set_clause: set_clause.unwrap(),
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
                if set_cl.is_some() {
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
                        set_clause: set_cl.unwrap(),
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

        // Parse final RETURN clause
        self.expect(TokenKind::Return)?;
        let return_clause = self.parse_return_clause()?;

        Ok(Statement::Match(MatchStatement {
            segments,
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
            Some(self.parse_positive_int()?)
        } else {
            None
        };

        // LIMIT (optional)
        let limit = if self.check(TokenKind::Limit) {
            self.advance();
            Some(self.parse_positive_int()?)
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
        self.expect(TokenKind::Dot)?;
        let property = self.expect_ident()?;
        self.expect(TokenKind::Eq)?;
        let value = self.parse_expression()?;

        Ok(SetItem {
            variable,
            property,
            value,
        })
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
            Some(self.parse_positive_int()?)
        } else {
            None
        };

        // LIMIT (optional)
        let limit = if self.check(TokenKind::Limit) {
            self.advance();
            Some(self.parse_positive_int()?)
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

    fn parse_positive_int(&mut self) -> Result<u64, ParseError> {
        match self.peek_kind().cloned() {
            Some(TokenKind::Int(n)) if n >= 0 => {
                self.advance();
                Ok(n as u64)
            }
            _ => Err(self.unexpected_token("positive integer")),
        }
    }

    fn parse_return_item(&mut self) -> Result<ReturnItem, ParseError> {
        if self.check(TokenKind::Star) {
            self.advance();
            return Ok(ReturnItem::All);
        }

        let var = self.expect_ident()?;

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
            _ => {}
        }

        // Aggregate functions
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
            "COUNT" => AggregateFunction::Count(inner),
            "SUM" => {
                let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: ")".to_string(),
                    span: self.current_span(),
                })?;
                AggregateFunction::Sum(inner)
            }
            "AVG" => {
                let inner = inner.ok_or_else(|| ParseError::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: ")".to_string(),
                    span: self.current_span(),
                })?;
                AggregateFunction::Avg(inner)
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
                AggregateFunction::Collect(inner)
            }
            _ => {
                return Err(ParseError::UnexpectedToken {
                    expected: "function (COUNT, SUM, AVG, MIN, MAX, COLLECT, nodes, relationships, length, shortestPath, allShortestPaths)".to_string(),
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

    // ========== CONSTRAINT DDL ==========

    /// CREATE CONSTRAINT name FOR (n:Label) REQUIRE n.prop IS UNIQUE/NOT NULL/:: TYPE
    fn parse_create_constraint(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Create)?;
        self.expect(TokenKind::Constraint)?;

        let name = self.expect_ident()?;

        self.expect(TokenKind::For)?;

        // Parse (variable:Label)
        self.expect(TokenKind::LParen)?;
        let variable = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let label = self.expect_ident()?;
        self.expect(TokenKind::RParen)?;

        self.expect(TokenKind::Require)?;

        // Parse variable.property or (variable.property, variable.property, ...)
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

        Ok(Statement::CreateFulltextIndex(CreateFulltextIndexStatement {
            name,
            label,
            variable,
            properties,
        }))
    }

    /// DROP FULLTEXT INDEX name
    fn parse_drop_fulltext_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Drop)?;
        self.expect(TokenKind::Fulltext)?;
        self.expect(TokenKind::Index)?;
        let name = self.expect_ident()?;

        Ok(Statement::DropFulltextIndex(DropFulltextIndexStatement { name }))
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
        let mut label = None;
        let mut properties = HashMap::new();

        // Variable name (optional)
        if let Some(TokenKind::Ident(_)) = self.peek_kind() {
            variable = Some(self.expect_ident()?);
        }

        // Label (optional)
        if self.check(TokenKind::Colon) {
            self.advance();
            label = Some(self.expect_ident()?);
        }

        // Properties (optional)
        if self.check(TokenKind::LBrace) {
            properties = self.parse_properties()?;
        }

        self.expect(TokenKind::RParen)?;

        Ok(NodePattern {
            variable,
            label,
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

    fn parse_properties(&mut self) -> Result<HashMap<String, Literal>, ParseError> {
        self.expect(TokenKind::LBrace)?;

        let mut props = HashMap::new();

        if !self.check(TokenKind::RBrace) {
            loop {
                let key = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let value = self.parse_literal()?;
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

        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expression, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::LParen) => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            Some(TokenKind::LBracket) => {
                // List literal: [expr, expr, ...]
                self.parse_list_expression()
            }
            Some(TokenKind::Case) => self.parse_case_expression(),
            Some(TokenKind::Ident(_)) => {
                let var = self.expect_ident()?;
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
            Some(_) => Err(self.unexpected_token("expression")),
            None => Err(ParseError::UnexpectedEof),
        }
    }

    fn parse_list_expression(&mut self) -> Result<Expression, ParseError> {
        self.expect(TokenKind::LBracket)?;

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
        self.peek_kind().map_or(false, |k| {
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
                assert_eq!(node.label, None);
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
                assert_eq!(node.label, Some("Person".to_string()));
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
                assert_eq!(node.label, Some("Person".to_string()));
                assert_eq!(
                    node.properties.get("name"),
                    Some(&Literal::String("Alice".to_string()))
                );
                assert_eq!(node.properties.get("age"), Some(&Literal::Int(30)));
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
                assert_eq!(path.start.label, Some("Person".to_string()));
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
            assert_eq!(set.items[0].variable, "n");
            assert_eq!(set.items[0].property, "age");
            assert_eq!(set.items[0].value, Expression::Literal(Literal::Int(31)));
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
            assert_eq!(set.items[0].variable, "n");
            assert_eq!(set.items[0].property, "age");
            assert_eq!(set.items[1].variable, "n");
            assert_eq!(set.items[1].property, "name");
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
        let stmt = parse(
            "MATCH (n:Person) WHERE CASE WHEN n.age < 20 THEN true ELSE false END RETURN n",
        )
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
        let stmt =
            parse("MATCH (n:Person) WITH n ORDER BY n.age RETURN n").unwrap();

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
            assert_eq!(with.limit, Some(10));
        } else {
            panic!("expected MATCH statement");
        }
    }

    #[test]
    fn test_with_skip() {
        let stmt = parse("MATCH (n:Person) WITH n SKIP 5 RETURN n").unwrap();

        if let Statement::Match(m) = stmt {
            let with = m.segments[0].with_clause.as_ref().unwrap();
            assert_eq!(with.skip, Some(5));
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
        let stmt =
            parse("MATCH (n:Person) WITH n.city AS city, COUNT(*) AS count WHERE count > 5 RETURN city")
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
        let stmt = parse(
            "MATCH (n:Person) WITH n.city AS city WITH city RETURN city",
        )
        .unwrap();

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
        let stmt = parse(
            "MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name",
        )
        .unwrap();

        if let Statement::Union(u) = stmt {
            assert_eq!(u.queries.len(), 2);
            assert_eq!(u.union_type, UnionType::Union);
        } else {
            panic!("expected UNION statement");
        }
    }

    #[test]
    fn test_union_all_parse() {
        let stmt = parse(
            "MATCH (n:Person) RETURN n.name UNION ALL MATCH (n:Company) RETURN n.name",
        )
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
        let stmt = parse(
            r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31 RETURN n"#,
        )
        .unwrap();

        if let Statement::MatchSet(ms) = stmt {
            assert_eq!(ms.set_clause.items.len(), 1);
            assert!(ms.return_clause.is_some());
        } else {
            panic!("expected MatchSet statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_match_set_multiple() {
        let stmt = parse(
            r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31, n.city = "Tokyo" RETURN n"#,
        )
        .unwrap();

        if let Statement::MatchSet(ms) = stmt {
            assert_eq!(ms.set_clause.items.len(), 2);
        } else {
            panic!("expected MatchSet statement");
        }
    }

    #[test]
    fn test_parse_match_set_no_return() {
        let stmt = parse(
            r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31"#,
        )
        .unwrap();

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
        let stmt = parse(
            r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25"#,
        )
        .unwrap();

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
        let stmt = parse(
            r#"MERGE (n:Person {name: "Alice"}) RETURN n"#,
        )
        .unwrap();

        if let Statement::Merge(m) = stmt {
            assert!(m.return_clause.is_some());
        } else {
            panic!("expected Merge statement");
        }
    }

    // ========== MATCH + REMOVE tests ==========

    #[test]
    fn test_parse_match_remove_property() {
        let stmt = parse(
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age RETURN n"#,
        )
        .unwrap();

        if let Statement::MatchRemove(mr) = stmt {
            assert_eq!(mr.remove_clause.items.len(), 1);
            assert!(matches!(mr.remove_clause.items[0], RemoveItem::Property(_, _)));
            assert!(mr.return_clause.is_some());
        } else {
            panic!("expected MatchRemove statement, got {:?}", stmt);
        }
    }

    #[test]
    fn test_parse_match_remove_label() {
        let stmt = parse(
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n:Person RETURN n"#,
        )
        .unwrap();

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
        let stmt = parse(
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age, n.city RETURN n"#,
        )
        .unwrap();

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
}
