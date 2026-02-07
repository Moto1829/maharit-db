# Temporal functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/temporal/

## 概要
- `date`, `time`, `localtime`, `datetime`, `localdatetime` と各種 `truncate`/`realtime`/`statement`/`transaction`。
- `datetime.fromEpoch` / `fromEpochMillis`。

## 重要ルール
- current値取得は時計種別（`statement`/`transaction`/`realtime`）を選択可能。
- `truncate` は指定単位で切り捨て、下位成分は既定値。
- 型変換は `date({date: ...})` などで可能。

## 関連
- Duration/Format関数は別ページ参照。
