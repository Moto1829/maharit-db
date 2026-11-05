# タスク 137: README の英語化

状態: 完了 (2026-11-05)

方針（2026-11-05 決定）: GitHub のトップに出す `README.md` を英語にし、日本語は `README.ja.md` に移す。

## 作業内容
- [x] 現在の `README.md` を `git mv README.md README.ja.md` で移す（履歴を残す）
- [x] 英語の `README.md` を新規作成する（構成は日本語版と揃える）
- [x] 両ファイルの先頭に言語リンクを置く: `English | [日本語](README.ja.md)` / `[English](README.md) | 日本語`
- [x] 日本語版・英語版の両方で、古くなった記述や事実と違う記述を直す
  - 「mdBook 形式」→ MkDocs。リンク先を GitHub Pages（https://moto1829.github.io/maharit-db/ ）に変更。英語版 docs は準備中と明記（`/en/` はタスク 136 以降）
  - Rust バッジ `1.75+` → `1.98.1`（`rust-toolchain.toml` へリンク）。CI バッジを追加
  - clone URL・`Cargo.toml` の `repository`・`docs/quickstart.md` の clone URL を `Moto1829/maharit-db` に修正
  - インストール手順に `install.sh` / `install.ps1`（GitHub Release のバイナリ。v0.2.0 以降）を追加
  - 日本語形態素解析は `japanese` feature でビルドした場合のみと明記
  - 実装されていない「PITR（ポイントインタイムリカバリ）」を削除。zstd 圧縮を追記
  - `maharit-viz` の「WebSocket リアルタイム表示」は事実と違うため「Web 可視化アプリ」に修正
  - REPL の出力例が架空の形式だったため、実際の出力に差し替え（並びを固定するため `ORDER BY` を追加）
  - `maharit server` の既定は「インメモリ」ではなく `./maharit.db` に保存する、と正しく記載
  - Docker の 9090（メトリクス）は既定でコンテナ内ループバックに bind されて外から届かないため、記載しない
  - 存在しない `dijkstra()` 関数の例を `Dijkstra::with_weight_property(..).find_path(..)` に修正
  - 機能一覧に、インデックス・制約、A*、強連結成分・サイクル検出・トポロジカルソート、フェイルオーバー、Python クライアントを追記
- [x] `release.yml` のリリースノートに含まれるインストール手順と矛盾しないことを確認

## 検証
- README の Rust コード例（クライアント・アルゴリズム・入出力）を、使い捨てクレートでコンパイルできることを確認
- REPL の例を実際に実行し、出力をそのまま掲載

## 残課題
- **`LICENSE` ファイルが存在しない**（`Cargo.toml` では MIT と宣言済み）。著作権者名を決めて追加が必要。それまでライセンスバッジのリンク先は `Cargo.toml` にしてある
- `install.sh` / `install.ps1` は GitHub Release がまだ無いため、v0.2.0 をリリースするまで動かない
