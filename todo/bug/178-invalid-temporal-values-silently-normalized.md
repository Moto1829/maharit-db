# バグ 178: 存在しない日付や範囲外の duration がエラーにならず、別の値になる

## 現状（2026-11-15、main 60302578 で確認）
```text
RETURN date('2024-02-30') AS x                          // => 2024-03-01   期待: エラー
RETURN date({year: 2024, month: 13, day: 40}) AS x      // => 2025-02-09   期待: エラー
RETURN duration({months: 9223372036854775807}) AS x     // => P-1M         期待: エラー（overflow）
RETURN date('2024-01-01') + duration({months: 9223372036854775807}) AS x   // => 2023-12-01
```
入力ミスやデータ不正が、エラーにならず別の日付として保存・比較される。

## 原因
- date の文字列・マップの読み込みで月・日の範囲（うるう年含む）を検査せず、日数に換算して繰り上げている
- `executor.rs` の duration 構築で `(years * 12 + months_v) as i32` としており、範囲外が黙って切り詰められる
  （`years * 12` 自体も i64 でオーバーフローしうる）

## 作業
- [ ] date / datetime / localdatetime の文字列とマップで、月 1–12・日 1–月末・時分秒の範囲を検査してエラーにする
- [ ] duration の各成分を `checked_*` と `i32::try_from` で組み立て、範囲外は `integer overflow` エラーにする（bug/161 と同じ扱い）
- [ ] テンポラル + duration の加算もオーバーフローを検査
- [ ] 今まで受け付けていた入力がエラーになるので `fix(query)!:` で出す
