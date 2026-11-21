//! ノード・エッジの ID（連番の `u64`）を鍵にする表のためのハッシュ。
//!
//! 標準の `RandomState`（SipHash）は文字列などを想定した、整数 1 つには重い計算をする。
//! ID は外から選べない連番なので、乗算 1 回（フィボナッチハッシュ）で十分に散らばる。
//! 上位ビットもよく混ざるので、上位ビットで分割先を決める DashMap にも使える。

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// `u64` の ID 用の `Hasher`。整数以外を渡されたときもバイト列から計算する。
#[derive(Default, Clone, Copy)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0 ^ n).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // 上位ビットを下位にも混ぜる（hashbrown は下位、DashMap は上位を使う）
        self.0 ^ (self.0 >> 29)
    }
}

pub type IdBuildHasher = BuildHasherDefault<IdHasher>;
pub type IdMap<K, V> = HashMap<K, V, IdBuildHasher>;
pub type IdSet<K> = HashSet<K, IdBuildHasher>;
