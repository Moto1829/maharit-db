# 202: CI でレプリケーションのテストがまれに失敗する

## 起きたこと

2026-11-16、main `30abab12` の Test で 2 か所が同時に失敗し、失敗したジョブだけを再実行すると両方通った
（run 43879582473）。同じコミットをローカルで、差分テスト 15 seed × 3 回、E2E（レプリケーション 3 回）を
実行しても再現しなかった。

1. soak「Replication diff (15 seeds)」: seed 3 が `follower was disconnected` で失敗
   - 収束（グラフの一致）と索引の検査は通っていて、最後の `repl.get_follower_count() != 1` だけが失敗
   - この seed は最終的に 43 ノード / 1,024 エッジ（`MATCH (a:A), (b:A) … CREATE` でエッジが増える）
   - ログでは seed の途中に `Replication: connected to the leader` がもう一度出ている（フォロワーが一度切れて繋ぎ直した）
   - 再現コマンド: `MAHARIT_DIFF_SEEDS=3 MAHARIT_DIFF_OPS=200 MAHARIT_DIFF_CLIENTS=4 cargo test -p maharit-server leader_and_follower_converge`
2. E2E: 2 回目のクラスター起動が失敗（`✗ クラスター起動失敗（…/cluster_2.log）`）。1 回目の `replication_test_1` は 45 件通過
   - `cluster_2.log` は CI に残っておらず、理由は分からない

## 調べること

- 1: 遅いマシンでフォロワーが切断される理由（broadcast の遅れによる切断か、ハートビートの時間切れか）。
  切断して繋ぎ直すのが正しい動作なら、テストは「最後に 1 台繋がっている」ことを待ってから確かめる
- 2: `scripts/start_replication_local.sh` が直前のクラスターの終了（ポートの解放）を待っているか。
  失敗時に `cluster_N.log` と各ノードのログを CI の成果物として残す

## 結果（2026-11-21）

手元では 40 並列で走らせても再現しなかったので、コードとログから原因を絞って両方に手当てした。

1. 差分テスト: ログの「connected to the leader」は、接続が切れて再接続を待った後にだけ出る。リーダーは WAL のキューが
   `WAL_SEND_TIMEOUT`（5 秒）詰まったままのフォロワーを切り離し、フォロワーは 1 秒後に繋ぎ直してスナップショットから同期し直す。
   遅い CI でこれが起き、テストが繋ぎ直しの前にフォロワー数を数えていたと考えられる（グラフの一致は先に満たされていた）。
   設計どおりの動作なので、テストは最大 10 秒フォロワーが繋がるのを待ってから確かめるようにした
2. E2E: `stop_replication_local.sh` が kill を送るだけで終了を待っていなかった。停止時のスナップショット保存中に次の
   クラスターを起動すると、ポートが取れずに失敗するか、`wait_port` がまだ動いている古いリーダーに繋がる。
   停止を待つ（15 秒で強制終了）ようにし、起動側もポートが空くのを確かめるようにした。起動に失敗したときも各ノードのログを
   `$E2E_LOG_DIR` に残す（CI の成果物に入る）

3. 本当の原因（手元で `test_wal_burst_larger_than_queue_is_not_dropped` が同じ形で落ちて分かった）:
   - `get_follower_count` はフォロワー表のロックを `try_read` で取り、取れないと 0 を返していた。ACK のたびに書き込みロックを
     取るので、書き込みが続いている間に数えると、繋がっていても 0 台に見える（stats の `follower_count` も同じ）
   - フォロワーが同じ ID で繋ぎ直した後に古い接続の後始末が走ると、新しい接続の項目まで切断済みにしていた
   - 生きているフォロワー数を `AtomicUsize` で持ち（表を変えたときに数え直す）、項目に接続番号を持たせて、自分の接続のときだけ
     切断済みにするようにした。回帰テスト `test_follower_count_survives_lock_and_reconnect`

確認: `bash scripts/run_e2e_local.sh --replication-runs 2` が全スイート通過。
