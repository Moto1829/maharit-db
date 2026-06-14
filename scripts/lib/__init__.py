"""MaharitDB E2E スクリプト用の共通モジュール。

`scripts/` 配下の Python テストスクリプトから利用される。

- `client`: TCP プロトコル (4 バイト長プレフィックス + JSON) のクライアント実装
- `reporting`: ANSI カラー定数と pass/fail 集計ヘルパー (check / section / summarize)
"""
