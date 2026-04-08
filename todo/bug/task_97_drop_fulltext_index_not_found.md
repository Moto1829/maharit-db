# タスク: DROP FULLTEXT INDEX が index not found エラーを返す

## 概要

`CREATE FULLTEXT INDEX` は成功（`result` を返す）するが、その後の `DROP FULLTEXT INDEX` で "index not found" エラーが発生する。制約の問題（task_96）と同様に、フルテキストインデックスが FulltextManager に正しく登録されていないか、Executor/サーバー間での共有に問題がある可能性がある。

## 失敗したテスト
- スクリプト: `scripts/constraint_test.py`
- テスト: `フルテキストインデックス` セクション — "フルテキストインデックス削除"
- エラーメッセージ:
```
フルテキストインデックス削除 — {'type': 'error', 'message': 'Execution error: fulltext error: index not found: ft_article_body'}
```

## 再現クエリ

```cypher
-- インデックス作成（成功する）
CREATE FULLTEXT INDEX ft_article_body FOR (a:Article) ON (a.body)
-- 期待: result
-- 実際: result（成功）

-- テストデータ投入
CREATE (:Article {body: 'Graph databases are powerful'})

-- CONTAINS 検索（インデックスなしでもフルスキャンで動作する）
MATCH (a:Article) WHERE a.body CONTAINS 'graph' RETURN a.title

-- インデックス削除
DROP FULLTEXT INDEX ft_article_body
-- 期待: result
-- 実際: error: fulltext error: index not found: ft_article_body
```

## 根本原因の分析

task_96（制約問題）と同根の可能性が高い。Executor が保持する FulltextManager が TCP サーバーセッション間で共有されていないため、CREATE したインデックスが次のクエリ実行時に消えている。

また、`CONTAINS` 検索が CREATE/DROP に関わらず動作するのは、インデックスなしのフルスキャンにフォールバックしているためと推測される。

## 対応方針

1. `crates/maharit-query/src/executor.rs` の `Statement::CreateFulltextIndex` / `Statement::DropFulltextIndex` ハンドラを確認
2. TCP サーバーが FulltextManager をどのように保持しているかを調査（task_96 と合わせて修正）
3. FulltextManager の永続化（WAL への書き込み・起動時ロード）を確認
4. task_96 が修正されれば同時に解決される可能性が高い

## 優先度
MEDIUM

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — CreateFulltextIndex/DropFulltextIndex の実行処理
- `crates/maharit-server/src/tcp_server.rs` — FulltextManager の保持方法
- `scripts/constraint_test.py` — 失敗テスト（test_fulltext_index）
