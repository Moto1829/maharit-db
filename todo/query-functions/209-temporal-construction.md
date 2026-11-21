# タスク 209: 時間型の作り方を標準に合わせる（時刻型・タイムゾーン・書式・マップ）

## 背景（2026-11-21、openCypher TCK）
時間型の TCK 1,004 シナリオのうち 20 しか通らない。このタスクは「値を作る」側（Temporal1・2・4・5・7）を扱う。

足りないもの:
- 型: `localdatetime` / `time` / `localtime` が無い。`datetime` はタイムゾーン（`+01:00`、`[Europe/Stockholm]`）を持てない
- `datetime({year: …, month: …, …})` などマップからの作成（今は `date` だけ。`datetime()` は文字列のみ）
- 文字列の書式: 週（`2015-W30-2`）、通日（`2015-202`）、区切りなしの基本形式（`20150721`、`T214032`）、年月だけ（`2015-07`）
- `duration` の小数の成分（`{days: 1.5}`、`P0.75M`）
- 成分の取り出し（`.year` `.week` `.dayOfWeek` `.timezone` `.epochSeconds` など）と比較

TCK: `temporal/Temporal1`（約 200）、`Temporal2`（約 50）、`Temporal4`（約 40）、`Temporal5`、`Temporal7`。

## 進め方
- `Value` / `PropertyValue` に型を足す（ナノ秒の精度、タイムゾーンのオフセットと名前）。永続化（スナップショット・WAL）の形式に新しい型が増えるので、`maharit-storage/CLAUDE.md` の手順で版を上げる
- タイムゾーン名の解決には IANA のデータが要る。依存（chrono-tz など）を足すか、オフセットだけに対応するかを先に決める
- 段階に分ける: 型と永続化 → マップと文字列からの作成 → 成分と比較

## 作業
- [ ] 型の設計と永続化（版を上げる、旧版の読み込みを残す）
- [ ] `localdatetime` / `time` / `localtime` / タイムゾーン付き `datetime`
- [ ] マップ・文字列（週・通日・基本形式）からの作成、`duration` の小数
- [ ] 成分の取り出しと比較
- [ ] docs の時間型のページ（日英）
- [ ] 段階ごとに `feat(query):`。既存の `datetime` の表示や精度が変わるなら `!`
