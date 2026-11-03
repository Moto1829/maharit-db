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
- task 135 の CI 監査にこの lockfile も含める
