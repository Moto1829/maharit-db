# タスク: smoke_test クリーンアップ後に未知ラベルのノードが残留する

## 概要

`scripts/smoke_test.py` の最終チェック「全ノード削除済み」が FAIL する。
テスト実行後に `Persistent` ラベルのノードが 1 件残り、`MATCH (n) RETURN n` が 1 件を返す。

## 失敗したテスト

- スクリプト: `scripts/smoke_test.py`
- テスト項目: `全ノード削除済み`
- エラーメッセージ:
```
✗ 全ノード削除済み — 残り=1
```
- 残存ノード確認:
```json
[
  {
    "n": "(0:Persistent)",
    "labels(n)": "[\"Persistent\"]"
  }
]
```

## 根本原因の分析

`smoke_test.py` の `test_cleanup()` 関数は以下の特定ラベルのみを削除する:
- `TxTest`, `StreamTest`, `Company`, `Person`

これらのラベル以外のノードがデータベースに存在する場合、クリーンアップが不完全になる。
Docker Volume (`maharit-db_maharit-data`) には以前のテスト実行やその他のテストスクリプトが残したデータが
永続化されており、smoke_test.py が使用する Docker コンテナ起動時に引き継がれる。

具体的には `Persistent` ラベルのノードが Docker Volume に残留しており、
smoke_test.py はこのラベルを削除対象に含めていない。

影響ファイル:
- `scripts/smoke_test.py` の `test_cleanup()` 関数 (L278〜L290)
- Docker Volume `maharit-db_maharit-data`

## 対応方針

以下の2つのアプローチが考えられる:

### 案A: テスト冒頭でDBをリセット（推奨）
`main()` 関数の最初または `test_cleanup()` の実装として全ノード削除を追加:
```python
# テスト開始前にDB全体をクリア
run_query(client, "MATCH (n) DETACH DELETE n")
```

### 案B: クリーンアップを全削除に変更
`test_cleanup()` の最後のアサーション前に:
```python
run_query(client, "MATCH (n) DETACH DELETE n")
```
とし、`全ノード削除済み` チェックの前に実行する。

### 案C: テスト終了条件を緩和
smoke_test.py が自分で作成したノードのみを対象にするよう、
削除対象ラベルに `Persistent` 等の未知ラベルを追加しない。
代わりに、「smoke_test が作成したノード数がすべて削除されたか」を検証する。

**推奨は案A**（テスト冪等性の確保）。

## 優先度

MEDIUM

## 状態

完了 (2026-03-30)

## 関連ファイル

- `scripts/smoke_test.py` — `test_cleanup()` (L278〜L290), `main()` (L323〜)
