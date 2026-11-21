# タスク 210: 時間型の関数と演算を足す（truncate・between・toString・算術）

前提: タスク 209（型が揃ってから）。

## 背景（2026-11-21、openCypher TCK）
- `date.truncate('month', d)` など切り捨て（`Temporal9`、322 シナリオ。全て構文エラー）
- `duration.between(a, b)` / `duration.inMonths` / `inDays` / `inSeconds`（`Temporal10`、131）
- 別の時間型からの変換 `date(datetime)`、`datetime({date: d, time: t})` など（`Temporal3`、約 180）
- `toString(時間型)` が未対応（`Temporal6`、12）
- 時間型 + `duration` の算術の不足、`duration * 数` / `duration / 数`（`Temporal8`）

## 作業
- [ ] パーサー: `date.truncate(...)` のような名前空間付きの関数呼び出し（`duration.between` も同じ）
- [ ] truncate（単位ごと、マップでの上書き）
- [ ] between / inMonths / inDays / inSeconds
- [ ] 型どうしの変換
- [ ] toString（標準の書式）
- [ ] duration の乗除と、全ての時間型との加減
- [ ] docs（日英）、`feat(query):`
