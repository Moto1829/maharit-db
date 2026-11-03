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

## 対応内容（2026-11-03）
- `.github/workflows/audit.yml` を追加
  - トリガー: `Cargo.toml` / `Cargo.lock` / ワークフロー自体が変わった push・PR、毎週月曜 00:00 UTC、手動実行
  - `taiki-e/install-action` で cargo-audit を入れ、`cargo-audit audit --file <lockfile>` を実行
    （`cargo audit` だと rust-toolchain.toml によりツールチェーンのインストールが走るため直接呼ぶ）
  - ワークスペースの `Cargo.lock` はブロッキング。`clients/python-native/Cargo.lock` は pyo3 の更新（task 133）まで
    `continue-on-error`（現状は脆弱性ありで失敗する）
  - 脆弱性でのみ失敗し、メンテ終了・unsound・yanked は警告として表示（task 130〜132 で対応中）
- `.github/dependabot.yml` を追加（cargo: ルートと python-native、github-actions。週次、minor/patch はまとめて 1 PR）
- 確認: actionlint・YAML 構文 OK、ローカルの cargo-audit で終了コードがワークスペース 0 / python-native 1 になることを確認
- 見送り: `uses:` のコミット SHA 固定。dependabot の github-actions 更新で追従できる状態にしてから別途検討する

## ステータス
完了
