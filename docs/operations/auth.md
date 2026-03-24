---
title: 認証・ロール管理
parent: サーバー・運用
nav_order: 3
---

# 認証・ロール管理

MaharitDB はロールベースアクセス制御（RBAC）を実装しており、ユーザーの作成・管理とロールへの権限付与が可能です。

## ユーザー管理

### CREATE USER

新しいユーザーを作成します。

```cypher
-- 基本的なユーザー作成
CREATE USER alice WITH PASSWORD "securepassword"

-- ロールを指定してユーザーを作成
CREATE USER bob WITH PASSWORD "password123" ROLE reader

-- 管理者ユーザーを作成
CREATE USER admin_user WITH PASSWORD "adminpass" ROLE admin
```

パスワードは内部でハッシュ化（bcrypt または Argon2）されて保存されます。平文パスワードは保持されません。

### ALTER USER

既存のユーザーのパスワードやロールを変更します。

```cypher
-- パスワードを変更
ALTER USER alice SET PASSWORD "newpassword"

-- ロールを変更
ALTER USER alice SET ROLE writer

-- 複数の変更を同時に行う
ALTER USER alice SET PASSWORD "newpass" SET ROLE admin
```

### DROP USER

ユーザーを削除します。

```cypher
DROP USER alice
```

削除されたユーザーのセッションは次のリクエスト時に無効化されます。

### SHOW USERS

登録されているユーザーの一覧を表示します。

```cypher
SHOW USERS
```

出力例：

```
+------------+--------+------------------+
| username   | role   | created_at       |
+------------+--------+------------------+
| alice      | reader | 2024-01-01       |
| bob        | writer | 2024-01-02       |
| admin_user | admin  | 2024-01-01       |
+------------+--------+------------------+
```

## ロール（RBAC）

### ビルトインロール

| ロール | 説明 | 権限 |
|--------|------|------|
| `admin` | 管理者 | すべての操作（ユーザー管理含む） |
| `writer` | 書き込みユーザー | クエリ実行、データ書き込み |
| `reader` | 読み取りユーザー | MATCH と RETURN のみ |

### ロールの権限詳細

**admin ロール**:
- すべてのクエリ操作
- ユーザーの作成・変更・削除
- インデックス・制約の管理
- バックアップ・リストア
- サーバー設定の変更

**writer ロール**:
- MATCH、CREATE、SET、DELETE、MERGE、REMOVE
- UNWIND、WITH、UNION
- インデックスの作成（制約の管理は不可）

**reader ロール**:
- MATCH と RETURN のみ
- WHERE によるフィルタリング
- ORDER BY、LIMIT、SKIP

## 認証の設定

### サーバー起動時の認証有効化

```bash
maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --enable-auth \
  --default-admin-password "initialadminpass"
```

初回起動時に `admin` ユーザーが自動的に作成されます。

> **セキュリティ上の重要事項**: 起動直後に `ALTER USER admin SET PASSWORD "新しいパスワード"` コマンドでパスワードを変更してください。

### クライアントからの認証

```rust
use maharit_client::ClientBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClientBuilder::new("localhost:7687")
        .with_auth("alice", "securepassword")
        .build()
        .await?;

    let result = client.query("MATCH (n:Person) RETURN n.name").await?;
    Ok(())
}
```

## セッション管理

- ログイン成功後、セッショントークンが発行されます
- セッションは一定時間（デフォルト 24 時間）後に自動的に失効します
- `LOGOUT` コマンドで明示的にセッションを終了できます

```cypher
LOGOUT
```

## パスワードポリシー

デフォルトのパスワードポリシー：
- 最低 8 文字以上
- 大文字・小文字・数字を含むことを推奨

## 監査ログ

認証イベントは構造化ログとして記録されます。

```json
{"level":"INFO","event":"user_login","username":"alice","peer":"192.168.1.100","success":true,"timestamp":"2024-01-01T10:00:00Z"}
{"level":"WARN","event":"user_login","username":"alice","peer":"192.168.1.100","success":false,"reason":"invalid_password","timestamp":"2024-01-01T10:01:00Z"}
{"level":"INFO","event":"user_created","username":"carol","created_by":"admin","role":"reader","timestamp":"2024-01-01T10:05:00Z"}
```
