# Task 20: 認証・認可

## 概要
TCPサーバーへの接続時にユーザー認証と権限管理を行う機能を実装する。

## 実装内容

### ユーザー管理
- [x] ユーザーの作成・削除・更新
- [x] パスワードのハッシュ化（FNV-1a + salt）
- [x] ユーザー情報の永続化

### 認証
- [x] 接続時のユーザー名/パスワード認証
- [x] セッション管理
- [x] 認証トークン（セッションベース）

### 認可（ロールベースアクセス制御）
- [x] ロールの定義（admin, read-write, read-only）
- [x] クエリ種別ごとの権限チェック
- [x] ラベル/プロパティ単位のアクセス制御（AclManager: AclRule/AclSubject/AclResource/AclPermission）

### クエリ構文
- [x] CREATE USER username SET PASSWORD 'pass' ROLE role
- [x] DROP USER username
- [x] ALTER USER username SET PASSWORD/ROLE
- [x] SHOW USERS

### 監査ログ
- [x] 認証イベントの記録
- [x] クエリ実行の記録
- [x] ログのローテーション

## クエリ例
```cypher
-- ユーザー作成（管理者のみ）
CREATE USER alice SET PASSWORD 'secret' ROLE read-write

-- ロール変更
ALTER USER alice SET ROLE admin

-- ユーザー削除
DROP USER alice
```

## 依存
- `12-tcp-server.md` が完了していること

## 対象クレート
`maharit-server`
