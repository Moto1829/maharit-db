//! パターン（ノード・リレーションシップ・可変長・shortestPath）とプロパティマップの構文解析。

use super::*;

impl Parser {
    // ========== Pattern ==========

    pub(super) fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
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
    pub(super) fn peek_shortest_path_kind(&self) -> Option<ShortestPathKind> {
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

    pub(super) fn parse_node_pattern(&mut self) -> Result<NodePattern, ParseError> {
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

    pub(super) fn is_edge_start(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(TokenKind::Dash) | Some(TokenKind::ArrowLeft)
        )
    }

    pub(super) fn parse_edge_pattern(&mut self) -> Result<EdgePattern, ParseError> {
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

    pub(super) fn parse_length_range(&mut self) -> Result<LengthRange, ParseError> {
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
    pub(super) fn parse_pattern_properties(
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

    pub(super) fn parse_properties(&mut self) -> Result<HashMap<String, Expression>, ParseError> {
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
    pub(super) fn parse_property_key(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::String(_)) => self.expect_string(),
            _ => self.expect_ident_or_keyword(),
        }
    }
}
