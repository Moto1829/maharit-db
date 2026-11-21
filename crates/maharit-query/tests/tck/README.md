# openCypher TCK (vendored)

`features/` は [openCypher](https://github.com/opencypher/openCypher) の TCK（Technology Compatibility Kit）の
`tck/features/expressions/` から、次の領域をそのまま写したもの（commit `677cbafabb8c3c5eed458fd3b1ec0daec8d67d23`）。

boolean / comparison / conditional / list / literals / mathematical / null / precedence / string / temporal

ライセンスは Apache License 2.0（同じフォルダの `LICENSE`）。ファイルは書き換えない。
更新するときは同じ構成で写し直し、`MAHARIT_TCK_WRITE=1 cargo test -p maharit-query --test tck` で
`tests/tck_known_failures.txt` を作り直す。実行器は `tests/tck.rs`。
