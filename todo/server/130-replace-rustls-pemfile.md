# タスク 130: メンテ終了の rustls-pemfile をやめる

## 概要（2026-11-03）

`rustls-pemfile` 2.2.0 はメンテナンス終了（RUSTSEC-2025-0134）。後継は `rustls-pki-types` の
PEM API（`rustls::pki_types::pem::PemObject`）。rustls 0.23 系なら追加の依存は不要。

## 使っている箇所
- `crates/maharit-server/src/tls.rs`（証明書と秘密鍵の読み込み、ホットリロード）
- `crates/maharit-client/src/lib.rs`（CA 証明書の読み込み）

## 作業
- `CertificateDer::pem_file_iter` / `PrivateKeyDer::from_pem_file` などに置き換え、`rustls-pemfile` を依存から外す
- 不正な PEM・鍵が無いファイルのときのエラーメッセージを変えないようにする（TLS のテストで確認）
