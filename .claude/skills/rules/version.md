# バージョン管理

## バージョン体系 — Semantic Versioning MAJOR.MINOR.PATCH
  - 現在 0.x.y（API 未安定）。1.0.0 から安定版
  - MAJOR: プロトコル・フォーマット破壊 / MINOR: 新機能 / PATCH: バグ修正

## タグ命名
  - v0.2.0（v プレフィックス必須）
  - タグの強制上書き禁止（修正時はバージョンを上げる）
  - バージョンインクリメントに追従してタグをつけること
  
## Cargo.toml 管理
  - ルートの [workspace.package] version 一箇所のみ変更
  -　コードを変更した実装した場合は必ずバージョンをインクリメントすること