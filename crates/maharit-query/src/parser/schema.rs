//! 制約・索引・全文索引の作成と削除、SHOW、ユーザー管理と ACL の文の構文解析。

use super::*;

impl Parser {
    // ========== CONSTRAINT DDL ==========

    /// CREATE CONSTRAINT name FOR pattern REQUIRE ...
    ///
    /// Supported forms:
    /// - `FOR (n:Label) REQUIRE n.prop IS UNIQUE/NOT NULL/:: TYPE`
    /// - `FOR (n:Label1) REQUIRE n:Label2`  (RequiredLabel)
    /// - `FOR (s:SLabel)-[r:EType]->(t:TLabel)`  (EndpointLabel, no REQUIRE needed)
    pub(super) fn parse_create_constraint(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_endpoint_label_constraint(
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
    pub(super) fn parse_drop_constraint(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Drop)?;
        self.expect(TokenKind::Constraint)?;

        let name = self.expect_ident()?;

        Ok(Statement::DropConstraint(DropConstraintStatement { name }))
    }

    /// SHOW CONSTRAINTS / SHOW USERS
    pub(super) fn parse_show(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_create_fulltext_index(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_drop_fulltext_index(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_create_index(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_drop_index(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_role_value(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::String(_)) => self.expect_string(),
            _ => self.expect_ident(),
        }
    }

    pub(super) fn parse_create_user(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn parse_drop_user(&mut self) -> Result<Statement, ParseError> {
        self.expect(TokenKind::Drop)?;
        self.expect(TokenKind::User)?;
        let username = self.expect_ident()?;

        Ok(Statement::DropUser(DropUserStatement { username }))
    }

    /// ALTER USER username SET PASSWORD 'pass' / SET ROLE role
    pub(super) fn parse_alter_user(&mut self) -> Result<Statement, ParseError> {
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
    pub(super) fn expect_word(&mut self, word: &str) -> Result<(), ParseError> {
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
    pub(super) fn parse_acl(&mut self) -> Result<Statement, ParseError> {
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
}
