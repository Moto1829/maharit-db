# Task 106: maharit-viz Web UI に認証を導入する

## 背景・目的

現状の maharit-viz は **認証なし**で誰でも `/api/query` を叩ける状態。
本番想定や複数ユーザー環境では危険。

`maharit-server` には既に完全な認証基盤がある:

- `crates/maharit-server/src/auth.rs`
  - `User` / `Session` / `Role::{Admin, ReadWrite, ReadOnly}`
  - `authenticate(username, password) -> Result<sessionToken, AuthError>`
  - `Operation::{Read, Write, Admin}` で操作ごとに権限チェック

しかし TCP プロトコル (`tcp_server.rs::Request`) には認証メッセージが無い:

```
現状サポートする Request:
  query / streamQuery / ping / stats / disconnect /
  begin / commit / rollback
```

**つまり、認証情報を server に渡す経路が現状存在しない**。
これが本タスクの最大の制約で、Phase 1 で先に解決する。

## 全体方針（4 フェーズ）

依存関係: Phase 1 → Phase 2 → Phase 3。Phase 4 はいつでも実施可能。

### Phase 1: TCP プロトコルに認証を追加（server 側）

**目的**: クライアントから username/password で login し、以降のリクエストを
session token で認証できるようにする。

#### 推奨案: A + B のハイブリッド

- **A**. `Request::Login { username, password }` と
  `Response::LoggedIn { sessionToken, role, expiresAt }` を追加
- **B**. 既存の `Query` / `StreamQuery` / `BeginTransaction` 等に
  `sessionToken: Option<String>` フィールドを追加（後方互換のため Optional）
- サーバー設定で「認証必須モード」を ON/OFF できるようにする
  （`ServerConfig::require_auth: bool` を新設、デフォルトは現状互換で `false`）
- `require_auth = true` のとき:
  - `sessionToken` 無し or 無効なリクエストは `Response::AuthError { message }` を返す
  - `Login` 以外の操作はすべて認証必須

#### 既存 SessionManager の活用

`SessionManager` が既に存在するので、TCP セッションごとに `Option<sessionToken>` を
保持する形にする（現状の `ConnectionState` に追加）。

#### 認証エラー型

- `Response::AuthError` を `Response::Error` と並列で追加
- 既存クライアント (maharit-client) も対応が必要

#### 後方互換

- `require_auth = false` のサーバーは現状通り無認証で動く
- 既存テストはそのまま PASS する想定

### Phase 2: maharit-viz 側のログイン・セッション管理

#### バックエンド (`maharit-viz`)

- ログインエンドポイント:
  - `POST /api/login { username, password }` → server に `Login` 投げて
    `sessionToken` を取得 → **HttpOnly Cookie** にセット
  - `POST /api/logout` → Cookie を消す（必要なら server に通知）
- 認証 middleware:
  - `/api/query` 等の API は Cookie の `sessionToken` を取り出して
    server リクエストに付与
  - Cookie がなければ `401 Unauthorized`

#### Cookie 設計

- 名前: `maharit_viz_session`
- 属性: `HttpOnly; SameSite=Lax; Path=/`（HTTPS 環境では `Secure` も）
- 有効期限: server 側 session の expiresAt と一致させる

XSS 対策のため **localStorage には保存しない**。

#### フロントエンド (`assets/`)

- `index.html` を 2 画面構成に:
  - `/login` 相当のログインフォーム
  - `/` (既存) のクエリ UI（未認証なら `/login` にリダイレクト）
- セッション切れの 401 を検知したらログイン画面に戻す
- ヘッダに「現在のユーザー名」と「ログアウト」ボタンを表示

実装方式の選択肢:

- A. SPA 内で表示切替（`index.html` 1 ページ + JS で出し分け、ルーティングなし）
- B. 別 HTML（`login.html` を新規作成、axum でルーティング）
- → **A を推奨**（既存 ES Modules 構造との親和性が高い）

### Phase 3: RBAC を UI に反映

ログイン後のレスポンスに含まれる `role` (Admin / ReadWrite / ReadOnly) を
受け取って UI で活用:

- ヘッダにロールバッジを表示
- ReadOnly のとき:
  - クエリエディタの placeholder に「読み取り専用クエリのみ」表示
  - CREATE/MERGE/DELETE/SET/REMOVE 検出時に Run ボタン下に警告を出す
    （送信は止めない。実エラーは server 側で返ってくる）
- Admin のときだけ「ユーザー管理」リンクを表示（将来用）

### Phase 4: TLS 必須化

認証情報を平文 HTTP で送るのは危険。Phase 2 を本番運用する場合、
以下のいずれかが必須:

#### 案 1: axum 自体に TLS サポートを追加

- `axum-server` の `bind_rustls` を使う
- `--tls-cert`, `--tls-key` CLI オプションを追加
- メリット: 余計なコンポーネント不要
- デメリット: 証明書管理を viz が抱える

#### 案 2: リバースプロキシ前提

- viz は HTTP のまま、`docker-compose.yml` に caddy/nginx を追加
- メリット: TLS 設定の柔軟性、Let's Encrypt 自動化が楽
- デメリット: コンポーネント増加

→ **デフォルトは案 2、案 1 はオプション**で提供。
ドキュメントに本番運用時の構成例を載せる（`docs/operations/`）。

## UX フロー

```
未認証 → ログイン画面表示
       → POST /api/login (username, password)
       → 認証成功
       → HttpOnly Cookie に session token
       → クエリ画面表示

クエリ実行 → POST /api/query (Cookie 付き)
            → viz が Cookie の token を取り出して
              server リクエストに sessionToken として付与
            → 結果を表示

セッション切れ → 401 検知
              → ログイン画面に戻る

ログアウト → POST /api/logout
          → Cookie 削除
          → ログイン画面表示
```

## スコープ外（将来検討）

- OAuth / OIDC（Google / GitHub ログイン）
- LDAP / Active Directory 連携
- MFA（TOTP / WebAuthn）
- セッション同時数制限
- 監査ログの UI 表示

## 推奨実施順

| Phase | 規模 | 優先度 | コミット粒度 |
|-------|------|--------|-------------|
| Phase 1 | 中（server 拡張） | HIGH（他フェーズの前提） | 1 PR |
| Phase 2 | 中（viz 拡張） | HIGH | 1 PR |
| Phase 3 | 小（UI のみ） | MEDIUM | 1 PR |
| Phase 4 | 中（インフラ） | MEDIUM（本番化前に必須） | 1 PR |

## 関連ファイル

- `crates/maharit-server/src/auth.rs` (既存の認証基盤)
- `crates/maharit-server/src/tcp_server.rs` (Request enum 拡張)
- `crates/maharit-client/src/lib.rs` (Login / sessionToken 対応)
- `crates/maharit-viz/src/web.rs` (login/logout 追加, Cookie 中継)
- `crates/maharit-viz/assets/` (ログイン画面追加, role 表示)
- `Dockerfile.viz` / `docker-compose.yml` (Phase 4 TLS 構成)
- `docs/operations/` (TLS / 認証の運用ガイド)

## 優先度

MEDIUM（本番利用前に必須、開発専用なら後回し可）
