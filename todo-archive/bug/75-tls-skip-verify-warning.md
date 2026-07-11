# bug/75: TLS 証明書検証スキップ時の警告を追加

## 概要
`skip_verify`（サーバー側 `build_client_config` / クライアント側 `TlsClientConfig`）で
TLS 証明書検証をバイパスできる。既定は `false`（オプトイン）だが、有効化しても
何の警告も出ず、運用者が誤って本番で使うと中間者攻撃に対して無防備になる。

## 対応
- `tls.rs::build_client_config` の `skip_verify` 経路に `tracing::warn!` と `eprintln!` の
  明確な警告を追加。
- `maharit-client` の `skip_verify` 経路にも `eprintln!` 警告を追加
  （client は tracing 非依存のため eprintln のみ）。

## 影響
- 重大度: INFO（既定 false・テスト用オプトインのまま）
- 動作は変えず、危険な設定を可視化するのみ。

## ステータス
完了（server 231 / client 32 テストパス）
