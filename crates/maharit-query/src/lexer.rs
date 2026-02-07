use std::fmt;
use std::iter::Peekable;
use std::str::Chars;

use thiserror::Error;

/// ソースコード上の位置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn new(start: usize, end: usize, line: usize, column: usize) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }
}

/// トークンの種別
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // キーワード
    Create,
    Match,
    Where,
    Return,
    Delete,
    Set,
    Detach,
    And,
    Or,
    Not,
    True,
    False,
    Null,
    // ソート・ページネーション用キーワード
    Order,
    By,
    Asc,
    Desc,
    Limit,
    Skip,
    Distinct,
    Nulls,
    First,
    Last,
    // 高度なクエリ用キーワード
    Optional,
    With,
    As,
    Case,
    When,
    Then,
    Else,
    End,
    Union,
    All,
    // 複合クエリ用キーワード
    Merge,
    Remove,
    Unwind,
    On,
    // スキーマ制約用キーワード
    Constraint,
    Require,
    Unique,
    Show,
    Drop,
    For,
    Is,

    // 識別子とリテラル
    Ident(String),
    Int(i64),
    Float(f64),
    String(String),

    // 比較演算子
    Eq,         // =
    Neq,        // <>
    Lt,         // <
    Gt,         // >
    Lte,        // <=
    Gte,        // >=
    RegexMatch, // =~

    // 算術演算子
    Plus,  // +
    Minus, // -
    Star,  // *
    Slash, // /

    // 記号
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    LBrace,   // {
    RBrace,   // }
    Colon,    // :
    Comma,    // ,
    Dot,      // .
    Pipe,     // |

    // 矢印パターン
    Arrow,     // ->
    ArrowLeft, // <-
    Dash,      // -

    // 終端
    Eof,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Create => write!(f, "CREATE"),
            TokenKind::Match => write!(f, "MATCH"),
            TokenKind::Where => write!(f, "WHERE"),
            TokenKind::Return => write!(f, "RETURN"),
            TokenKind::Delete => write!(f, "DELETE"),
            TokenKind::Set => write!(f, "SET"),
            TokenKind::Detach => write!(f, "DETACH"),
            TokenKind::And => write!(f, "AND"),
            TokenKind::Or => write!(f, "OR"),
            TokenKind::Not => write!(f, "NOT"),
            TokenKind::True => write!(f, "true"),
            TokenKind::False => write!(f, "false"),
            TokenKind::Null => write!(f, "null"),
            TokenKind::Order => write!(f, "ORDER"),
            TokenKind::By => write!(f, "BY"),
            TokenKind::Asc => write!(f, "ASC"),
            TokenKind::Desc => write!(f, "DESC"),
            TokenKind::Limit => write!(f, "LIMIT"),
            TokenKind::Skip => write!(f, "SKIP"),
            TokenKind::Distinct => write!(f, "DISTINCT"),
            TokenKind::Nulls => write!(f, "NULLS"),
            TokenKind::First => write!(f, "FIRST"),
            TokenKind::Last => write!(f, "LAST"),
            TokenKind::Optional => write!(f, "OPTIONAL"),
            TokenKind::With => write!(f, "WITH"),
            TokenKind::As => write!(f, "AS"),
            TokenKind::Case => write!(f, "CASE"),
            TokenKind::When => write!(f, "WHEN"),
            TokenKind::Then => write!(f, "THEN"),
            TokenKind::Else => write!(f, "ELSE"),
            TokenKind::End => write!(f, "END"),
            TokenKind::Union => write!(f, "UNION"),
            TokenKind::All => write!(f, "ALL"),
            TokenKind::Merge => write!(f, "MERGE"),
            TokenKind::Remove => write!(f, "REMOVE"),
            TokenKind::Unwind => write!(f, "UNWIND"),
            TokenKind::On => write!(f, "ON"),
            TokenKind::Constraint => write!(f, "CONSTRAINT"),
            TokenKind::Require => write!(f, "REQUIRE"),
            TokenKind::Unique => write!(f, "UNIQUE"),
            TokenKind::Show => write!(f, "SHOW"),
            TokenKind::Drop => write!(f, "DROP"),
            TokenKind::For => write!(f, "FOR"),
            TokenKind::Is => write!(f, "IS"),
            TokenKind::Ident(s) => write!(f, "{}", s),
            TokenKind::Int(n) => write!(f, "{}", n),
            TokenKind::Float(n) => write!(f, "{}", n),
            TokenKind::String(s) => write!(f, "\"{}\"", s),
            TokenKind::Eq => write!(f, "="),
            TokenKind::Neq => write!(f, "<>"),
            TokenKind::Lt => write!(f, "<"),
            TokenKind::Gt => write!(f, ">"),
            TokenKind::Lte => write!(f, "<="),
            TokenKind::Gte => write!(f, ">="),
            TokenKind::RegexMatch => write!(f, "=~"),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::LParen => write!(f, "("),
            TokenKind::RParen => write!(f, ")"),
            TokenKind::LBracket => write!(f, "["),
            TokenKind::RBracket => write!(f, "]"),
            TokenKind::LBrace => write!(f, "{{"),
            TokenKind::RBrace => write!(f, "}}"),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Comma => write!(f, ","),
            TokenKind::Dot => write!(f, "."),
            TokenKind::Pipe => write!(f, "|"),
            TokenKind::Arrow => write!(f, "->"),
            TokenKind::ArrowLeft => write!(f, "<-"),
            TokenKind::Dash => write!(f, "-"),
            TokenKind::Eof => write!(f, "EOF"),
        }
    }
}

/// トークン
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// レクサーエラー
#[derive(Debug, Clone, Error, PartialEq)]
pub enum LexerError {
    #[error("unexpected character '{0}' at line {1}, column {2}")]
    UnexpectedChar(char, usize, usize),

    #[error("unterminated string at line {0}, column {1}")]
    UnterminatedString(usize, usize),

    #[error("invalid number format at line {0}, column {1}")]
    InvalidNumber(usize, usize),
}

/// レクサー
pub struct Lexer<'a> {
    input: &'a str,
    chars: Peekable<Chars<'a>>,
    pos: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.chars().peekable(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    /// 全トークンを取得
    pub fn tokenize(mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_eof = token.kind == TokenKind::Eof;
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    /// 次のトークンを取得
    pub fn next_token(&mut self) -> Result<Token, LexerError> {
        self.skip_whitespace();

        let start = self.pos;
        let line = self.line;
        let column = self.column;

        let Some(ch) = self.peek() else {
            return Ok(Token::new(
                TokenKind::Eof,
                Span::new(start, start, line, column),
            ));
        };

        let kind = match ch {
            // 識別子またはキーワード
            'a'..='z' | 'A'..='Z' | '_' => self.read_ident(),

            // 数値
            '0'..='9' => self.read_number()?,

            // 文字列
            '"' | '\'' => self.read_string()?,

            // 演算子と記号
            '=' => {
                self.advance();
                if self.peek() == Some('~') {
                    self.advance();
                    TokenKind::RegexMatch
                } else {
                    TokenKind::Eq
                }
            }
            '<' => {
                self.advance();
                if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::Neq
                } else if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::Lte
                } else if self.peek() == Some('-') {
                    self.advance();
                    TokenKind::ArrowLeft
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::Gte
                } else {
                    TokenKind::Gt
                }
            }
            '+' => {
                self.advance();
                TokenKind::Plus
            }
            '-' => {
                self.advance();
                if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Dash
                }
            }
            '*' => {
                self.advance();
                TokenKind::Star
            }
            '/' => {
                self.advance();
                // コメント対応（将来的に）
                TokenKind::Slash
            }
            '(' => {
                self.advance();
                TokenKind::LParen
            }
            ')' => {
                self.advance();
                TokenKind::RParen
            }
            '[' => {
                self.advance();
                TokenKind::LBracket
            }
            ']' => {
                self.advance();
                TokenKind::RBracket
            }
            '{' => {
                self.advance();
                TokenKind::LBrace
            }
            '}' => {
                self.advance();
                TokenKind::RBrace
            }
            ':' => {
                self.advance();
                TokenKind::Colon
            }
            ',' => {
                self.advance();
                TokenKind::Comma
            }
            '.' => {
                self.advance();
                TokenKind::Dot
            }
            '|' => {
                self.advance();
                TokenKind::Pipe
            }
            _ => {
                return Err(LexerError::UnexpectedChar(ch, line, column));
            }
        };

        Ok(Token::new(kind, Span::new(start, self.pos, line, column)))
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_ident(&mut self) -> TokenKind {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_alphanumeric() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }
        let ident = &self.input[start..self.pos];

        // キーワードのチェック（大文字小文字を区別しない）
        match ident.to_uppercase().as_str() {
            "CREATE" => TokenKind::Create,
            "MATCH" => TokenKind::Match,
            "WHERE" => TokenKind::Where,
            "RETURN" => TokenKind::Return,
            "DELETE" => TokenKind::Delete,
            "SET" => TokenKind::Set,
            "DETACH" => TokenKind::Detach,
            "AND" => TokenKind::And,
            "OR" => TokenKind::Or,
            "NOT" => TokenKind::Not,
            "TRUE" => TokenKind::True,
            "FALSE" => TokenKind::False,
            "NULL" => TokenKind::Null,
            "ORDER" => TokenKind::Order,
            "BY" => TokenKind::By,
            "ASC" => TokenKind::Asc,
            "DESC" => TokenKind::Desc,
            "LIMIT" => TokenKind::Limit,
            "SKIP" => TokenKind::Skip,
            "DISTINCT" => TokenKind::Distinct,
            "NULLS" => TokenKind::Nulls,
            "FIRST" => TokenKind::First,
            "LAST" => TokenKind::Last,
            "OPTIONAL" => TokenKind::Optional,
            "WITH" => TokenKind::With,
            "AS" => TokenKind::As,
            "CASE" => TokenKind::Case,
            "WHEN" => TokenKind::When,
            "THEN" => TokenKind::Then,
            "ELSE" => TokenKind::Else,
            "END" => TokenKind::End,
            "UNION" => TokenKind::Union,
            "ALL" => TokenKind::All,
            "MERGE" => TokenKind::Merge,
            "REMOVE" => TokenKind::Remove,
            "UNWIND" => TokenKind::Unwind,
            "ON" => TokenKind::On,
            "CONSTRAINT" => TokenKind::Constraint,
            "REQUIRE" => TokenKind::Require,
            "UNIQUE" => TokenKind::Unique,
            "SHOW" => TokenKind::Show,
            "DROP" => TokenKind::Drop,
            "FOR" => TokenKind::For,
            "IS" => TokenKind::Is,
            _ => TokenKind::Ident(ident.to_string()),
        }
    }

    fn read_number(&mut self) -> Result<TokenKind, LexerError> {
        let start = self.pos;
        let line = self.line;
        let column = self.column;
        let mut has_dot = false;

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                self.advance();
            } else if ch == '.' && !has_dot {
                // Look ahead: if the next char after '.' is also '.', don't consume it
                // This handles range syntax like "2..5"
                let next_pos = self.pos + 1;
                if next_pos < self.input.len() && self.input.as_bytes().get(next_pos) == Some(&b'.')
                {
                    break;
                }
                // Also check if the char after '.' is a digit (valid float) or not
                if next_pos < self.input.len() {
                    let next_char = self.input.as_bytes().get(next_pos);
                    if !matches!(next_char, Some(b'0'..=b'9')) {
                        break;
                    }
                }
                has_dot = true;
                self.advance();
            } else {
                break;
            }
        }

        let num_str = &self.input[start..self.pos];

        if has_dot {
            num_str
                .parse::<f64>()
                .map(TokenKind::Float)
                .map_err(|_| LexerError::InvalidNumber(line, column))
        } else {
            num_str
                .parse::<i64>()
                .map(TokenKind::Int)
                .map_err(|_| LexerError::InvalidNumber(line, column))
        }
    }

    fn read_string(&mut self) -> Result<TokenKind, LexerError> {
        let line = self.line;
        let column = self.column;
        let quote = self.advance().unwrap(); // consume opening quote
        let mut value = String::new();

        loop {
            match self.peek() {
                None => return Err(LexerError::UnterminatedString(line, column)),
                Some(ch) if ch == quote => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.peek() {
                        Some('n') => {
                            self.advance();
                            value.push('\n');
                        }
                        Some('t') => {
                            self.advance();
                            value.push('\t');
                        }
                        Some('\\') => {
                            self.advance();
                            value.push('\\');
                        }
                        Some('"') => {
                            self.advance();
                            value.push('"');
                        }
                        Some('\'') => {
                            self.advance();
                            value.push('\'');
                        }
                        Some(ch) => {
                            self.advance();
                            value.push(ch);
                        }
                        None => return Err(LexerError::UnterminatedString(line, column)),
                    }
                }
                Some(ch) => {
                    self.advance();
                    value.push(ch);
                }
            }
        }

        Ok(TokenKind::String(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(input: &str) -> Vec<TokenKind> {
        Lexer::new(input)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn test_keywords() {
        assert_eq!(
            tokenize("CREATE MATCH WHERE RETURN DELETE SET"),
            vec![
                TokenKind::Create,
                TokenKind::Match,
                TokenKind::Where,
                TokenKind::Return,
                TokenKind::Delete,
                TokenKind::Set,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_keywords_case_insensitive() {
        assert_eq!(
            tokenize("create Match WHERE"),
            vec![
                TokenKind::Create,
                TokenKind::Match,
                TokenKind::Where,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_identifiers() {
        assert_eq!(
            tokenize("person Person _var var123"),
            vec![
                TokenKind::Ident("person".to_string()),
                TokenKind::Ident("Person".to_string()),
                TokenKind::Ident("_var".to_string()),
                TokenKind::Ident("var123".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_numbers() {
        assert_eq!(
            tokenize("42 3.14 0 100.0"),
            vec![
                TokenKind::Int(42),
                TokenKind::Float(3.14),
                TokenKind::Int(0),
                TokenKind::Float(100.0),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_strings() {
        assert_eq!(
            tokenize(r#""hello" 'world' "with \"escape""#),
            vec![
                TokenKind::String("hello".to_string()),
                TokenKind::String("world".to_string()),
                TokenKind::String("with \"escape".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_operators() {
        assert_eq!(
            tokenize("= <> < > <= >= + - * /"),
            vec![
                TokenKind::Eq,
                TokenKind::Neq,
                TokenKind::Lt,
                TokenKind::Gt,
                TokenKind::Lte,
                TokenKind::Gte,
                TokenKind::Plus,
                TokenKind::Dash,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_arrows() {
        assert_eq!(
            tokenize("-> <- -"),
            vec![
                TokenKind::Arrow,
                TokenKind::ArrowLeft,
                TokenKind::Dash,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_brackets() {
        assert_eq!(
            tokenize("()[]{}"),
            vec![
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_cypher_pattern() {
        assert_eq!(
            tokenize("(n:Person)-[:KNOWS]->(m)"),
            vec![
                TokenKind::LParen,
                TokenKind::Ident("n".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("Person".to_string()),
                TokenKind::RParen,
                TokenKind::Dash,
                TokenKind::LBracket,
                TokenKind::Colon,
                TokenKind::Ident("KNOWS".to_string()),
                TokenKind::RBracket,
                TokenKind::Arrow,
                TokenKind::LParen,
                TokenKind::Ident("m".to_string()),
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_create_statement() {
        assert_eq!(
            tokenize("CREATE (n:Person {name: \"Alice\", age: 30})"),
            vec![
                TokenKind::Create,
                TokenKind::LParen,
                TokenKind::Ident("n".to_string()),
                TokenKind::Colon,
                TokenKind::Ident("Person".to_string()),
                TokenKind::LBrace,
                TokenKind::Ident("name".to_string()),
                TokenKind::Colon,
                TokenKind::String("Alice".to_string()),
                TokenKind::Comma,
                TokenKind::Ident("age".to_string()),
                TokenKind::Colon,
                TokenKind::Int(30),
                TokenKind::RBrace,
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_boolean_and_null() {
        assert_eq!(
            tokenize("true false null TRUE FALSE NULL"),
            vec![
                TokenKind::True,
                TokenKind::False,
                TokenKind::Null,
                TokenKind::True,
                TokenKind::False,
                TokenKind::Null,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_unexpected_char() {
        let result = Lexer::new("@").tokenize();
        assert!(matches!(result, Err(LexerError::UnexpectedChar('@', 1, 1))));
    }

    #[test]
    fn test_unterminated_string() {
        let result = Lexer::new("\"hello").tokenize();
        assert!(matches!(result, Err(LexerError::UnterminatedString(1, 1))));
    }

    #[test]
    fn test_variable_length_path_tokens() {
        // Test that 2..5 is tokenized as Int(2), Dot, Dot, Int(5)
        // and not as Float(2.0), Dot, Int(5)
        assert_eq!(
            tokenize("2..5"),
            vec![
                TokenKind::Int(2),
                TokenKind::Dot,
                TokenKind::Dot,
                TokenKind::Int(5),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_variable_length_edge_tokens() {
        assert_eq!(
            tokenize("[:KNOWS*2..5]"),
            vec![
                TokenKind::LBracket,
                TokenKind::Colon,
                TokenKind::Ident("KNOWS".to_string()),
                TokenKind::Star,
                TokenKind::Int(2),
                TokenKind::Dot,
                TokenKind::Dot,
                TokenKind::Int(5),
                TokenKind::RBracket,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_regex_match_token() {
        assert_eq!(
            tokenize("=~"),
            vec![TokenKind::RegexMatch, TokenKind::Eof]
        );
    }

    #[test]
    fn test_regex_match_in_context() {
        assert_eq!(
            tokenize(r#"n.name =~ "A.*""#),
            vec![
                TokenKind::Ident("n".to_string()),
                TokenKind::Dot,
                TokenKind::Ident("name".to_string()),
                TokenKind::RegexMatch,
                TokenKind::String("A.*".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_eq_still_works() {
        // Ensure = still works after adding =~
        assert_eq!(
            tokenize("= =~"),
            vec![TokenKind::Eq, TokenKind::RegexMatch, TokenKind::Eof]
        );
    }

    #[test]
    fn test_union_keywords() {
        assert_eq!(
            tokenize("UNION ALL"),
            vec![TokenKind::Union, TokenKind::All, TokenKind::Eof]
        );
    }

    #[test]
    fn test_union_keywords_case_insensitive() {
        assert_eq!(
            tokenize("union all"),
            vec![TokenKind::Union, TokenKind::All, TokenKind::Eof]
        );
    }
}
