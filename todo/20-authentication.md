# 認証・認可

## 概要
TCPサーバーへの接続時にユーザー認証と権限管理を行う機能を実装する。

## 実装内容

### ユーザー管理
- [ ] ユーザーの作成・削除・更新
- [ ] パスワードのハッシュ化（argon2/bcrypt）
- [ ] ユーザー情報の永続化

### 認証
- [ ] 接続時のユーザー名/パスワード認証
- [ ] セッション管理
- [ ] 認証トークン（JWT等）

### 認可（ロールベースアクセス制御）
- [ ] ロールの定義（admin, read-write, read-only）
- [ ] クエリ種別ごとの権限チェック
- [ ] ラベル/プロパティ単位のアクセス制御（将来的）

### 監査ログ
- [ ] 認証イベントの記録
- [ ] クエリ実行の記録
- [ ] ログのローテーション

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
