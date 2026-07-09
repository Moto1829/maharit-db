# bug/88: auth_test.py の失敗（既存）— ユーザー管理SQLの文法不一致と server context 未配線

## 概要
`python3 scripts/auth_test.py` を標準の `maharit-db-server`（認証無効）に対して実行すると
9 件失敗する。調査の結果、**今回の性能/セキュリティ改修とは無関係の既存問題**である
（`parser.rs` / `repl.rs` は本セッションのどのコミットでも未変更）。

## 失敗の内訳と原因
1. **文法不一致（パーサー vs テスト）**
   - テスト送信: `CREATE USER x SET PASSWORD 'p' SET ROLE 'reader'`
   - パーサー実装 (`parser.rs::parse_create_user`): `CREATE USER x SET PASSWORD 'p' ROLE <ident>`
     - 2 つ目の `SET` は受け付けない → `expected ROLE, found SET`
     - role は識別子（`reader`）を期待、テストは文字列（`'reader'`）→ `expected identifier`
   - `ALTER USER` も同様（`SET ROLE 'writer'`）。
2. **SHOW USERS が server context を要求**
   - TCP クエリ経路の Executor は `AuthManager` を持たないため
     `SHOW USERS requires server context` を返す（ユーザー一覧を返せない）。
3. **DROP USER / ALTER USER の存在チェックなし**
   - 存在しないユーザーへの操作が成功扱いになる（Executor に AuthManager が無く検証不能）。
4. **前提**: auth_test.py の docstring 自体が「認証有効サーバーを想定」と明記。
   標準コンテナは認証無効で起動しているため、そもそも対象環境が異なる。

## あるべき対応（要判断）
- テスト側をパーサー文法（`ROLE <ident>`, `SET` 1 個）に合わせる、または
- パーサー/実行側をテストの文法（`SET ROLE 'string'`）に合わせる。
- ユーザー管理 SQL（CREATE/DROP/ALTER/SHOW USER）を TCP サーバー経路で `AuthManager` に
  ルーティングする（現状は Executor 止まりで placeholder を返す）。
- どちらを正とするか（文法・機能仕様）を決めてから修正する。

## 他の e2e 結果（同セッションで実行、すべて通過）
- smoke_test 32 / query_feature_test 63 / concurrent_test 19 /
  constraint_test 26（プロパティ索引の作成/検索/削除を含む）/ persistence_test 17。
- replication_test / failover_test は 3 ノードクラスター起動（`start_replication_local.sh`）が
  必要なため未実行。

## ステータス
未対応（原因切り分け済み・既存問題として記録）
