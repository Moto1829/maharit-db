//! 式の構文解析（演算子の優先順位、関数・集約の呼び出し、リスト・CASE・リテラル）。

use super::*;

impl Parser {
    pub(super) fn parse_aggregate_function(
        &mut self,
        func_name: &str,
    ) -> Result<ReturnItem, ParseError> {
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

    // ========== Expression ==========

    pub(super) fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        self.parse_or_expression()
    }

    pub(super) fn parse_or_expression(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_xor_expression()?;

        while self.check(TokenKind::Or) {
            self.advance();
            let right = self.parse_xor_expression()?;
            left = Expression::BinaryOp(Box::new(left), BinaryOp::Or, Box::new(right));
        }

        Ok(left)
    }

    /// 優先順位は OR < XOR < AND（openCypher と同じ）
    pub(super) fn parse_xor_expression(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_and_expression()?;

        while self.check(TokenKind::Xor) {
            self.advance();
            let right = self.parse_and_expression()?;
            left = Expression::BinaryOp(Box::new(left), BinaryOp::Xor, Box::new(right));
        }

        Ok(left)
    }

    pub(super) fn parse_and_expression(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_not()?;

        while self.check(TokenKind::And) {
            self.advance();
            let right = self.parse_not()?;
            left = Expression::BinaryOp(Box::new(left), BinaryOp::And, Box::new(right));
        }

        Ok(left)
    }

    /// `NOT` は `AND` より強く、比較より弱い（`NOT a = b` は `NOT (a = b)`）。
    pub(super) fn parse_not(&mut self) -> Result<Expression, ParseError> {
        if self.check(TokenKind::Not) {
            self.advance();
            let expr = self.parse_not()?;
            return Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(expr)));
        }
        self.parse_comparison()
    }

    pub(super) fn parse_comparison(&mut self) -> Result<Expression, ParseError> {
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

    pub(super) fn chainable_comparison(kind: &TokenKind) -> Option<BinaryOp> {
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

    pub(super) fn parse_additive(&mut self) -> Result<Expression, ParseError> {
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

    pub(super) fn parse_multiplicative(&mut self) -> Result<Expression, ParseError> {
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

    pub(super) fn parse_unary(&mut self) -> Result<Expression, ParseError> {
        if self.check(TokenKind::Dash) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expression::UnaryOp(UnaryOp::Neg, Box::new(expr)));
        }

        self.parse_postfix()
    }

    pub(super) fn parse_postfix(&mut self) -> Result<Expression, ParseError> {
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

    pub(super) fn parse_primary(&mut self) -> Result<Expression, ParseError> {
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
            Some(TokenKind::Parameter(name)) => {
                let name = name.clone();
                self.advance();
                Ok(Expression::Parameter(name))
            }
            Some(_) => Err(self.unexpected_token("expression")),
            None => Err(ParseError::UnexpectedEof),
        }
    }

    pub(super) fn parse_list_expression(&mut self) -> Result<Expression, ParseError> {
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

    pub(super) fn parse_case_expression(&mut self) -> Result<Expression, ParseError> {
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

    pub(super) fn parse_literal(&mut self) -> Result<Literal, ParseError> {
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
}
