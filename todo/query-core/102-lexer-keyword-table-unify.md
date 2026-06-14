# Task 102: lexer のキーワード定義の二重メンテを一本化

## 背景

`crates/maharit-query/src/lexer.rs` には現在、同じキーワード一覧を
**3 箇所で別々にメンテ**する状態になっている:

1. `read_ident()` 内の `match ident.to_uppercase()` テーブル
   （文字列 → TokenKind）
2. `impl Display for TokenKind` の match
   （TokenKind → 表示用大文字文字列）
3. `keyword_as_ident()` の match（Task 98 で追加）
   （TokenKind → 識別子位置の小文字名）

新キーワードを追加すると 3 箇所を漏れなく更新する必要があり、漏れがあると:

- 1 を忘れる → 識別子のままになり予約語として機能しない
- 2 を忘れる → エラーメッセージで表示できず panic
- 3 を忘れる → プロパティキー / プロパティアクセスでパースエラー（Task 98 と同根）

## 提案

キーワード定義を **単一テーブル** にまとめ、3 箇所から参照する形に変える。

### 案 A: マクロでテーブル展開

```rust
macro_rules! keywords {
    ($($kw:literal => $variant:ident),* $(,)?) => {
        // (TokenKind enum バリアントは別途定義)

        fn lookup_keyword(s: &str) -> Option<TokenKind> {
            match s.to_uppercase().as_str() {
                $($kw => Some(TokenKind::$variant),)*
                _ => None,
            }
        }

        pub fn keyword_as_ident(kind: &TokenKind) -> Option<&'static str> {
            match kind {
                $(TokenKind::$variant => Some(&const_lower!($kw)),)*
                _ => None,
            }
        }

        // Display も同様に
    }
}

keywords! {
    "CREATE" => Create,
    "MATCH"  => Match,
    // ...
}
```

`const_lower!` は const fn またはマクロで実装。

### 案 B: 静的テーブル + ヘルパー関数

```rust
const KEYWORDS: &[(&str, TokenKind)] = &[
    ("CREATE", TokenKind::Create),
    ("MATCH",  TokenKind::Match),
    // ...
];
```

TokenKind が `Copy` でないと const にしづらいので、初回アクセス時に `OnceLock`
で `HashMap<&str, TokenKind>` と `HashMap<std::mem::Discriminant<TokenKind>, &str>`
を構築するのも選択肢。

案 A が記述量も少なく、コンパイル時に解決されるので推奨。

## 検証

- `cargo test -p maharit-query` が全件 PASS（487 件）
- 既存の Display / keyword_as_ident の挙動が変わらないこと

## 優先度

LOW（保守性向上、現状動作には問題なし）

## 関連ファイル

- `crates/maharit-query/src/lexer.rs`
