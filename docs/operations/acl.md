---
title: アクセス制御（ACL）
parent: サーバー・運用
nav_order: 4
---

# アクセス制御（ACL）

MaharitDB はラベルおよびプロパティ単位の細粒度アクセス制御（Fine-grained ACL）をサポートしています。RBAC のロールに加えて、より詳細なアクセス制限をユーザーごとに設定できます。

## 概要

細粒度 ACL を使用することで、以下のような制御が可能です。

- 特定のラベルのノードへのアクセスを制限
- 特定のプロパティの読み取り・書き込みを制限
- ユーザーグループごとに異なるデータ可視性を設定

## ラベルベースのアクセス制御

### ラベルへのアクセス権を付与

```cypher
-- ユーザー alice に Person ラベルへの読み取り権限を付与
GRANT READ ON LABEL Person TO alice

-- 書き込み権限を付与
GRANT WRITE ON LABEL Person TO alice

-- すべてのラベルへの読み取り権限
GRANT READ ON ALL LABELS TO alice
```

### ラベルへのアクセス権を剥奪

```cypher
-- 読み取り権限を剥奪
REVOKE READ ON LABEL Person FROM alice

-- 書き込み権限を剥奪
REVOKE WRITE ON LABEL Person FROM alice
```

### アクセス権の確認

```cypher
-- ユーザーの ACL 一覧を表示
SHOW ACL FOR alice

-- 特定ラベルのアクセス権を確認
SHOW ACL ON LABEL Person
```

## プロパティベースのアクセス制御

特定のプロパティへのアクセスを制限できます。

### プロパティへのアクセス権を付与

```cypher
-- Person ノードの name プロパティの読み取りを許可
GRANT READ ON PROPERTY name ON LABEL Person TO alice

-- 機密プロパティ（salary）の書き込みを禁止
DENY WRITE ON PROPERTY salary ON LABEL Employee TO default_user
```

### プロパティを非表示にする

アクセス権のないプロパティは `null` として返されます。

```cypher
-- salary プロパティへのアクセス権がない場合
MATCH (e:Employee {name: "Alice"})
RETURN e.name, e.salary  -- salary は null として返される
```

## ロールベース ACL

個別ユーザーではなくロールに対して ACL を設定できます。

```cypher
-- reader ロールに Product ラベルの読み取りを許可
GRANT READ ON LABEL Product TO ROLE reader

-- 機密ラベルへのアクセスを reader ロールに禁止
DENY READ ON LABEL FinancialRecord TO ROLE reader
```

## ACL ポリシーの優先順位

1. DENY（明示的な拒否）が最優先
2. GRANT（明示的な許可）が次
3. デフォルトはロールの権限に従う

```cypher
-- admin ロールのユーザーでも特定ラベルを拒否できる
DENY READ ON LABEL SecretProject TO specific_user
```

## ACL の例

### 部門ごとにデータを分離する

```cypher
-- 営業部門のユーザー（sales_user）は Customer ラベルのみアクセス可
GRANT READ ON LABEL Customer TO sales_user
GRANT WRITE ON LABEL Customer TO sales_user
DENY READ ON ALL LABELS TO sales_user  -- 他はすべて拒否
GRANT READ ON LABEL Customer TO sales_user  -- Customer は許可（順序に注意）

-- エンジニア（eng_user）は技術的なラベルのみ
GRANT READ ON LABEL Repository TO eng_user
GRANT READ ON LABEL PullRequest TO eng_user
GRANT READ ON LABEL Issue TO eng_user
```

### 機密プロパティの保護

```cypher
-- HR マネージャーのみ給与情報にアクセス可
DENY READ ON PROPERTY salary ON LABEL Employee TO ROLE reader
DENY READ ON PROPERTY salary ON LABEL Employee TO ROLE writer
GRANT READ ON PROPERTY salary ON LABEL Employee TO ROLE hr_manager
```

## デフォルト ACL の設定

新規ユーザー作成時のデフォルト ACL を設定できます。

```cypher
-- デフォルトで Public ラベルのみ読み取り可
SET DEFAULT ACL READ ON LABEL Public
```

## ACL と既存の RBAC の使い分け

| 用途 | 推奨する方法 |
|------|------------|
| 一般的なアクセス制御 | RBAC（reader/writer/admin） |
| データの論理的分離 | ラベルベース ACL |
| 機密情報の保護 | プロパティベース ACL |
| マルチテナント | ラベルベース ACL + RBAC の組み合わせ |
