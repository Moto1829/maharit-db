use std::collections::HashMap;

use thiserror::Error;

use crate::ast::*;
use crate::lexer::{Lexer, Span, Token, TokenKind};

mod expr;
mod pattern;
mod schema;
mod statement;
mod subquery;

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
    /// 元の入力。キーワードと同じ綴りの名前（`:User`、`n.order`）を、書かれたとおりに取り出すのに使う。
    source: String,
}

impl Parser {
    pub fn new(input: &str) -> Result<Self, ParseError> {
        let tokens = Lexer::new(input).tokenize()?;
        Ok(Self {
            tokens,
            pos: 0,
            source: input.to_string(),
        })
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
