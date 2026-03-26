# Task 71: バックアップ・リストア時にインデックス定義を保存・復元

## Status: completed

## Overview
`backup.rs` の `serialize_graph` / `deserialize_graph` を拡張し、`PropertyIndex` の定義をバックアップファイルに保存・復元できるようにする。

## Changes

### `crates/maharit-storage/src/backup.rs`

1. **`serialize_graph(graph, indexes)`** — シグネチャ変更。エッジの後に `index_count (u32)` + 各定義 (`label`, `property`, `unique`) を書き込む。
2. **`deserialize_graph_internal(data)`** — 内部関数として追加。`(Graph, Vec<IndexDefinition>)` を返す。エッジ読み込み後に `index_count` を読み、EOF なら空 Vec を返す（旧フォーマット互換）。
3. **`deserialize_graph(data)`** — `deserialize_graph_internal` のラッパー。`Graph` のみを返す（後方互換維持）。
4. **`Backup::create_with_index(graph, property_index, path, options)`** — インデックス定義を含むバックアップを作成する新 public メソッド。
5. **`Backup::restore_with_index(path)`** — インデックス定義を復元し `PropertyIndex` を再構築する新 public メソッド。`(Graph, PropertyIndex)` を返す。
6. **テスト追加**:
   - `test_backup_restore_with_index_definitions` — インデックス定義・データの往復確認
   - `test_restore_old_format_without_index_section` — 旧フォーマット（インデックスなし）の互換性確認

## Test Results
- `cargo test -p maharit-storage`: 74/74 passed
