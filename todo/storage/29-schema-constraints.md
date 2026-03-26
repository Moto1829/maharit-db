# Task 29: スキーマ制約

## 概要
データの整合性を保証するためのスキーマ制約機能を実装する。

## 実装内容

### ユニーク制約
- [x] プロパティのユニーク制約
- [x] 複合ユニーク制約
- [x] 制約違反時のエラー

### 存在制約（NOT NULL）
- [x] 必須プロパティの定義
- [x] ノード作成時の検証
- [x] プロパティ削除の防止

### 型制約
- [x] プロパティの型指定
- [x] 型チェック
- [x] 型変換エラー

### ラベル制約
- [x] ノードに必須のラベル
- [x] エッジの始点/終点ラベル制約

### 制約管理
- [x] 制約の作成
- [x] 制約の削除
- [x] 制約の一覧取得
- [x] 制約のオン/オフ（バルクロード時）

## クエリ例
```cypher
-- ユニーク制約
CREATE CONSTRAINT unique_email FOR (n:User) REQUIRE n.email IS UNIQUE

-- 存在制約
CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL

-- 型制約
CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER

-- エッジ制約
CREATE CONSTRAINT knows_constraint FOR ()-[r:KNOWS]->()
REQUIRE (r.since IS NOT NULL)

-- 制約の確認
SHOW CONSTRAINTS

-- 制約の削除
DROP CONSTRAINT unique_email
```

## 依存
- `07-label-index.md` が完了していること
- `08-property-index.md` が完了していること

## 対象クレート
`maharit-core`, `maharit-query`
