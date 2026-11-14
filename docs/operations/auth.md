---
title: 認証・ロール管理
parent: サーバー・運用
nav_order: 3
---

# 認証・ロール管理

MaharitDB のサーバーは、ユーザー名とパスワードによるログインと、3 つのロールによるアクセス制御（RBAC）を備えています。

## 認証を有効にする

認証は既定で**無効**です。無効のサーバーは、ログインなしですべてのリクエストを受け付けます（起動時に警告を出します）。ネットワークに公開するときは必ず有効にしてください。

```bash
maharit server --host 0.0.0.0 --port 7687 \
  --require-auth --admin-password "$MAHARIT_ADMIN_PASSWORD"
```

- `--require-auth`: すべてのリクエストにセッショントークンを要求し、ロールの権限を検査する
- `--admin-password <PW>`（環境変数 `MAHARIT_ADMIN_PASSWORD`）: 起動時に作る `admin` ユーザーのパスワード。`--require-auth` のときは必須で、指定が無いと起動しない

## ユーザーを保存する（`--users-file`）

`--users-file <PATH>`（環境変数 `MAHARIT_USERS_FILE`）を付けると、ユーザーをそのファイルに保存し、再起動後も使えます。指定しないとユーザーはメモリ上にだけあり、再起動すると `admin` だけに戻ります。

```bash
maharit server --require-auth --users-file /var/lib/maharit/users.json \
  --admin-password "$MAHARIT_ADMIN_PASSWORD"
```

- ファイルが**無い**とき: `--admin-password` の `admin` だけを持つファイルを作ります（パスワードが無いと起動しません）
- ファイルが**ある**とき: ユーザーをファイルから読み込みます。`--admin-password` は使いません（指定されていればその旨を表示します）。`admin` のパスワードを変えるには `ALTER USER admin SET PASSWORD '…'` を使います。有効な `admin` ロールのユーザーがいないファイルや、読めないファイルなら起動しません
- `CREATE USER` / `ALTER USER` / `DROP USER` のたびにファイルを書き直します（一時ファイルに書いてから置き換えます）
- ファイルにはユーザー名・ロール・ソルト・パスワードのハッシュを JSON で保存します。平文のパスワードは保存しません。Unix では所有者だけが読み書きできる権限（0600）で作ります
- `--require-auth` なしで指定すると起動しません
- ユーザーはデータファイルにもバックアップにもレプリケーションにも含まれません。ファイルは別にバックアップし、レプリケーションの各ノードやシャードには必要に応じて同じファイルを置いてください

## ログインとセッション

クライアントは `login` リクエストでログインし、返ってきたセッショントークンを以降のリクエストの `sessionToken` に付けます（[プロトコル](../api/protocol.md)）。

```json
{"type": "login", "username": "alice", "password": "s3cret-pass"}
```

```json
{"type": "loggedIn", "sessionToken": "9f1c…", "role": "read_only", "expiresAt": 1767225600}
```

- トークンは 256 ビットの乱数です
- セッションは、最後のリクエストから **30 分**使われないと失効します（使うたびに延長されます）。失効すると `authError` が返るので、ログインし直してください
- ログアウトのリクエストはありません。接続を閉じてもセッションは失効まで残ります

Rust クライアントでは `login` を呼ぶと、以降のクエリにトークンが自動で付きます。

```rust
use maharit_client::Client;

let mut client = Client::connect("127.0.0.1:7687").await?;
client.login("alice", "s3cret-pass").await?;
let result = client.query("MATCH (n:Person) RETURN n.name").await?;
```

Python クライアントには、まだログインの API がありません。

## ロール

| ロール | 別名 | できること |
|---|---|---|
| `reader` | `read_only` | 読み取りクエリ（`MATCH … RETURN`、`SHOW CONSTRAINTS` / `SHOW INDEXES`、プロシージャ呼び出しなど） |
| `writer` | `read_write` | 読み取りに加えて、書き込み（`CREATE` / `MERGE` / `SET` / `DELETE` / `REMOVE` など）とインデックス・制約の作成・削除 |
| `admin` | | すべての操作。ユーザー管理（`CREATE USER` など、`SHOW USERS` を含む）は admin だけ |

書き込みかどうかはクエリの文の種類で決まります。権限が無いと `authError`（`permission denied: …`）が返ります。

ロールより細かい、ラベルやプロパティ単位の制御はまだありません（[アクセス制御（ACL）](acl.md)）。

## ユーザー管理

ユーザー管理の文は、認証を有効にしたサーバーで admin としてログインして実行します。ロールは文字列でも識別子でも書けます。

### CREATE USER

```cypher
CREATE USER alice SET PASSWORD 's3cret-pass' SET ROLE reader
```

`SET ROLE` の `SET` は省略できます（`CREATE USER bob SET PASSWORD 'pw' ROLE writer`）。ロールの指定は必須です。

### ALTER USER

```cypher
ALTER USER alice SET PASSWORD 'new-pass'
```

```cypher
ALTER USER alice SET ROLE writer
```

```cypher
ALTER USER alice SET PASSWORD 'new-pass' SET ROLE admin
```

### DROP USER

```cypher
DROP USER alice
```

### SHOW USERS

```cypher
SHOW USERS
```

各行は `username`・`role`（`admin` / `read_write` / `read_only`）・`active` を持ちます。

```text
username | role       | active
---------+------------+-------
admin    | admin      | true
alice    | read_only  | true
```

## パスワードの保存

パスワードはユーザーごとのソルトと PBKDF2-HMAC-SHA256（10 万回）でハッシュ化して保持し、平文は保持しません。パスワードの長さや文字種の制限はありません。

## 監査ログ

`--audit-log <PATH>`（環境変数 `MAHARIT_AUDIT_LOG`）を付けると、次の出来事を 1 行 1 件の JSON でファイルに追記します。書き込めないパスなら起動しません。

| `event` | 記録する場面 | 項目 |
|---|---|---|
| `login_success` / `login_failure` | ログインの成功・失敗 | `username` |
| `user_created` / `user_modified` / `user_deleted` | `CREATE USER` / `ALTER USER` / `DROP USER` の成功 | `username`（対象）、`by`（実行したユーザー） |
| `permission_denied` | ロールの権限が足りずに拒否したとき | `username`、`action`（`write` / `manage_users` / `promote` / `begin_write_transaction`） |

```text
{"timestamp":1763100000,"event":"login_success","username":"admin"}
{"timestamp":1763100005,"event":"user_created","username":"alice","by":"admin"}
{"timestamp":1763100010,"event":"permission_denied","username":"alice","action":"write"}
```

- `timestamp` は Unix 時刻（秒）。パスワードは記録しません
- ファイルが 10 MiB を超えると `<PATH>.1` 〜 `<PATH>.3` に回して新しいファイルに書きます
- 認証が無効なサーバーでも使えます（ユーザー名は `anonymous` になります）
- コーディネーター（`--coordinator`）では記録しません。各シャードで指定してください
