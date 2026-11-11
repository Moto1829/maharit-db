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

!!! warning "ユーザーはサーバーの再起動で消える"
    ユーザーとパスワードはメモリ上にだけあり、データファイルにもバックアップにも保存されません。再起動すると `admin` だけに戻るので、`CREATE USER` で作ったユーザーは起動後に作り直してください。

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

ログインやユーザー管理の操作を記録する監査ログは、まだサーバーに組み込まれていません。
