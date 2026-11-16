# 194: CI でレプリケーションのテストがまれに失敗する

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
