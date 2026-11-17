//! parser.rs のテスト。分野ごとのファイルに分け、共通のヘルパーはここに置く。

use super::*;

mod keywords_subquery;
mod read_clauses;
mod write_clauses;

fn parse(input: &str) -> Result<Statement, ParseError> {
    Parser::new(input)?.parse()
}
