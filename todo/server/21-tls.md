# TLS/SSL対応

## 概要
TCPサーバーとクライアント間の通信をTLS/SSLで暗号化する。

## 実装内容

### サーバー側
- [x] TLS設定の読み込み（証明書、秘密鍵）
- [x] TLSアクセプターの実装
- [x] 暗号化/非暗号化接続の両対応
- [x] 証明書の自動リロード（将来的）

### クライアント側
- [x] TLS接続オプション
- [x] 証明書検証の設定（検証/スキップ）
- [x] カスタムCA証明書の指定

### 設定
- [x] 環境変数による設定（MAHARIT_TLS_CERT, MAHARIT_TLS_KEY）
- [x] 設定ファイルでの指定
- [x] 最小TLSバージョンの設定

## 依存クレート候補
- `tokio-rustls` - 非同期TLS
- `rustls` - TLS実装

## 依存
- `12-tcp-server.md` が完了していること
- `18-client.md` が完了していること

## 対象クレート
`maharit-server`, `maharit-client`
