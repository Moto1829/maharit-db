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
        match self.peek_kind() {
            Some(TokenKind::Create) => self.parse_create(),
            Some(TokenKind::Match) => self.parse_match_or_delete(),
            Some(_) => Err(self.unexpected_token("CREATE or MATCH")),
            None => Err(ParseError::UnexpectedEof),
        }
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
        self.expect(TokenKind::Match)?;

        let mut patterns = Vec::new();
        patterns.push(self.parse_pattern()?);

        while self.check(TokenKind::Comma) {
            self.advance();
            patterns.push(self.parse_pattern()?);
        }

        // WHERE clause (optional)
        let where_clause = if self.check(TokenKind::Where) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };

        // SET clause (optional, before DELETE or RETURN)
        let set_clause = if self.check(TokenKind::Set) {
            Some(self.parse_set_clause()?)
        } else {
            None
        };

        // DELETE or RETURN
        if self.check(TokenKind::Delete) || self.check(TokenKind::Detach) {
            let delete_clause = self.parse_delete_clause()?;
            Ok(Statement::Delete(DeleteStatement {
                patterns,
                where_clause,
                set_clause,
                delete_clause,
            }))
        } else {
            // RETURN clause
            self.expect(TokenKind::Return)?;
            let return_clause = self.parse_return_clause()?;

            Ok(Statement::Match(MatchStatement {
                patterns,
                where_clause,
                return_clause,
            }))
        }
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
                    expected: "aggregate function (COUNT, SUM, AVG, MIN, MAX, COLLECT)".to_string(),
                    found: func_name.to_string(),
                    span: self.current_span(),
                });
            }
        };

        Ok(ReturnItem::Aggregate(aggregate))
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

        // Check for min value
        if let Some(TokenKind::Int(n)) = self.peek_kind().cloned() {
            min = n as u32;
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
        } else {
            // No .., so min is also max (exact count)
            max = Some(min);
        }

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
            assert_eq!(m.patterns.len(), 1);
            assert!(m.where_clause.is_none());
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
            let where_clause = m.where_clause.unwrap();
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
            let where_clause = m.where_clause.unwrap();
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
            if let Pattern::Path(path) = &m.patterns[0] {
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
            if let Pattern::Path(path) = &m.patterns[0] {
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
            if let Pattern::Path(path) = &m.patterns[0] {
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
}
