---
name: release
description: MaharitDB のリリース（バージョンを上げてタグを打ち、GitHub Release を作る）や、次のバージョン番号・リリースノートの確認をするときに使う。
---

# リリース

バージョンの決め方（どのコミットでどの桁を上げるか）は CLAUDE.md の「バージョニング・リリース」が正本。

## 手順

```bash
brew install git-cliff                        # 初回のみ
bash scripts/release.sh auto --dry-run        # 次バージョンとリリースノートのプレビュー
bash scripts/release.sh 0.3.0                 # version 更新 → CHANGELOG 再生成 → commit → tag
bash scripts/release.sh 0.3.0 --push          # 上記 + push（release.yml が起動）
git cliff -o CHANGELOG.md                     # [Unreleased] を含めて CHANGELOG だけ再生成
```

- タグの push で `.github/workflows/release.yml` が動く: タグと Cargo.toml の version の一致を検証 → 5 ターゲットをビルド → `maharit-<tag>-<target>.tar.gz|.zip` と `.sha256`（`install.sh` / `install.ps1` が参照する名前）→ git-cliff のノートで GitHub Release を作る（`-rc.N` などは prerelease）
- 正式版のノートは途中の rc を畳み込み、直前の正式版からの差分になる
- タグの前に全ターゲットのビルドを確かめたいときは、Actions の Release を手動実行する（`workflow_dispatch` はビルドだけで Release は作らない）

## 注意

- 一度打ったタグは削除・force-push しない。問題があれば直して次の PATCH を出す
- タグを打つ・push するのは外から見える操作なので、ユーザーに確認してから行う
