# ストレージ: バイナリシリアライズへの移行

## 概要

スナップショットの保存が独自テキスト形式のため低速。
`bincode` または `MessagePack` への移行でシリアライズ/デシリアライズコストを半減させる。

## 現状の問題

```rust
// persistence.rs
fn write_string<W: Write>(writer: &mut W, s: &str) -> Result<()> {
    Self::write_u32(writer, s.len() as u32)?;
    writer.write_all(s.as_bytes())?;  // 独自フォーマット
    Ok(())
}

// ラベルを文字列結合してから保存
Self::write_string(&mut writer, &node.labels.join(":"))?;  // アロケーション発生

// 読み込み時に毎回 UTF-8 検証
String::from_utf8(buf).map_err(|_| ...)?;
```

また `BufWriter` のデフォルトバッファ（8KB）が小さく、
大規模グラフ保存時にシステムコールが頻発する。

## 実装内容

- [x] `bincode` クレートを導入
- [x] `Node`, `Edge`, `Graph` に `#[derive(Serialize, Deserialize)]` を追加
  （すでに `serde` は利用中）
- [x] `save()` / `load()` を `bincode::encode_into_std_write()` /
  `bincode::decode_from_std_read()` に置き換え
- [x] ラベルを `join(":")` せず `Vec<String>` のまま保存（複数ラベル対応も同時解決）
- [x] `BufWriter::with_capacity(4 * 1024 * 1024, file)` に変更（4MB バッファ）
- [x] 既存のスナップショット形式からの移行パス（バージョンヘッダで判定）

## 対象クレート

`maharit-storage`
