# タスク 133: clients/python-native の依存更新（pyo3 など）

## 概要（2026-11-03）

`clients/python-native` は独立した Cargo workspace（`[workspace]` で切り離し）で、lockfile も別。
そのため CI の `cargo test --workspace` にも依存監査にも入っていない。`cargo audit --file clients/python-native/Cargo.lock`
では脆弱性 26 件・警告 8 件が出た。

- pyo3 0.22.6: `PyString::from_object` のバッファオーバーフローの恐れ（RUSTSEC-2025-0020、0.24.1 で修正）、
  `PyCFunction::new_closure` に `Sync` 境界が無い（RUSTSEC-2026-0177、0.29.0 で修正）
- rustls / rustls-webpki / aws-lc-sys / quinn-proto / rkyv / tar など（`cargo update` で解消できる見込み）
- lockfile が古く、`maharit-core` が 0.1.0 のまま（本体は 0.2.0）

## 作業
- pyo3 を 0.29 以上に上げ、API の変更（Bound API など）に追従する
- `cargo update` で lockfile を更新し、ビルドと Python 側のテストを確認する
- `.github/workflows/audit.yml` の python-native 行を `blocking: true` にする（task 135 で非ブロッキングとして追加済み）

## 対応内容（2026-11-03）
- pyo3 0.22 → 0.30 に更新し、`cargo update` で lockfile も更新（maharit-core 0.2.0 を参照）。
  `cargo audit --file clients/python-native/Cargo.lock` は脆弱性・警告とも 0 件
- `src/lib.rs` を新 API に移行（`PyDict::new` / `PyList::new`、`IntoPyObject`、`PyObject` を `Bound` に）。
  `get_node` / `get_edge` の `unwrap()` をやめて `PyResult` でエラーを返す形に
- 既存の不具合: `pyproject.toml` が存在しない `python/` ディレクトリとモジュール名 `maharit_native._core` を
  指定していて `maturin develop` が失敗していた。README と Rust 側に合わせて `module-name = "maharit_native"` に修正
- `tests/test_graph.py`（unittest）を追加し、README の使い方・存在しない ID・削除を検証
- CI: `test.yml` に python-native ジョブ（clippy → maturin develop → unittest）を追加。
  `audit.yml` の python-native をブロッキングに変更
- 確認: ローカルで clippy クリーン、`maturin develop` 成功、unittest 3 件通過、actionlint OK

## ステータス
完了
