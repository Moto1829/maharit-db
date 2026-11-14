# タスク 173: ラベル・プロパティ単位のアクセス制御（ACL）をサーバーに組み込む

server/158 から分けた。ユーザーの永続化（`--users-file`）と監査ログ（`--audit-log`）は 158 で実装済み。
ACL は意味の決め方で結果が変わるので、まず下の設計案を確定してから実装する。

## 現状（2026-11-14）
- `auth.rs` に `AclManager` / `AclRule`（主体: ユーザー or ロール、対象: ラベル / ラベル+プロパティ / 全ラベル、Allow / Deny）と
  評価順（ユーザー Deny > ロール Deny > ユーザー Allow > ロール Allow > 既定 Allow）があるが、どこからも使われていない
- docs/operations/acl.md は「未実装」と書いてある

## 設計案

### 文法（Cypher 風。ユーザー管理文と同じく admin だけが実行でき、サーバー側で処理する）
```text
GRANT  READ|WRITE ON :Label            TO USER alice | TO ROLE reader
GRANT  READ|WRITE ON :Label(prop)      TO …
DENY   READ|WRITE ON :Label[(prop)]    TO …
REVOKE READ|WRITE ON :Label[(prop)]    FROM …     // 一致する GRANT / DENY を取り除く
SHOW ACL                                          // 規則の一覧（subject / resource / action / permission）
```
- `*` を全ラベルとして許す（`ON :*`）。`READ` と `WRITE` を分けるため `AclRule` に `action: Read | Write` を追加する
- パース: maharit-query の `Statement` に `Grant` / `Deny` / `Revoke` / `ShowAcl` を追加し、エンジンでは CREATE USER と同じく
  何もしない（サーバーの `try_handle_user_management` 相当で処理）。REPL・埋め込み利用では「サーバーでのみ有効」とエラーにする

### 保存先
- `--users-file` と同じファイルに `acl: [...]` として保存する（`PersistedUsers` を version 3 に上げ、v2 は ACL 空として読む）
- ユーザーを DROP したら、そのユーザーの規則も消す
- `--users-file` なしではメモリ上だけ（ユーザーと同じ扱い）

### 適用箇所と意味（ここが要決定）
- 読み取り（READ の Deny）:
  - ノード: MATCH の候補から外す（パターンにも一致しない＝見えない）。結果の行から黙って消えるので、件数や集計にも効く
  - プロパティ: 値を `null` として見せる（キーも `keys()` / `properties()` から外す）。WHERE での参照も null 扱い
  - リレーションシップ: 両端のどちらかが見えなければ見えない（案。型単位の規則は後回し）
- 書き込み（WRITE の Deny）: 対象ラベルのノードの CREATE / SET / REMOVE / DELETE / MERGE、対象プロパティの SET / REMOVE をエラーにする
  （行ごとに黙って飛ばすのではなく文全体を失敗させ、bug/160 のロールバックで元に戻す）
- 実装場所: Executor に「可視性フィルタ」（`Fn(NodeId) -> bool` とプロパティマスク）を渡せるようにし、`match_node_pattern` /
  プロパティ参照 / 結果の組み立てで適用する。サーバーはセッションのユーザーとロールから `AclManager` で作って渡す
- インデックス・全文検索・`count(*)` の高速パス（`try_simple_count`）・プロシージャ（`db.labels` など）も規則を守る必要がある
  （高速パスは ACL 有効時は使わない）

### RBAC との関係
- まずロールで判定（reader は書き込み不可、など）し、通った後に ACL で絞る。ACL はロールの権限を広げない（GRANT WRITE を reader に付けても書けない）
- 規則が 1 つも無ければ現状と同じ動作（既定 Allow）。admin ロールには ACL を適用しない（締め出し防止）
- 監査ログ: ACL で拒否した書き込みを `permission_denied`（`action: "acl_write"`）として記録。GRANT / DENY / REVOKE も記録する

### 範囲外（当面）
- レプリケーション: ACL はノードごとの設定（users-file と同じ）。フォロワーにも同じファイルを置く
- コーディネーター: シャードで適用されるので、コーディネーターは素通し
- ストリーミングクエリ・トランザクション内のクエリも同じフィルタを通すこと

## 作業
- [ ] 上の「要決定」（読み取り Deny の意味＝見えない / エラー、admin 除外の可否）をユーザーと確定
- [ ] AST・パーサー・サーバー処理・保存（v3）
- [ ] Executor の可視性フィルタとプロパティマスク、高速パスの無効化
- [ ] テスト（ユニット・E2E）、docs/operations/acl.md（日英）を実装に合わせて書き直す
