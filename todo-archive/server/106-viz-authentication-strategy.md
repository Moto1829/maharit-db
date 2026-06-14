# Task 106: maharit-viz Web UI に認証 / TLS をオプションで導入する

## 背景・目的

現状の maharit-viz は **認証なし・HTTP のみ** で動作している。本番想定や
複数ユーザー環境では危険だが、開発時の手軽さは保ちたい。

そこで、**認証と TLS をいずれもオプション機能として実装**し、必要なときだけ
有効化できるようにする。

> **デフォルト動作（破壊的変更なし）**
> - 認証: **無効** (`require_auth=false`)、誰でも `/api/query` を叩ける現状の挙動
> - TLS: **無効**、HTTP のみで listen
> - `cargo run -p maharit-viz` / `docker compose up` の体験は今までと完全に同じ
> - ただし起動時に **WARN ログを出力** して、デフォルト構成が本番想定で
>   ないことを明示する（後述）

## ログ要件

無効化されていることを運用者が気づけるよう、起動時に WARN レベルで出力する。

| 状態 | ログ例 |
|------|-------|
| 認証無効で起動 | `WARN: maharit-viz authentication is DISABLED. /api/query is publicly accessible. Set MAHARIT_VIZ_AUTH=true (or --auth) to enable.` |
| TLS 無効で起動 | `WARN: maharit-viz is serving over plain HTTP (no TLS). Set --tls-cert/--tls-key (or MAHARIT_VIZ_TLS_CERT/KEY) for production use.` |
| 認証無効＋TLS無効 | 上記 2 つの WARN を両方とも出す |
| 認証有効＋TLS無効 | TLS 無効の WARN に加えて `WARN: authentication credentials are transmitted over plain HTTP.` を追加 |

実装は `tracing::warn!` を使い、`maharit-server` 側 (`require_auth=false` 時) も
同じ方針で WARN を出す。INFO ログでサーバー起動を通知している箇所の直後で出すと
運用者が見落としにくい。

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

### Phase 2: maharit-viz 側のログイン・セッション管理（オプション）

> **デフォルトは無効。** `VizConfig::require_auth: bool` を新設し、CLI フラグ
> または環境変数で明示的に有効化したときだけログインフローが動く。
> 無効時は今まで通り誰でも `/api/query` を叩ける。

#### 切り替え方法

| 切り替え | デフォルト | 有効化 |
|---------|-----------|-------|
| CLI フラグ | 無効 | `maharit-viz --auth` |
| 環境変数 | 無効 | `MAHARIT_VIZ_AUTH=true` |
| docker-compose | 無効 | `environment: MAHARIT_VIZ_AUTH=true` |

CLI フラグと環境変数の両方を指定した場合は CLI フラグを優先。

#### バックエンド (`maharit-viz`)

認証**有効時のみ** 以下のエンドポイントと middleware を有効化する:

- ログインエンドポイント:
  - `POST /api/login { username, password }` → server に `Login` 投げて
    `sessionToken` を取得 → **HttpOnly Cookie** にセット
  - `POST /api/logout` → Cookie を消す（必要なら server に通知）
- 認証 middleware:
  - `/api/query` 等の API は Cookie の `sessionToken` を取り出して
    server リクエストに付与
  - Cookie がなければ `401 Unauthorized`

認証**無効時**は middleware を組み込まず、`/api/login` ルートも未登録に
すれば「ログイン機能はそもそも存在しない」状態になる（混乱回避）。

#### Cookie 設計

- 名前: `maharit_viz_session`
- 属性: `HttpOnly; SameSite=Lax; Path=/`（TLS 有効時は `Secure` も自動付与）
- 有効期限: server 側 session の expiresAt と一致させる

XSS 対策のため **localStorage には保存しない**。

#### フロントエンド (`assets/`)

認証**有効時のみ** 以下を有効化:

- `index.html` を 2 画面構成に:
  - `/login` 相当のログインフォーム
  - `/` (既存) のクエリ UI（未認証なら `/login` にリダイレクト）
- セッション切れの 401 を検知したらログイン画面に戻す
- ヘッダに「現在のユーザー名」と「ログアウト」ボタンを表示

認証**無効時**はログインフォームを表示せず、現状の UI のみ。
フロント側は `/api/info` のレスポンスに `"auth_enabled": true|false` を含めて
判定する。

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

### Phase 4: TLS サポート（オプション）

> **デフォルトは無効。** TLS 関連の CLI フラグ／環境変数が指定された
> ときだけ HTTPS で listen する。指定なしでは現状通り HTTP。
> 認証フェーズと独立して有効化／無効化できる
> （例: 「認証は有効・TLS は無効」も「認証は無効・TLS は有効」も成立）。

#### 切り替え方法

| 切り替え | デフォルト | 有効化 |
|---------|-----------|-------|
| CLI フラグ | 無効 (HTTP) | `maharit-viz --tls-cert <PATH> --tls-key <PATH>` |
| 環境変数 | 無効 (HTTP) | `MAHARIT_VIZ_TLS_CERT=...` + `MAHARIT_VIZ_TLS_KEY=...` |
| docker-compose | 無効 (HTTP) | 上記環境変数 + 証明書をボリュームでマウント |

両方指定されたとき初めて TLS が有効になる（片方だけは起動時にエラー）。

#### 案 1: axum 自体に TLS サポートを追加（推奨デフォルト）

- `axum-server` の `bind_rustls` を使う
- 上記の CLI フラグ／環境変数を実装
- メリット: 余計なコンポーネント不要、`docker compose up` 1 発で完結
- デメリット: 証明書管理を viz が抱える

#### 案 2: リバースプロキシ前提（ドキュメントで案内のみ）

- viz は HTTP のまま、`docker-compose.yml` の追加サンプルとして caddy / nginx
  の構成例を提供
- メリット: TLS 設定の柔軟性、Let's Encrypt 自動化が楽
- デメリット: コンポーネント増加

→ **コードとして実装するのは案 1**。案 2 は `docs/operations/` に
構成例を載せるだけ。

## UX フロー

### 認証無効（デフォルト）

```
ブラウザでアクセス → そのままクエリ画面（現状と同じ）
```

ログインフォームもログアウトボタンも一切表示しない。

### 認証有効

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

## 設定マトリクス

| 認証 | TLS | 想定用途 |
|-----|-----|---------|
| 無効 | 無効 | **デフォルト**。ローカル開発、Docker compose で立ち上げて触る |
| 有効 | 無効 | 信頼できる内部ネットワーク + 認証は要る場合（推奨はしない） |
| 無効 | 有効 | エッジで TLS 終端したいだけのケース |
| 有効 | 有効 | **本番運用の推奨構成** |

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

## 解決済み (2026-06-15)

全 4 フェーズを段階的に実装。デフォルトは両方無効、起動時 WARN ログで通知。

### Phase 1: server/client TCP プロトコル拡張 (commit 641c95c5)

- `ServerConfig::require_auth: bool` (default false)
- `Request::Login { username, password }` + 既存に `session_token: Option<String>`
- `Response::LoggedIn` / `Response::AuthError`
- `TcpServer` に `AuthManager` 統合 + `check_session()` ヘルパー
- `Client::login` / `logout` / `session_token()` / `set_session_token()`
- WARN: `maharit-server authentication is DISABLED. ...`
- 検証: workspace 全体 1068 件 PASS

### Phase 2 + 4: maharit-viz の認証 + TLS (commit 952f009e)

- `VizConfig::require_auth: bool` (default false)
- `VizConfig::tls: Option<TlsConfig>` (default None)
- CLI: `--auth` / `--tls-cert <PATH> --tls-key <PATH>`
- 環境変数: `MAHARIT_VIZ_AUTH=true` / `MAHARIT_VIZ_TLS_CERT/KEY`
- 認証無効時は `/api/login` / `/api/logout` ルートを登録しない
- 認証有効時: HttpOnly Cookie `maharit_viz_session` で認証
  + middleware `auth_gate` で Cookie 検証
- TLS 有効時は `axum_server::bind_rustls` で HTTPS
- WARN: 認証無効 / TLS 無効 / 認証情報の HTTP 送信 を 3 種類区別

### Phase 3: フロントエンド UI (commit a2630dc5)

- `index.html` にログイン modal + ロールバッジ + ログアウトボタン
- `styles.css` にダークテーマの認証 UI スタイル
- `modules/api.js` に `login()` / `logout()`
- `modules/auth.js` 新規: `init(els, callback)` で `/api/info` の `auth_enabled` に応じて UI 切替
- `app.js` で 401 検知 → 自動再ログイン誘導

### 動作検証 (Docker)

```
$ curl -s http://localhost:8080/api/info
{"auth_enabled":false,"tls_enabled":false,...}

$ curl http://localhost:8080/api/login -d ...
HTTP 405  (route not registered)

$ docker logs maharit-viz
WARN: maharit-viz authentication is DISABLED. ...
WARN: maharit-viz is serving over plain HTTP (no TLS). ...
```

`--auth` フラグを付けた viz では:

```
$ curl -X POST .../api/login -d '{"username":"admin","password":"admin"}'
HTTP 200 + Set-Cookie: maharit_viz_session=...; HttpOnly; SameSite=Lax

$ curl --cookie ... .../api/query -d '{"query":"MATCH (n) RETURN COUNT(n) AS c"}'
HTTP 200 + {"columns":["c"],"rows":[{"c":"0"}]}

$ curl -X POST .../api/logout
HTTP 204
```

### スコープ外 (将来検討)

- OAuth / OIDC
- LDAP / AD 連携
- MFA
- 同時セッション数制限
- フロントの ReadOnly 警告（`auth.isWriteQuery` ヘルパーは追加済み、UI 連携は未）
