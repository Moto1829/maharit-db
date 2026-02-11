# 文字列関数

## 概要
Cypherの組み込み文字列関数を実装する。

## 実装内容

### 基本文字列操作
- [ ] trim(string): 前後の空白を除去
- [ ] ltrim(string): 先頭の空白を除去
- [ ] rtrim(string): 末尾の空白を除去
- [ ] toLower(string): 小文字に変換
- [ ] toUpper(string): 大文字に変換
- [ ] reverse(string): 文字列を反転

### 部分文字列操作
- [ ] substring(string, start, length?): 部分文字列の取得
- [ ] left(string, length): 左からN文字取得
- [ ] right(string, length): 右からN文字取得
- [ ] split(string, delimiter): 区切り文字で分割（リスト返却）

### 変換・置換
- [ ] replace(string, search, replace): 文字列置換
- [ ] toString(value): 文字列への変換

### 文字列情報
- [ ] size(string): 文字列の長さ

## クエリ例
```cypher
-- 小文字変換
MATCH (n:Person) RETURN toLower(n.name) AS lower_name

-- 部分文字列
MATCH (n:Person) RETURN substring(n.name, 0, 3) AS prefix

-- 分割
MATCH (n:Person) RETURN split(n.email, '@') AS parts

-- 置換
MATCH (n:Person) RETURN replace(n.name, 'Mr. ', '') AS clean_name

-- トリム
MATCH (n:Person) RETURN trim(n.name) AS trimmed
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
