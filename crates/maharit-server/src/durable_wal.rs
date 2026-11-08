//! On-disk write-ahead log for the server.
//!
//! Every write statement's mutations ([`WalEntryData`], the same entries that
//! are streamed to replication followers) are appended to `<data>.wal` before
//! the client gets its response. On startup the server loads the snapshot
//! (`<data>`) and replays the WAL on top of it, so a crash (`kill -9`, power
//! loss) loses no acknowledged write.
//!
//! # File format
//!
//! ```text
//! header:  b"MHRTWAL1" | base: [u8; 32]
//! record:  len: u32 LE | crc32(payload): u32 LE | payload: JSON Vec<WalEntryData>
//! ```
//!
//! `base` is the SHA-256 of the snapshot file the log applies to (all zeros
//! when there was no snapshot). A checkpoint writes a new snapshot and then a
//! fresh log whose `base` is the new snapshot's digest. If the process dies
//! between the two steps, the old log's `base` no longer matches the snapshot
//! and is discarded at startup instead of being applied twice.
//!
//! A record that is cut short or fails its CRC can only be the last one being
//! written when the process died; replay stops there and the tail is truncated.
//!
//! # Group commit
//!
//! Records are appended while the caller holds the server's write gate (so log
//! order equals execution order). `sync_to` is then called *after* releasing
//! the gate: concurrent writers that queue up behind an in-flight `fsync` are
//! all covered by the next one.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::replication::WalEntryData;

const MAGIC: &[u8; 8] = b"MHRTWAL1";
const HEADER_LEN: u64 = 8 + 32;
/// Upper bound for one record; larger lengths can only come from corruption.
const MAX_RECORD_LEN: usize = 1 << 30;

/// SHA-256 digest identifying a snapshot file.
pub type SnapshotDigest = [u8; 32];

/// How the server persists writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalMode {
    /// Append to the WAL and `fsync` before replying (survives power loss).
    Fsync,
    /// Append to the WAL without `fsync` (survives a process crash, not an OS
    /// crash or power loss).
    Os,
    /// No WAL: data is only saved at shutdown / checkpoint.
    Off,
}

impl WalMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fsync" => Some(Self::Fsync),
            "os" => Some(Self::Os),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// Path of the WAL that belongs to the data file `data_path`.
pub fn wal_path(data_path: &Path) -> PathBuf {
    let mut s = data_path.as_os_str().to_owned();
    s.push(".wal");
    PathBuf::from(s)
}

/// Digest of the snapshot at `path`, or all zeros if it does not exist.
pub fn snapshot_digest(path: &Path) -> io::Result<SnapshotDigest> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Sha256::digest(&bytes).into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok([0; 32]),
        Err(e) => Err(e),
    }
}

/// What [`DurableWal::open`] found in an existing log.
#[derive(Debug, Default)]
pub struct Recovery {
    /// Write statements to replay, in order.
    pub batches: Vec<Vec<WalEntryData>>,
    /// Bytes of a torn / corrupt tail that were cut off.
    pub truncated_bytes: u64,
    /// The log belonged to an older snapshot (a checkpoint finished writing
    /// the snapshot but not the new log) and was discarded.
    pub discarded_stale: bool,
}

pub struct DurableWal {
    path: PathBuf,
    mode: WalMode,
    /// Append handle. Only written while holding the server's write gate.
    file: Mutex<File>,
    /// Bytes written so far (end offset of the last appended record).
    written: AtomicU64,
    /// End offset known to be on disk. The mutex also serializes `fsync`s.
    synced: Mutex<u64>,
}

impl DurableWal {
    /// Open the log at `path` for the snapshot identified by `base`, returning
    /// the records to replay.
    pub fn open(path: &Path, base: SnapshotDigest, mode: WalMode) -> io::Result<(Self, Recovery)> {
        let mut recovery = Recovery::default();
        let end = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(mut file) => {
                let mut header = [0u8; HEADER_LEN as usize];
                let len = file.metadata()?.len();
                if len < HEADER_LEN {
                    // Crashed while creating the log: nothing was logged yet.
                    None
                } else {
                    file.read_exact(&mut header)?;
                    if &header[..8] != MAGIC {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("{} is not a MaharitDB WAL file", path.display()),
                        ));
                    }
                    if header[8..] != base {
                        recovery.discarded_stale = true;
                        None
                    } else {
                        let valid_end = read_records(&mut file, len, &mut recovery.batches)?;
                        if valid_end < len {
                            recovery.truncated_bytes = len - valid_end;
                            file.set_len(valid_end)?;
                            file.sync_all()?;
                        }
                        Some(valid_end)
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let end = match end {
            Some(end) => end,
            None => {
                write_fresh(path, base)?;
                HEADER_LEN
            }
        };
        let mut file = OpenOptions::new().write(true).open(path)?;
        file.seek(SeekFrom::Start(end))?;
        Ok((
            Self {
                path: path.to_path_buf(),
                mode,
                file: Mutex::new(file),
                written: AtomicU64::new(end),
                synced: Mutex::new(end),
            },
            recovery,
        ))
    }

    /// Size of the log in bytes.
    pub fn len(&self) -> u64 {
        self.written.load(Ordering::SeqCst)
    }

    /// True when the log holds no records.
    pub fn is_empty(&self) -> bool {
        self.len() <= HEADER_LEN
    }

    /// Append one write statement's entries. Must be called while holding the
    /// server's write gate. Returns the end offset to pass to [`sync_to`](Self::sync_to).
    pub fn append(&self, entries: &[WalEntryData]) -> io::Result<u64> {
        let payload = serde_json::to_vec(entries)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut record = Vec::with_capacity(8 + payload.len());
        record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(&crc32(&payload).to_le_bytes());
        record.extend_from_slice(&payload);

        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        file.write_all(&record)?;
        let end = self
            .written
            .fetch_add(record.len() as u64, Ordering::SeqCst)
            + record.len() as u64;
        Ok(end)
    }

    /// Block until everything up to `offset` is on disk (no-op unless the mode
    /// is [`WalMode::Fsync`]). Call without holding the write gate so that
    /// concurrent writers share one `fsync`.
    pub fn sync_to(&self, offset: u64) -> io::Result<()> {
        if self.mode != WalMode::Fsync {
            return Ok(());
        }
        let mut synced = self.synced.lock().unwrap_or_else(|e| e.into_inner());
        if *synced >= offset {
            return Ok(());
        }
        // Everything appended so far is covered by this fsync.
        let target = self.written.load(Ordering::SeqCst);
        let file = self
            .file
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_clone()?;
        file.sync_data()?;
        *synced = target;
        Ok(())
    }

    /// Start a fresh, empty log for the snapshot `base` (after a checkpoint
    /// saved a new snapshot). Must be called while holding the write gate.
    pub fn reset(&self, base: SnapshotDigest) -> io::Result<()> {
        write_fresh(&self.path, base)?;
        let mut file = OpenOptions::new().write(true).open(&self.path)?;
        file.seek(SeekFrom::End(0))?;
        *self.file.lock().unwrap_or_else(|e| e.into_inner()) = file;
        self.written.store(HEADER_LEN, Ordering::SeqCst);
        *self.synced.lock().unwrap_or_else(|e| e.into_inner()) = HEADER_LEN;
        Ok(())
    }
}

/// Atomically replace `path` with an empty log (tmp file + fsync + rename).
fn write_fresh(path: &Path, base: SnapshotDigest) -> io::Result<()> {
    let tmp = {
        let mut s = path.as_os_str().to_owned();
        s.push(".tmp");
        PathBuf::from(s)
    };
    {
        let mut f = File::create(&tmp)?;
        f.write_all(MAGIC)?;
        f.write_all(&base)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    sync_parent_dir(path)
}

/// `fsync` the directory containing `path` so a rename is durable.
pub fn sync_parent_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Read records after the header; return the end offset of the last valid one.
fn read_records(
    file: &mut File,
    len: u64,
    batches: &mut Vec<Vec<WalEntryData>>,
) -> io::Result<u64> {
    let mut bytes = Vec::with_capacity((len - HEADER_LEN) as usize);
    file.read_to_end(&mut bytes)?;
    let mut pos = 0usize;
    while bytes.len() - pos >= 8 {
        let n = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
        if n > MAX_RECORD_LEN || bytes.len() - pos - 8 < n {
            break;
        }
        let payload = &bytes[pos + 8..pos + 8 + n];
        if crc32(payload) != crc {
            break;
        }
        match serde_json::from_slice::<Vec<WalEntryData>>(payload) {
            Ok(entries) => batches.push(entries),
            Err(_) => break,
        }
        pos += 8 + n;
    }
    Ok(HEADER_LEN + pos as u64)
}

/// CRC-32 (IEEE 802.3), bitwise. Records are small and this is not on the
/// read path, so a table is not worth it.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "maharit_durable_wal_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn node(id: u64) -> WalEntryData {
        WalEntryData::CreateNode {
            node_id: id,
            labels: vec!["P".to_string()],
        }
    }

    #[test]
    fn crc32_matches_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn append_and_recover() {
        let dir = tmp_dir("recover");
        let path = dir.join("db.wal");
        let base = [7u8; 32];
        {
            let (wal, rec) = DurableWal::open(&path, base, WalMode::Fsync).unwrap();
            assert!(rec.batches.is_empty());
            let end = wal.append(&[node(0), node(1)]).unwrap();
            wal.sync_to(end).unwrap();
            let end = wal.append(&[node(2)]).unwrap();
            wal.sync_to(end).unwrap();
        }
        let (wal, rec) = DurableWal::open(&path, base, WalMode::Fsync).unwrap();
        assert_eq!(rec.batches.len(), 2);
        assert_eq!(rec.batches[0].len(), 2);
        assert_eq!(rec.truncated_bytes, 0);
        // Appending after reopen continues the same log.
        wal.append(&[node(3)]).unwrap();
        drop(wal);
        let (_, rec) = DurableWal::open(&path, base, WalMode::Fsync).unwrap();
        assert_eq!(rec.batches.len(), 3);
    }

    #[test]
    fn torn_tail_is_truncated() {
        let dir = tmp_dir("torn");
        let path = dir.join("db.wal");
        let base = [1u8; 32];
        {
            let (wal, _) = DurableWal::open(&path, base, WalMode::Os).unwrap();
            wal.append(&[node(0)]).unwrap();
            wal.append(&[node(1)]).unwrap();
        }
        // Cut the last record in half.
        let len = std::fs::metadata(&path).unwrap().len();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - 5)
            .unwrap();
        let (_, rec) = DurableWal::open(&path, base, WalMode::Os).unwrap();
        assert_eq!(rec.batches.len(), 1);
        assert!(rec.truncated_bytes > 0);
        // The file was cut back to the last valid record.
        let (_, rec) = DurableWal::open(&path, base, WalMode::Os).unwrap();
        assert_eq!(rec.batches.len(), 1);
        assert_eq!(rec.truncated_bytes, 0);
    }

    #[test]
    fn corrupt_record_stops_replay() {
        let dir = tmp_dir("corrupt");
        let path = dir.join("db.wal");
        let base = [2u8; 32];
        {
            let (wal, _) = DurableWal::open(&path, base, WalMode::Os).unwrap();
            wal.append(&[node(0)]).unwrap();
            wal.append(&[node(1)]).unwrap();
        }
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 2;
        bytes[last] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        let (_, rec) = DurableWal::open(&path, base, WalMode::Os).unwrap();
        assert_eq!(rec.batches.len(), 1);
    }

    #[test]
    fn stale_log_is_discarded_and_reset_starts_over() {
        let dir = tmp_dir("stale");
        let path = dir.join("db.wal");
        {
            let (wal, _) = DurableWal::open(&path, [3u8; 32], WalMode::Os).unwrap();
            wal.append(&[node(0)]).unwrap();
        }
        // A different snapshot: the old log must not be replayed.
        let (wal, rec) = DurableWal::open(&path, [4u8; 32], WalMode::Os).unwrap();
        assert!(rec.discarded_stale);
        assert!(rec.batches.is_empty());
        assert!(wal.is_empty());

        wal.append(&[node(1)]).unwrap();
        wal.reset([5u8; 32]).unwrap();
        assert!(wal.is_empty());
        wal.append(&[node(2)]).unwrap();
        drop(wal);
        let (_, rec) = DurableWal::open(&path, [5u8; 32], WalMode::Os).unwrap();
        assert_eq!(rec.batches.len(), 1);
    }

    #[test]
    fn not_a_wal_is_an_error() {
        let dir = tmp_dir("magic");
        let path = dir.join("db.wal");
        std::fs::write(&path, vec![0u8; 64]).unwrap();
        assert!(DurableWal::open(&path, [0; 32], WalMode::Os).is_err());
    }

    #[test]
    fn wal_path_appends_suffix() {
        assert_eq!(
            wal_path(Path::new("/var/lib/maharit.db")),
            PathBuf::from("/var/lib/maharit.db.wal")
        );
    }
}
