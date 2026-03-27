---
title: トランザクション
parent: 高度なトピック
nav_order: 1
---

# トランザクション

MaharitDB はトランザクションをサポートしており、複数の操作をアトミックに実行できます。

## 基本的な使い方

### 明示的なトランザクション

```cypher
-- トランザクション開始
BEGIN

-- 複数の操作
CREATE (alice:Person {name: "Alice", balance: 1000})
CREATE (bob:Person {name: "Bob", balance: 500})
MATCH (alice:Person {name: "Alice"}), (bob:Person {name: "Bob"})
SET alice.balance = alice.balance - 100
SET bob.balance = bob.balance + 100

-- コミット（変更を確定）
COMMIT
```

```cypher
-- エラーが発生した場合はロールバック
BEGIN
MATCH (n:Person {name: "Alice"})
SET n.balance = n.balance - 10000  -- 残高不足だった場合
ROLLBACK
```

### 暗黙的なトランザクション

単一のクエリは自動的にトランザクション内で実行されます。

```cypher
-- このクエリ全体がアトミックに実行される
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
CREATE (a)-[:KNOWS]->(b)
SET a.updated_at = 2024
```

## MVCC（マルチバージョン同時実行制御）

MaharitDB はスナップショット分離（Snapshot Isolation）に基づく MVCC を実装しています。

### スナップショット分離の動作

```
時刻 T1: トランザクション A 開始 → スナップショット取得
時刻 T2: トランザクション B 開始 → スナップショット取得
時刻 T3: トランザクション A が Alice の age を読み取る → 30
時刻 T4: トランザクション B が Alice の age を 31 に更新してコミット
時刻 T5: トランザクション A が Alice の age を再度読み取る → 30（スナップショット時点の値）
時刻 T6: トランザクション A がコミット
```

### 分離レベル

MaharitDB は Snapshot Isolation（スナップショット分離）を提供します。

- **コミット済み読み取り（Read Committed）**: コミット済みのデータのみ読み取り可
- **スナップショット分離（Snapshot Isolation）**: トランザクション開始時点のスナップショットから読み取り

Phantom Read（ファントム読み取り）については、スナップショット分離により防止されます。

## 書き込み競合の検出

同じデータを複数のトランザクションが書き込もうとした場合、後発のトランザクションが中断されます（First-Committer-Wins ルール）。

```
トランザクション A: alice.balance = 900 に更新中
トランザクション B: alice.balance = 800 に更新中（並行）
→ B が先にコミットした場合、A はコミット時にエラーになる
→ アプリケーション側でリトライが必要
```

Rust クライアントでのリトライ実装例：

```rust
use maharit_client::Client;

async fn transfer_with_retry(
    client: &mut Client,
    from: &str,
    to: &str,
    amount: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let max_retries = 3;

    for attempt in 0..max_retries {
        let result = client.execute_transaction(|tx| async move {
            tx.execute(&format!(
                "MATCH (a:Person {{name: '{from}'}}) SET a.balance = a.balance - {amount}"
            )).await?;
            tx.execute(&format!(
                "MATCH (b:Person {{name: '{to}'}}) SET b.balance = b.balance + {amount}"
            )).await?;
            Ok(())
        }).await;

        match result {
            Ok(_) => return Ok(()),
            Err(e) if e.is_conflict() && attempt < max_retries - 1 => {
                eprintln!("Conflict detected, retrying (attempt {}/{})", attempt + 1, max_retries);
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    Err("Max retries exceeded".into())
}
```

## WAL との連携

トランザクションがコミットされると、変更内容は WAL（Write-Ahead Log）に書き込まれます。

```
1. BEGIN: トランザクション開始エントリを WAL に記録
2. 操作: 各変更を WAL にバッファリング
3. COMMIT: WAL にコミットエントリを書き込み（ディスクへの同期）
4. グラフへの変更を適用
```

サーバーがクラッシュした場合、WAL を再生することでコミット済みのトランザクションが復元されます。

## トランザクションの制限

- 長時間のトランザクションはメモリを消費し、他のトランザクションのコンフリクト検出に影響します
- デフォルトのトランザクションタイムアウトは 60 秒です（設定変更可能）
- 非常に大量のデータを変更するトランザクションは分割することを推奨します

## バッチ処理のパターン

大量データの処理には UNWIND を使った一括操作を推奨します：

```cypher
-- 一度のトランザクションで 1000 件のノードを作成
UNWIND $batch AS item
CREATE (n:Person {name: item.name, age: item.age})
```

パラメータで渡すバッチサイズは 1,000〜10,000 件が推奨です。

## WAL グループコミット

WAL グループコミットは、複数の書き込みリクエストを1回の `fsync` にまとめることで、書き込みスループットを向上させる機能です。通常、WAL へのコミットは1件ごとにディスク同期（`fsync`）を行いますが、高い書き込み負荷がかかるシステムでは `fsync` の呼び出し回数がボトルネックになります。グループコミットはこの問題を、複数のリクエストをバッファリングしてから1回の `fsync` でまとめてフラッシュすることで解決します。

### 動作原理

```
1. 書き込みリクエストが mpsc チャンネル経由でバックグラウンドタスクに送信される
2. バックグラウンドタスクが WAL バッファへの追記（fsync なし）を行う
3. 以下のいずれかの条件でフラッシュが実行される:
   - インターバル経過（デフォルト: 5ms）
   - バッチサイズ到達（デフォルト: 100件）
4. Wal::sync()（fsync）を1回呼び出す
5. フラッシュ完了後、各リクエストに LSN を返す
```

呼び出し元は `append()` を `await` するだけでよく、バックグラウンドでのバッチ処理は透過的に行われます。

### デフォルト設定

| パラメータ | デフォルト値 | 説明 |
|---|---|---|
| `flush_interval_ms` | `5` | フラッシュ間隔（ミリ秒） |
| `flush_batch_size` | `100` | バッチサイズ（件数） |

### 同期モードとの違い

同期モードでは書き込み1件ごとに `fsync` が呼ばれるため、耐久性は最大ですがスループットが低くなります。グループコミットモードでは複数の書き込みを1回の `fsync` にまとめるため、スループットが向上する一方、クラッシュ時に最大でインターバル分（デフォルト5ms）の書き込みが失われる可能性があります。

### 設定例

```rust
use maharit_storage::WalGroupCommitConfig;

// デフォルト: 5ms インターバル / 100件バッチ
let config = WalGroupCommitConfig::default();

// 同期モード（グループコミット無効、flush_interval_ms=0, flush_batch_size=1）
let sync_config = WalGroupCommitConfig::synchronous();

// カスタム設定: 10ms インターバル / 50件バッチ
let custom_config = WalGroupCommitConfig::new(10, 50);
```

`WalGroupCommitter::start()` は tokio ランタイム上でバックグラウンドタスクを起動します。tokio ランタイム外で呼ぶとパニックします。

### パフォーマンストレードオフ

| 設定 | 書き込みレイテンシ | スループット | クラッシュ時のデータ損失リスク |
|---|---|---|---|
| 同期モード（`synchronous()`） | 低（1件ごと確定） | 低 | なし |
| デフォルト（5ms / 100件） | 最大 5ms | 高 | 最大 5ms 分 |
| 高スループット（50ms / 1000件） | 最大 50ms | 最高 | 最大 50ms 分 |

耐久性を最優先する用途（金融トランザクション等）では同期モードを使用し、スループットを優先する用途（ログ収集・バルクインポート等）ではインターバルとバッチサイズを大きく設定することを推奨します。
