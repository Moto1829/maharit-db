# Task 103: parser のドットアクセス読み取りパターンを共通ヘルパー化

## 背景

Task 98 の修正で、`crates/maharit-query/src/parser.rs` にはドットアクセスで
プロパティ名を読むパターンが **8 箇所** に同じ形で散らばっている:

```rust
if self.check(TokenKind::Dot) {
    self.advance();
    let prop = self.expect_ident_or_keyword()?;
    /* var, prop を使って AST 構築 */
}
```

出現箇所:

- `parse_set_item`: SET n.prop = value
- `parse_order_by_item`: ORDER BY n.prop
- `parse_return_item`: RETURN n.prop
- `parse_remove_item`: REMOVE n.prop
- `parse_constraint` (複合): REQUIRE n.prop1, n.prop2 IS ...
- `parse_constraint` (単一): REQUIRE n.prop IS ...
- `parse_create_fulltext_index`: FOR (n.prop1, n.prop2)
- `parse_primary` (Expression::Property): expression 内の n.prop

将来 `n.prop1.prop2`（ネストアクセス）や `n[\"prop\"]`（IndexAccess の追加形式）
に対応する場合、これら全箇所を修正することになる。

## 提案

以下のような共通ヘルパーを追加して、各箇所から呼ぶようにする:

```rust
/// `.prop` 形式のプロパティ名を読み取る。呼び出し前に変数名 (var) は読み終わっている前提。
/// Dot が無い場合は `Ok(None)` を返し、AST 上は単純変数として扱う。
fn try_consume_dot_property(&mut self) -> Result<Option<String>, ParseError> {
    if self.check(TokenKind::Dot) {
        self.advance();
        Ok(Some(self.expect_ident_or_keyword()?))
    } else {
        Ok(None)
    }
}

/// `n.prop` の dot とプロパティ名を必ず期待する版（Dot がなければエラー）
fn expect_dot_property(&mut self) -> Result<String, ParseError> {
    self.expect(TokenKind::Dot)?;
    self.expect_ident_or_keyword()
}
```

### Before / After

```rust
// Before
if self.check(TokenKind::Dot) {
    self.advance();
    let prop = self.expect_ident_or_keyword()?;
    Ok(ReturnItem::Property(var, prop))
} else {
    Ok(ReturnItem::Variable(var))
}

// After
match self.try_consume_dot_property()? {
    Some(prop) => Ok(ReturnItem::Property(var, prop)),
    None => Ok(ReturnItem::Variable(var)),
}
```

## 検証

- `cargo test -p maharit-query` が全件 PASS（487 件）
- 既存の Task 98 追加テスト (`test_parse_property_access_keyword` 等) が通る

## 優先度

LOW（コード重複の削減、将来の拡張が容易になる）

## 関連ファイル

- `crates/maharit-query/src/parser.rs`

## 解決済み (2026-06-14)

### 実装内容

2 つのヘルパーを `Parser` に追加:

```rust
fn try_consume_dot_property(&mut self) -> Result<Option<String>, ParseError> { ... }
fn expect_dot_property(&mut self) -> Result<String, ParseError> { ... }
```

### 置換箇所 (計 7 箇所)

- `try_consume_dot_property` (Dot 任意):
  - `parse_order_by_item` (ORDER BY n.prop)
  - `parse_return_item` (RETURN n.prop)
  - `parse_remove_item` (REMOVE n.prop)
  - `parse_primary` (Expression::Property)
- `expect_dot_property` (Dot 必須):
  - `parse_set_item` (SET n.prop = value)
  - `parse_constraint` 複合 (REQUIRE n.prop1, n.prop2 IS ...)
  - `parse_constraint` 単一 (REQUIRE n.prop IS ...)
  - `parse_create_fulltext_index` (FOR (n.prop1, n.prop2))

### 検証

- `cargo test -p maharit-query` → **487/487 PASS** (リグレッションなし)
- 既存の Task 98 追加テスト (`test_parse_property_access_keyword` 等) も全て PASS
