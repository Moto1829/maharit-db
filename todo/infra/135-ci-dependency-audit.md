# タスク 135: CI に依存クレートの監査を入れる

## 概要（2026-11-03）

`.github/workflows/test.yml` には依存の脆弱性チェックが無く、dependabot の設定も無い。
今回の監査（task 128）で見つかった 36 件の脆弱性は、どれも定期チェックがあれば早く気づけた。

## 作業
- CI に `cargo audit`（または `cargo deny check advisories bans sources`）のジョブを足す
  - 対象はワークスペースの `Cargo.lock` と `clients/python-native/Cargo.lock`
  - 新しい勧告が出たときに気づけるよう、push / PR に加えて週 1 回の `schedule` でも実行する
  - 許容する警告（例: bincode のメンテ終了。task 131 で対応するまで）は理由付きで `audit.toml` / `deny.toml` に書く
- `.github/dependabot.yml` を追加する（`cargo` と `github-actions`。セキュリティ更新は個別、それ以外は週次でまとめる）
- 余力があれば、GitHub Actions の `uses:` をタグではなくコミット SHA で固定する（サプライチェーン対策）
