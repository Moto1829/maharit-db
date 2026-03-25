# Task 75: サーバー起動統合テスト

## 背景・目的

現在の Rust テストは JSON のパース/シリアライズ検証が中心で、
実際に `TcpServer::start()` を呼び出してソケット通信するテストが存在しない。

これにより `main.rs` の配線漏れや、モジュール間の結合不具合が
ユニットテストをすり抜けて本番環境で発現するリスクがある。

## 実装内容

`crates/maharit-server/src/tcp_server.rs` の `#[tokio::test]` として追加する。

### テストケース

#### 基本クエリ
- サーバーを 0.0.0.0:0（ランダムポート）で起動し TCP 接続
- `Ping` → `pong` レスポンスを確認
- `CREATE (n:Person {name: 'Alice'}) RETURN n` → ノードが返ること
- `MATCH (n:Person {name: 'Alice'}) RETURN n.name` → `"Alice"` が返ること
- `MATCH (n:Person) DETACH DELETE n` → 削除できること

#### プロパティの保存と取得
- 文字列・整数・浮動小数点・真偽値プロパティを持つノードを作成
- MATCH で各プロパティを取得し値が一致すること

#### エッジ操作
- ノード2つを作成 → エッジを作成 → MATCH でトラバーサルできること

#### エラーハンドリング
- 構文エラーのクエリに `Error` レスポンスが返ること
- 接続後すぐ切断してもサーバーがクラッシュしないこと

## 実装方法

```rust
async fn start_test_server() -> (TcpServer, String) {
    let config = ServerConfig {
        bind_address: "127.0.0.1:0".to_string(),
        ..Default::default()
    };
    let server = TcpServer::new(config);
    let addr = /* バインドされた実際のアドレスを取得 */;
    tokio::spawn(async move { server.start().await });
    tokio::time::sleep(Duration::from_millis(10)).await;
    (server, addr)
}
```

`ServerConfig::bind_address` にポート 0 を使うか、テスト用に
`TcpServer::start_and_return_addr()` のようなメソッドを追加する。

## 完了条件

- [x] `cargo test -p maharit-server integration` が通ること
- [x] 実際のソケット通信を経由したクエリ実行テストが3件以上あること（7件実装）
- [x] プロパティの保存・取得が検証されていること
