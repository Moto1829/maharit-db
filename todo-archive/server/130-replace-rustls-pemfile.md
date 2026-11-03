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

## 対応内容（2026-11-03）
- `tls.rs` / maharit-client を `rustls::pki_types::pem::PemObject`（`CertificateDer::pem_reader_iter`、
  `PrivateKeyDer::from_pem_reader`）に置き換え、ワークスペースと両クレートの依存から `rustls-pemfile` を削除
- エラーメッセージ（"no certificates found" / "no private key found"）は従来どおり
- クライアント: CA ファイルに証明書が 1 つも無い場合、ハンドシェイク時の unknown issuer ではなく
  読み込み時に `InvalidCertificate("No certificates found in CA cert file: …")` を返すようにした
- テスト追加: テスト専用の自己署名 P-256 証明書と鍵（PKCS#8 / SEC1）での読み込み・ServerConfig 構築・
  クライアント CA 利用、証明書や鍵が無いファイル・壊れた PEM のエラー、空の CA ファイル
  （それまで実際の証明書を読み込むテストは無かった）
- `rustls-pemfile` は axum-server 0.7（maharit-viz）経由で `Cargo.lock` に残るため、`cargo audit` の警告は
  axum-server 0.8 への更新（task 134）で消える
- 検証: clippy クリーン、`cargo test --workspace` 全通過（1150 件）

## ステータス
完了
