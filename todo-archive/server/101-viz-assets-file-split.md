# Task 101: maharit-viz の index.html を CSS / JS / HTML に分離

## 背景

Task 82 → 98 → 99 → 100 の積み重ねで `crates/maharit-viz/assets/index.html`
が **875 行** まで肥大化した。CSS（テーマ・テーブル・グラフ詳細・履歴
ドロップダウン）と JS（API クライアント・タブ切替・Tabulator・cytoscape・
履歴管理）と HTML が単一ファイルに同居している。

問題点:

- diff が読みづらくなり、コードレビューしにくい
- CSS と JS が一緒にキャッシュされるため、片方だけ修正してもクライアントが
  両方を再取得する
- 関心の分離ができていない（CSS/JS を別エディタウィンドウで見られない）
- 文字列リテラル中の HTML/CSS/JS を syntax-highlight するエディタ設定が必要

## 提案

`crates/maharit-viz/assets/` を以下のように分割する:

```
assets/
  index.html        # マークアップだけ。<link rel="stylesheet"> と <script src> で参照
  styles.css        # 全 CSS（テーマ変数 + コンポーネント）
  app.js            # アプリ全体の JS（後述の Task 105 でさらに分割する想定）
```

`tower-http::services::ServeDir` は既にディレクトリ全体を配信するので、
ファイル追加だけで自動的に配信される。Dockerfile.viz の COPY もディレクトリ
単位なので変更不要。

## 検証

- ブラウザで強制リロード後、Network タブで以下のリクエストが 200 を返すこと:
  - `/` → index.html
  - `/styles.css`
  - `/app.js`
- Table / Graph / Raw JSON / History / 詳細パネルの全機能が回帰なし
- `cargo test -p maharit-viz` が通る（既存テストは Rust 側のみ）

## 優先度

LOW（バグではない / 触る頻度の高いファイルなので効果は実感しやすい）

## 関連タスク

- Task 105 と組み合わせて実施するのが効率的（同じファイルを触るため）

## 関連ファイル

- `crates/maharit-viz/assets/index.html`

## 解決済み (2026-06-14)

Task 105 と同時実施。`crates/maharit-viz/assets/` を以下に分割:

```
assets/
  index.html          # 75 行 (タグのみ + CDN + module 読み込み)
  styles.css          # 全 CSS をここに集約
  app.js              # エントリポイント (DOM 配線 + Run query フロー)
  modules/
    api.js / table_view.js / graph_view.js / history.js / tabs.js / util.js
```

ServeDir はディレクトリ全体を再帰的に配信するため変更不要。Dockerfile.viz の
`COPY ... /app/assets` も同様に変更なし。

### 検証

- `curl -sI http://localhost:8080/{styles.css,app.js,modules/api.js}` → 200 OK / 正しい MIME
- `/api/query` の動作も継続して問題なし
- index.html は 875 行 → 75 行へ大幅縮小
