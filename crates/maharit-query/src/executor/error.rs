//! クエリ実行のエラー型 `ExecuteError`。

use thiserror::Error;

use super::*;

/// 実行エラー
#[derive(Debug, Clone, Error, PartialEq)]
pub enum ExecuteError {
    #[error("undefined variable: {0}")]
    UndefinedVariable(String),

    #[error("type error: {0}")]
    TypeError(String),

    #[error("graph error: {0}")]
    GraphError(#[from] maharit_core::GraphError),

    #[error("constraint error: {0}")]
    ConstraintError(#[from] ConstraintError),

    #[error("fulltext error: {0}")]
    FulltextError(#[from] FulltextError),

    #[error("parse error: {0}")]
    ParseError(#[from] crate::parser::ParseError),

    /// ACL で拒否された書き込み（サーバーが文の変更を取り消す）
    #[error("access denied: {0}")]
    AccessDenied(String),
}
