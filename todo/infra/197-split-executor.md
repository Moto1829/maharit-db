# タスク 197: executor.rs などの巨大ファイルを分割する

前提: タスク 196（検証コマンド）があると安全に進められる。

## 背景
main 時点の行数:

| ファイル | 全体 | うちテスト |
|---|---|---|
| `crates/maharit-query/src/executor.rs` | 15,171 行 | 約 7,900 行（7,271 行目から） |
| `crates/maharit-query/src/parser.rs` | 5,596 行 | 約 1,730 行（3,865 行目から） |
| `crates/maharit-server/src/tcp_server.rs` | 5,386 行 | 約 2,920 行（2,462 行目から） |

`executor.rs` は 1 つの `impl Executor` に約 480 個の関数が入っている。
AI はファイル全体を読めないので部分読みになり、既存のヘルパーを見落として重複実装したり、離れた場所の不変条件を壊したりしやすい。
編集のたびに読む量が多く、応答も遅くなる。

`executor/algo.rs`（392 行）は既に切り出してあるので、同じ形で続ける。

## 作業内容
挙動は変えない。1 段ごとに 1 コミットにし、毎回 `bash scripts/check.sh` を通す。

- [x] 第 1 段: テストをファイルの外へ出す（`executor/tests/` に分野ごと。`parser.rs` と `tcp_server.rs` も同様）。これだけで本体ファイルが半分になる
- [x] 第 2 段: `executor.rs` の型定義を分ける（`value.rs`: `Value` / `Row` / `ResultSet` と変換、`bindings.rs`: `BindingValue`）
- [x] 第 3 段: `impl Executor` を機能ごとのファイルに分ける。目安:
  - `match_.rs`（パターン照合）
  - `write.rs`（CREATE / SET / DELETE / MERGE / REMOVE / FOREACH）
  - `expr.rs`（式の評価）
  - `functions.rs`（スカラー関数）
  - `aggregate.rs`（集約）
  - `subquery.rs`（CALL / EXISTS / COUNT / COLLECT）
  - `schema.rs`（制約・索引）
- [x] 各ファイルの先頭に、何が入っているかを 2〜3 行の `//!` で書く
- [ ] `parser.rs` も文・式・パターンで分ける
- [ ] `tcp_server.rs` を、接続処理・メッセージの振り分け・クエリ実行に分ける
- [ ] CLAUDE.md の「maharit-query: クエリパイプライン」節を新しい構成に合わせる

## 注意
- タスク 188（slot-based bindings）は executor を広く書き換える。どちらを先にやるか決め、同時に進めない
- 他のセッションやブランチが executor.rs を触っていると衝突が大きい。着手前に開いているブランチを確認する
- 分割で `pub(super)` / `pub(crate)` が増える。公開範囲をクレートの外へ広げない

## 完了条件
- `crates/maharit-query/src/executor/` 配下の各ファイルが 2,000 行以下
- テストの件数が分割前と同じで、全て通る
- 公開 API（`maharit_query` の `pub` 項目）が変わっていない
