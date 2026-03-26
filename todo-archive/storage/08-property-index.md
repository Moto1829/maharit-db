# Task 08: プロパティインデックス

## 概要
プロパティ値による高速な検索を可能にするインデックスを実装する。

## 実装内容

### 完全一致インデックス
- [x] (プロパティ名, 値) -> ノードID集合
- [x] HashMapベースの実装
- [x] インデックス対象プロパティの指定

### 範囲インデックス（BTree）
- [x] 数値プロパティの範囲検索
- [x] BTreeMapベースの実装
- [x] `find_by_int_range(prop, min, max)`
- [x] `find_by_float_range(prop, min, max)`

### インデックス管理
- [x] インデックスの作成: `create_index(IndexDefinition)`
- [x] インデックスの削除: `drop_index(label, property)`
- [x] インデックスの一覧取得: `list_indexes()`

### 自動更新
- [x] プロパティ設定時のインデックス更新
- [x] ノード削除時のインデックス削除

## API
```rust
let mut index = PropertyIndex::new();

// インデックス定義の作成
index.create_index(IndexDefinition::new("Person", "name"));

// プロパティのインデックス
index.index_property(node_id, "name", &PropertyValue::String("Alice".into()));

// 検索
let nodes = index.find_by_property("name", &PropertyValue::String("Alice".into()));
let nodes = index.find_by_int_range("age", 20, 30);
let nodes = index.find_greater_than("age", &PropertyValue::Int(18));
```

## 依存
- `07-label-index.md` が完了していること

## 対象クレート
`maharit-core`
