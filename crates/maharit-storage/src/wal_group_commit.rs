//! WAL グループコミット（非同期バッファリング）
//!
//! 複数の書き込みリクエストをまとめて1回の `fsync` でフラッシュすることで
//! 書き込みスループットを向上させる。
//!
//! # 動作原理
//!
//! 1. 呼び出し元は `WalGroupCommitter::append()` を await する。
//! 2. バックグラウンドタスクがリクエストを mpsc チャンネルで受け取り、
//!    WAL バッファに追記する（`Wal::append`、fsync なし）。
//! 3. インターバル経過またはバッチサイズ到達で `Wal::sync()` を1回呼び出す。
//! 4. フラッシュ完了後、各リクエストの oneshot チャンネルに LSN を送信する。
//!
//! # 設定
//!
//! ```
//! use maharit_storage::WalGroupCommitConfig;
//!
//! // デフォルト: 5ms インターバル / 100件バッチ
//! let config = WalGroupCommitConfig::default();
//!
//! // 同期モード（グループコミット無効）
//! let sync_config = WalGroupCommitConfig::synchronous();
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::time;

use crate::wal::{RecordPayload, RecordType, Wal, WalError};
use crate::Lsn;

/// グループコミット設定
#[derive(Debug, Clone)]
pub struct WalGroupCommitConfig {
    /// バッファフラッシュ間隔（ミリ秒）
    /// - `0` は同期モード（各書き込み後に即時 fsync）
    pub flush_interval_ms: u64,
    /// バッファフラッシュのバッチサイズ
    /// - このサイズに達した場合は `flush_interval_ms` を待たずに fsync する
    /// - `1` は同期モードと同等
    pub flush_batch_size: usize,
}

impl Default for WalGroupCommitConfig {
    fn default() -> Self {
        Self {
            flush_interval_ms: 5,
            flush_batch_size: 100,
        }
    }
}

impl WalGroupCommitConfig {
    /// 同期モード（グループコミットなし）
    pub fn synchronous() -> Self {
        Self {
            flush_interval_ms: 0,
            flush_batch_size: 1,
        }
    }

    /// カスタム設定
    pub fn new(flush_interval_ms: u64, flush_batch_size: usize) -> Self {
        Self {
            flush_interval_ms,
            flush_batch_size: flush_batch_size.max(1),
        }
    }
}

type WriteRequest = (
    RecordType,
    RecordPayload,
    oneshot::Sender<Result<Lsn, WalError>>,
);

/// WAL グループコミッター
///
/// `Arc<Mutex<Wal>>` を共有しながら非同期でバッチ書き込みを行う。
/// `start()` を呼ぶと tokio バックグラウンドタスクが起動される。
pub struct WalGroupCommitter {
    sender: mpsc::Sender<WriteRequest>,
}

impl WalGroupCommitter {
    /// グループコミッターを起動する
    ///
    /// # Panics
    ///
    /// tokio ランタイム外で呼ぶとパニックする。
    pub fn start(wal: Arc<Mutex<Wal>>, config: WalGroupCommitConfig) -> Self {
        let (sender, receiver) = mpsc::channel::<WriteRequest>(1024);
        tokio::spawn(Self::run(wal, receiver, config));
        Self { sender }
    }

    /// WAL に書き込みリクエストを送信し、fsync 完了まで待機する
    ///
    /// 戻り値は割り当てられた LSN。
    pub async fn append(
        &self,
        record_type: RecordType,
        payload: RecordPayload,
    ) -> Result<Lsn, WalError> {
        let (tx, rx) = oneshot::channel();

        self.sender
            .send((record_type, payload, tx))
            .await
            .map_err(|_| {
                WalError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "WAL group committer is shut down",
                ))
            })?;

        rx.await.map_err(|_| {
            WalError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "WAL group committer dropped response channel",
            ))
        })?
    }

    // ── バックグラウンドタスク ─────────────────────────────────────────────

    async fn run(
        wal: Arc<Mutex<Wal>>,
        mut receiver: mpsc::Receiver<WriteRequest>,
        config: WalGroupCommitConfig,
    ) {
        let interval_ms = config.flush_interval_ms;
        let batch_size = config.flush_batch_size.max(1);

        // 同期モード: 各書き込み直後に fsync
        if interval_ms == 0 || batch_size == 1 {
            while let Some((record_type, payload, tx)) = receiver.recv().await {
                let result = {
                    let mut w = wal.lock().unwrap();
                    w.append(record_type, payload).and_then(|lsn| {
                        w.sync()?;
                        Ok(lsn)
                    })
                };
                let _ = tx.send(result);
            }
            return;
        }

        // グループコミットモード
        let mut pending: Vec<(Lsn, oneshot::Sender<Result<Lsn, WalError>>)> = Vec::new();
        let mut interval = time::interval(Duration::from_millis(interval_ms));
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;

                req = receiver.recv() => {
                    match req {
                        None => {
                            // Sender がドロップされた: 残りをフラッシュして終了
                            if !pending.is_empty() {
                                Self::do_flush(&wal, &mut pending);
                            }
                            return;
                        }
                        Some((record_type, payload, tx)) => {
                            // WAL バッファに追記（fsync はまだしない）
                            let result = {
                                let mut w = wal.lock().unwrap();
                                w.append(record_type, payload)
                            };
                            match result {
                                Ok(lsn) => pending.push((lsn, tx)),
                                Err(e) => {
                                    let _ = tx.send(Err(e));
                                }
                            }
                            // バッチサイズに達したら即時フラッシュ
                            if pending.len() >= batch_size {
                                Self::do_flush(&wal, &mut pending);
                            }
                        }
                    }
                }

                _ = interval.tick() => {
                    if !pending.is_empty() {
                        Self::do_flush(&wal, &mut pending);
                    }
                }
            }
        }
    }

    /// pending キューを fsync してから各リクエストに結果を通知する
    fn do_flush(
        wal: &Arc<Mutex<Wal>>,
        pending: &mut Vec<(Lsn, oneshot::Sender<Result<Lsn, WalError>>)>,
    ) {
        let sync_result = {
            let mut w = wal.lock().unwrap();
            w.sync()
        };

        match sync_result {
            Ok(()) => {
                for (lsn, tx) in pending.drain(..) {
                    let _ = tx.send(Ok(lsn));
                }
            }
            Err(e) => {
                // WalError は Clone でないため、エラーメッセージを文字列化して配布する
                let msg = e.to_string();
                for (_, tx) in pending.drain(..) {
                    let _ = tx.send(Err(WalError::Io(std::io::Error::other(
                        msg.clone(),
                    ))));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::{RecordPayload, RecordType, Wal};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::UNIX_EPOCH;

    fn temp_path() -> PathBuf {
        let id = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from(format!("/tmp/maharit_gc_test_{}.wal", id))
    }

    #[tokio::test]
    async fn test_group_committer_basic() {
        let path = temp_path();
        let wal = Arc::new(Mutex::new(Wal::open(&path).unwrap()));
        let committer = WalGroupCommitter::start(Arc::clone(&wal), WalGroupCommitConfig::default());

        let lsn = committer
            .append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 0,
                    label: "Person".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(lsn, 1);
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_group_committer_synchronous_mode() {
        let path = temp_path();
        let wal = Arc::new(Mutex::new(Wal::open(&path).unwrap()));
        let committer =
            WalGroupCommitter::start(Arc::clone(&wal), WalGroupCommitConfig::synchronous());

        let lsn1 = committer
            .append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 0,
                    label: "A".to_string(),
                },
            )
            .await
            .unwrap();

        let lsn2 = committer
            .append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 1,
                    label: "B".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(lsn1, 1);
        assert_eq!(lsn2, 2);
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_group_committer_batch_100_writes() {
        let path = temp_path();
        let wal = Arc::new(Mutex::new(Wal::open(&path).unwrap()));
        let config = WalGroupCommitConfig::new(10, 50);
        let committer = WalGroupCommitter::start(Arc::clone(&wal), config);

        // 100 件を並列送信
        let mut handles = Vec::new();
        for i in 0u64..100 {
            let c = committer.sender.clone();
            handles.push(tokio::spawn(async move {
                let (tx, rx) = oneshot::channel();
                c.send((
                    RecordType::CreateNode,
                    RecordPayload::CreateNode {
                        node_id: i,
                        label: "Bulk".to_string(),
                    },
                    tx,
                ))
                .await
                .unwrap();
                rx.await.unwrap().unwrap()
            }));
        }

        let lsns: Vec<Lsn> = futures_util::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // LSN は 1〜100 の範囲に全て含まれる（順序は問わない）
        let mut sorted = lsns.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 100);
        assert_eq!(*sorted.first().unwrap(), 1);
        assert_eq!(*sorted.last().unwrap(), 100);

        // WAL ファイルに 100 件記録されていること
        let record_count = {
            let w = wal.lock().unwrap();
            w.record_count().unwrap()
        };
        assert_eq!(record_count, 100);

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_group_committer_durability() {
        // fsync 後に WAL を別プロセス相当で再オープンしてもデータが残っていること
        let path = temp_path();
        let wal = Arc::new(Mutex::new(Wal::open(&path).unwrap()));
        let committer = WalGroupCommitter::start(Arc::clone(&wal), WalGroupCommitConfig::default());

        for i in 0u64..5 {
            committer
                .append(
                    RecordType::CreateNode,
                    RecordPayload::CreateNode {
                        node_id: i,
                        label: "Durable".to_string(),
                    },
                )
                .await
                .unwrap();
        }

        // グループコミッターを捨てて WAL を閉じる（ドロップ）
        drop(committer);
        // WAL のロックを解放するため Arc を捨てる
        drop(wal);

        // 再オープンしてリカバリ
        let wal2 = Wal::open(&path).unwrap();
        let mut graph = maharit_core::Graph::new();
        let last_lsn = wal2.recover(&mut graph).unwrap();

        assert_eq!(last_lsn, 5);
        assert_eq!(graph.node_count(), 5);

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_group_committer_config_custom() {
        let config = WalGroupCommitConfig::new(20, 200);
        assert_eq!(config.flush_interval_ms, 20);
        assert_eq!(config.flush_batch_size, 200);
    }
}
