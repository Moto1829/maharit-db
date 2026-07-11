# bug/89: replication_test.py / failover_test.py の `import socket` 欠落（ローカルモード検出が常に失敗）

## 概要
`scripts/replication_test.py` と `scripts/failover_test.py` の `_port_open()` は
`socket.create_connection(...)` を使うのに、両ファイルとも `import socket` が抜けていた。
そのため `socket` が未定義 → `except Exception` で握りつぶされ `_port_open` が**常に False** を返し、
`check_docker_compose()` のローカルプロセスモード検出（全ポート open 判定）が成立せず、
`start_replication_local.sh` で起動しても「レプリケーション環境が起動していません」と誤って
終了していた（docker モードでしか実行できなかった）。

## 対応
- 両ファイルの stdlib import に `import socket` を追加。

## 検証（本修正後の実行結果）
- `replication_test.py`（ローカル 3 ノード）: **26/26 通過**
  （初回のみフォロワー2が初回接続タイミングで部分同期する一過性の揺れがあったが、
   クラスター再起動後の再実行で安定して全通過。決定論的バグではない）。
- `failover_test.py --no-docker`: **15/16 通過**。
  失敗 1 件「フォロワー2: is_leader_alive が false」は非対話ローカルモードの制約
  （リーダーを実際に停止/昇格する手順が手動前提で自動続行されるため、リーダー死亡が
   発生しない）＋ `stats` コマンドが `is_leader_alive` を公開していないため。→ bug/90 で別途記録。

## ステータス
完了（import 追加）
