# Changelog

このファイルは [git-cliff](https://git-cliff.org) により Conventional Commits から自動生成されます。
形式は [Keep a Changelog](https://keepachangelog.com/ja/1.1.0/)、バージョンは [Semantic Versioning](https://semver.org/lang/ja/) に従います。

## [Unreleased]

### 新機能

- /analyze-benchmark スキルを追加、benchmark.py から API 呼び出しを削除 ([fe427b7](https://github.com/Moto1829/maharit-db/commit/fe427b79f320fb87289dd35dfbf89387038f97d1))
- プロパティインデックス DDL と MATCH 最適化を実装 (#71) ([758df6b](https://github.com/Moto1829/maharit-db/commit/758df6b0df1a99e334326b94983061f87cc03e0f))
- aggregation query optimization (task 72) ([df55e24](https://github.com/Moto1829/maharit-db/commit/df55e241d158edb46eda2382e7fda9e26f848538))
- 集計クエリ最適化 - GROUP BY と COUNT(var) 高速化 (#72) ([503f372](https://github.com/Moto1829/maharit-db/commit/503f372502078bfd809f52afd06de836c96ef017))
- エッジ作成のバインディングキャッシュ動作確認とテスト追加 (#74) ([aa4ae8a](https://github.com/Moto1829/maharit-db/commit/aa4ae8a6eca08218bc4b62162de7fcc6a28480cb))
- UNWIND+CREATE バッチ書き込み対応 (#70) ([36d43f7](https://github.com/Moto1829/maharit-db/commit/36d43f7a15e38781c4cc7dcb77114a2c2891fe08))
- WAL グループコミット実装 (#73) ([94aec7c](https://github.com/Moto1829/maharit-db/commit/94aec7cfaad2dcf85f3068f21505a0a6b79babbc))
- GROUP BY クエリに HashAggregation プランノードを追加 (#72) ([fd56bfa](https://github.com/Moto1829/maharit-db/commit/fd56bfacb1564c2604497e5177e195a892aace83))
- インストールスクリプト追加 (#67 フェーズ2) ([365f5ad](https://github.com/Moto1829/maharit-db/commit/365f5ada37f3502075089293623880fcfdb74eb4))
- シャーディングを TcpServer に統合 (#69) ([d62dc7a](https://github.com/Moto1829/maharit-db/commit/d62dc7aa804acc8e2eddfbefa10a510e0a89a06c))
- シャーディングを TcpServer に統合 (#69) ([0b5c9b1](https://github.com/Moto1829/maharit-db/commit/0b5c9b1ef22c2137f500db12adca5bcffa13f7ca))
- レプリケーション検証用 docker-compose.replication.yml を追加 ([6a611f1](https://github.com/Moto1829/maharit-db/commit/6a611f16490cc5763bbe9446b9a9d176020a2e9e))
- レプリケーション動作確認スクリプト追加 (scripts/replication_test.py) ([10c9acc](https://github.com/Moto1829/maharit-db/commit/10c9accc0ab046265d33db75efcb103b1bb2a3f1))
- **storage**: バックアップ・リストア時にインデックス定義を保存・復元 (Task 71) ([fca870d](https://github.com/Moto1829/maharit-db/commit/fca870d2fd0ecdbf71c4499dce5384f6e949f35a))
- バックアップ・リストア時にインデックス定義を保存・復元（Task 71） ([27ebe1f](https://github.com/Moto1829/maharit-db/commit/27ebe1f83b9941e6e0b818e077c1278f16874539))
- **query**: 単独RETURNステートメントとASエイリアスをサポート (Task 89) ([c3478d2](https://github.com/Moto1829/maharit-db/commit/c3478d273042a581b27e635c616c7800e295b599))
- **scripts**: 起動前にdocker-compose環境チェックを追加 ([a0ec914](https://github.com/Moto1829/maharit-db/commit/a0ec91450ae71d69153e1e849877bd69f915784d))
- **scripts**: レプリケーションローカルテスト実行環境を整備 (Task 91) ([9376306](https://github.com/Moto1829/maharit-db/commit/9376306e7106cbe588df15ff368a772bd364b81a))
- **e2e**: query_feature_test.py / constraint_test.py / auth_test.py を追加 ([811a5ab](https://github.com/Moto1829/maharit-db/commit/811a5ab1cb7215acc84abb8bd9b6399cc27bb649))
- **viz**: Phase 1 - axum バックエンド + 仮置きフロントを実装 (Task 82) ([958595b](https://github.com/Moto1829/maharit-db/commit/958595bfe18779d5c934e2e96deace74515ff9ee))
- **viz**: Phase 2 - Tabulator + cytoscape.js + タブ切替を追加 (Task 82) ([a93d427](https://github.com/Moto1829/maharit-db/commit/a93d4270ed155f67d927a9b9c372ef31b82a03a7))
- **viz**: Phase 3 - Dockerfile.viz + docker-compose 統合 (Task 82 完了) ([b1bb1d2](https://github.com/Moto1829/maharit-db/commit/b1bb1d2b09b051a87070297f564ff159760e5b42))
- **viz**: Graph タブでノード/エッジクリック時にプロパティを表示 (task 99) ([0ab78d4](https://github.com/Moto1829/maharit-db/commit/0ab78d492c94cc426f5780bddaef526d440aea1b))
- **viz**: クエリ履歴ドロップダウンを追加 (task 100) ([c42fb69](https://github.com/Moto1829/maharit-db/commit/c42fb69bcb667346e5bfede6d9df7ca591930535))
- **server,client**: Phase 1 - TCP プロトコルに認証を追加 (task 106) ([641c95c](https://github.com/Moto1829/maharit-db/commit/641c95c5f7c133915edacd6b321da711cb3d5cb9))
- **viz**: Phase 2 + 4 - 認証と TLS をオプトイン実装 (task 106) ([952f009](https://github.com/Moto1829/maharit-db/commit/952f009e66cffaa7b768f47013c299453a81e016))
- **viz/ui**: Phase 3 - 認証 UI とロール表示 (task 106) ([a2630dc](https://github.com/Moto1829/maharit-db/commit/a2630dc5ad83ba0549e3c2dd72b92834b66f82e0))
- **viz/graph**: Graph 描画の見やすさ改善 A+B+C+F (task 107) ([6a02c37](https://github.com/Moto1829/maharit-db/commit/6a02c37b3fe16db96e153b7e4eb25c5ece9ef32b))
- **server,client,viz**: クエリ結果の型情報を JSON で保持 (Option C) ([fd213d4](https://github.com/Moto1829/maharit-db/commit/fd213d49c1fdefd0c322faf753d93888fa1bf2f4))
- **server**: stats にレプリケーション生存情報を公開しfailoverを自動化 ([ad5e294](https://github.com/Moto1829/maharit-db/commit/ad5e2945cf27c84565e13988d19b10286b38b9c7))
- **server**: --node-id フラグを追加しレプリケーションの node_id 衝突を解消 ([584d217](https://github.com/Moto1829/maharit-db/commit/584d2177c6366157ea5f7d5de0deb9383e5ad1e9))
- **server**: stats のレプリケーション統計を拡充(node_id/LSN/follower数) ([609e6c8](https://github.com/Moto1829/maharit-db/commit/609e6c8c794d8b030a55dedc0a1b9ae9753fdea0))
- **query**: shortestPath パターン・YIELD 後の句・CALL {} 内 UNION に対応し、docs の Cypher 例をパース検査 (task123) ([827aacc](https://github.com/Moto1829/maharit-db/commit/827aacc942d0c3c544e007c3aa4194c06f89ac61))
- **query**: shortestPath パターン・YIELD 後の句・CALL {} 内 UNION に対応し、docs の Cypher 例をパース検査 (task123) ([52f3852](https://github.com/Moto1829/maharit-db/commit/52f385280039890595514591c47357b12cfe232b))

### バグ修正

- cargo clippy --all の警告を全て修正 ([910ba32](https://github.com/Moto1829/maharit-db/commit/910ba32c9be1ab6d725877c79ef790c143d076d4))
- clippy警告を修正 ([0b01a6f](https://github.com/Moto1829/maharit-db/commit/0b01a6f7767c4ec463720d583f23478e8be041e4))
- フォロワーノードが TcpServer と同じ ConcurrentGraph を共有するよう修正 ([a119256](https://github.com/Moto1829/maharit-db/commit/a119256d1c7f061d2146aeffb61760c77ab52f5a))
- WAL にプロパティを含めレプリケーションテストを全通過させる ([d8c0275](https://github.com/Moto1829/maharit-db/commit/d8c0275b3fdc01f101977f538664e85791d9ef8e))
- **storage**: clippy警告を修正 - collapsible_if ([dc419a2](https://github.com/Moto1829/maharit-db/commit/dc419a2d91f2a56c36e596d37563320589850d69))
- **query**: マップリテラルのキーにクォート付き文字列を許可 (Task 88) ([e3ec965](https://github.com/Moto1829/maharit-db/commit/e3ec965adcd2a1f5ff7de8c21ad6c7194676ad34))
- **scripts**: smoke_test の冪等性を確保 (Task 92) ([c303a15](https://github.com/Moto1829/maharit-db/commit/c303a159b8c9ee6732deb4aaa5711462d83f57eb))
- **query**: UNWIND インラインJSONマップリテラルの統合テストを追加 (Task 88) ([073d715](https://github.com/Moto1829/maharit-db/commit/073d71585f11070aa3189f0f3aaeda9cf0cac000))
- **infra**: perf_check の Docker vs release バイナリ不一致を修正 (Task 93) ([91cb041](https://github.com/Moto1829/maharit-db/commit/91cb0417afff0129b70dbebbdf1291f4a194023f))
- **infra**: benchmark.py の単発計測ノイズを繰り返し計測で修正 (Task 94) ([88702e9](https://github.com/Moto1829/maharit-db/commit/88702e93a6ff0cd7d4f4fa5b6623a7c31b45c703))
- **server**: persist ConstraintManager and FulltextManager across queries (task 96, 97) ([86d02c6](https://github.com/Moto1829/maharit-db/commit/86d02c62cf0c78fe353873c3df8238e3a9fef784))
- **query**: WITH clause group aggregation returns null (task 95) ([a44da30](https://github.com/Moto1829/maharit-db/commit/a44da3025282135a512d1b8fa959c3d2535f198e))
- **query**: キーワードをプロパティ名・キーとして受け入れる (task 98) ([6234c06](https://github.com/Moto1829/maharit-db/commit/6234c0645e92cc5567133476b4feb946c6e4cac4))
- **viz**: Tabulator のドット列名解釈を無効化して値が null になる問題を修正 ([488653b](https://github.com/Moto1829/maharit-db/commit/488653b33032387aa3af925f17ef0ed7c470d033))
- **viz**: API レスポンスの値の二重ダブルクォートを解消 ([ab88a5c](https://github.com/Moto1829/maharit-db/commit/ab88a5ca8e54f9299ce06257cf9dd2347adfb07e))
- **query**: キーワードをノードラベル/リレーション型として受け入れる (task 108) ([00995ef](https://github.com/Moto1829/maharit-db/commit/00995eff5185ac6a24154844a8bb6c5f841fde4a))
- **scripts**: concurrent_test の期待値を型情報維持に追従 ([2b7e826](https://github.com/Moto1829/maharit-db/commit/2b7e82601ffb3b7125b43d6e074d23d930a9d7b9))
- **server**: TCPメッセージ長に上限を追加しメモリ枯渇DoSを防止 ([ea6b0fc](https://github.com/Moto1829/maharit-db/commit/ea6b0fc98f409f48d2da2788c7dedbb70fefd992))
- **server**: 認証の有効化フラグとRBAC強制を追加 ([d5bfe87](https://github.com/Moto1829/maharit-db/commit/d5bfe87bec8f693f88a34ddb0bafc2039049056f))
- **server**: 認証暗号を堅牢化（CSPRNGトークン・PBKDF2・定数時間比較） ([9dbf4d1](https://github.com/Moto1829/maharit-db/commit/9dbf4d19a73032a74775d2a0e49395d90cbf5e48))
- **server**: レプリケーションに共有シークレット認証を追加 ([7e86791](https://github.com/Moto1829/maharit-db/commit/7e8679198c82e06ff05721fd0253c6a87264f04a))
- **server**: 監視HTTPの既定バインドをループバックに変更 ([0b3a81a](https://github.com/Moto1829/maharit-db/commit/0b3a81a0e39af5ab3f3dc6e9218676534a2fb27d))
- **server,client**: TLS証明書検証スキップ時に警告を出す ([f705346](https://github.com/Moto1829/maharit-db/commit/f70534612188c27ca045250403c1e4a56f2e704d))
- **server**: コーディネーターに認証・RBAC・メッセージサイズ上限を追加 ([79a3f95](https://github.com/Moto1829/maharit-db/commit/79a3f95d3118b8c96ea3bc90b8d18ea3324498e0))
- **query**: range() のリソース枯渇 DoS を修正（上限とオーバーフロー対策） ([b6eb184](https://github.com/Moto1829/maharit-db/commit/b6eb184ee04c95affd590dd9daf65c18690fc0e4))
- **scripts**: replication/failover テストの import socket 欠落を修正 ([34bfd02](https://github.com/Moto1829/maharit-db/commit/34bfd02cc3c1612114db8de047fc5ed062457b8f))
- **server**: ユーザー管理(CREATE/DROP/ALTER/SHOW USER)をAuthManagerに配線 ([c078c4f](https://github.com/Moto1829/maharit-db/commit/c078c4fe67dad39b0fd7f4fa2f393d2ab7e3a0e3))
- **lint**: clippy -D warnings を通す (redundant_closure, manual_abs_diff) (task115) ([c702304](https://github.com/Moto1829/maharit-db/commit/c70230478bf5b346b097c7fa2979b23d395c8b02))
- **server**: 共有インデックスのクエリ毎クローンと並行書き込みの lost update を解消 (task114) ([b1a2424](https://github.com/Moto1829/maharit-db/commit/b1a24245b1cb735ecef95ee802fdca7b5cb0bd0b))
- **lint**: workspace 全体の clippy -D warnings を解消し CLAUDE.md の lint コマンドを修正 (task115) ([e9065d3](https://github.com/Moto1829/maharit-db/commit/e9065d32332754193f9ea5ff174c08fdb6571585))
- **replication**: WAL を実行時の変更ログから生成し SET/REMOVE/ラベル変更を複製 (task116) ([5461217](https://github.com/Moto1829/maharit-db/commit/54612174a48d401e6763529a1954830d795d980c))
- **replication**: フォロワー ACK 読み取りのキャンセル非安全によるストリーム断を修正 (task120) ([ce53638](https://github.com/Moto1829/maharit-db/commit/ce536389cf66e4468394cb4e141b4c6eec761779))
- **tx**: ROLLBACK をフォロワーへ複製し、ラベル/DETACH エッジ/索引も正しく巻き戻す (task117) ([99aeb40](https://github.com/Moto1829/maharit-db/commit/99aeb409c911932301d32233880effac9ceb120f))
- **replication**: WAL 配信を背圧付きのフォロワー別キューにしバースト時の取りこぼしを解消 (task122) ([0c34458](https://github.com/Moto1829/maharit-db/commit/0c34458e0ec61af7510d012b21fc8d2ed6827f91))
- **query**: 解釈できない句を黙って無視せずエラーにし、無視されていた句を実装 (task118) ([7d80324](https://github.com/Moto1829/maharit-db/commit/7d803243256c6fe8bac16ed7d38dcad2ee1c81be))
- **query**: 複数ホップのパス照合の誤結果と匿名ノードを修正し、Cypher 適合テストを追加 (task119, 121-3/4) ([90d132a](https://github.com/Moto1829/maharit-db/commit/90d132ae1aa26556adf2bae6101f7371516c9721))
- **metrics**: Linux 専用コードの clippy collapsible_if を修正 (task77) ([703f6d7](https://github.com/Moto1829/maharit-db/commit/703f6d745316a9b751e882612163bcb733ef6351))

### パフォーマンス

- **ci**: パフォーマンス回帰テストを追加 (Task 81) ([e0101de](https://github.com/Moto1829/maharit-db/commit/e0101deb845bcc68ef5dabee655e2b6b7c392362))
- **baseline**: releaseビルドの実計測値でベースラインを更新 (Task 90) ([0127ff4](https://github.com/Moto1829/maharit-db/commit/0127ff4146bb83358a04f011799db391d93cce9f))
- **query**: MATCHスキャンでのNode全体クローンを排除 ([dfbebd2](https://github.com/Moto1829/maharit-db/commit/dfbebd2235bd1839f71e55b356ffbb670c48001a))
- **query**: ラベル索引をMATCHスキャンに配線 ([8f84793](https://github.com/Moto1829/maharit-db/commit/8f84793bda886929b4deec41a492837754bada0c))
- **server**: ASTキャッシュをネットワーククエリ経路に配線 ([c71b40f](https://github.com/Moto1829/maharit-db/commit/c71b40f213178b7b62c41ebeb2381d3e0df50c97))
- **query**: WHERE等価述語を索引/スキャンにプッシュダウン ([45860f2](https://github.com/Moto1829/maharit-db/commit/45860f2c8067bbe78e49d46fc15b986f9de2a836))
- **server**: プロパティ索引をリクエスト間で共有 ([030a2d1](https://github.com/Moto1829/maharit-db/commit/030a2d1434326e959cadb0756f5cd1261b1130c5))
- **query**: フィルタ無しCOUNTを全binding実体化せず短絡 ([5e8a2ec](https://github.com/Moto1829/maharit-db/commit/5e8a2ecf93f48099b04dcab22f77a5274a81990d))
- **query**: WHERE範囲述語の索引プッシュダウン + 索引メンテナンス ([c580a9e](https://github.com/Moto1829/maharit-db/commit/c580a9e939ad4fd2308bcd0eb47350956851a7e4))
- **query**: 索引が無い場合は範囲ヒント収集をスキップ ([1285bd4](https://github.com/Moto1829/maharit-db/commit/1285bd498446e216687e5ccb416c282351fd3a4b))
- **query**: Bindings のキーを Arc<str> 化し clone を軽量化 ([9b3ecfe](https://github.com/Moto1829/maharit-db/commit/9b3ecfe0aaf8218fee898e0c0ce8363a203f872a))

### リファクタリング

- **maharit-viz**: WebSocketモジュール削除・render_layersバグ修正・依存整理 ([b2d5b7e](https://github.com/Moto1829/maharit-db/commit/b2d5b7e7575d281a6fa7b9fc1d078faa4818a2aa))
- **query/lexer**: キーワード定義をマクロで単一テーブル化 (task 102) ([0d85695](https://github.com/Moto1829/maharit-db/commit/0d85695fcb1226f44262db4ae69512c06d7abcb2))
- **query/parser**: ドットアクセス読み取りを共通ヘルパー化 (task 103) ([618a932](https://github.com/Moto1829/maharit-db/commit/618a932edbd926bff204a925940c73a411535333))
- **scripts**: MaharitClient と reporting を共通モジュール化 (task 104) ([4e31a84](https://github.com/Moto1829/maharit-db/commit/4e31a843090a1459c8e61ebddb2251689677e303))
- **viz**: assets を CSS/JS/HTML に分離し JS を ES Modules 化 (task 101, 105) ([81ba7c1](https://github.com/Moto1829/maharit-db/commit/81ba7c175200a6c8669a2d3084bb4b9410c7bf02))

### ドキュメント

- 永続化モードの設計方針を明記（サーバー常時永続化 vs ライブラリオンメモリ） ([9e8ff49](https://github.com/Moto1829/maharit-db/commit/9e8ff492286a2726c0678b1754a17d54c835a4a7))
- mdBook を廃止して Jekyll just-the-docs + GitHub Pages に移行 ([2a7689b](https://github.com/Moto1829/maharit-db/commit/2a7689b181c1f8f83573d7f047a98235d3035a96))
- Liquid 処理を無効化して Cypher クエリの構文エラーを修正 ([aff2086](https://github.com/Moto1829/maharit-db/commit/aff2086d48e9b199e9944a463676d28c1a83504e))
- Jekyll を廃止して MkDocs + Material に移行 ([9b807bc](https://github.com/Moto1829/maharit-db/commit/9b807bcc1868d89c1d2861386567d6e49c0c5142))
- Jekyll 残存ファイルを削除・ワークフロー更新を確定 ([d6777ca](https://github.com/Moto1829/maharit-db/commit/d6777caa6cbbfd5078e7a3670f8aa055ced0efe3))
- Google Analytics (G-452HXMLL62) を追加 ([24e1e34](https://github.com/Moto1829/maharit-db/commit/24e1e34dd92b8a8a678a536380c8c6b46eb9d687))
- プロパティインデックスの構文とEXPLAINプランノードを実装に合わせて更新 ([13f440d](https://github.com/Moto1829/maharit-db/commit/13f440dba5654dcf2c5f0c8a7ce12033146afadd))
- MkDocs から mdBook に移行 (#48) ([1c71cff](https://github.com/Moto1829/maharit-db/commit/1c71cff74984d1d2854d411d24da96ed4f4b3883))
- E2E テストタスクを追加 (Task 75-81) ([594da99](https://github.com/Moto1829/maharit-db/commit/594da99cd37478e724b97139994ac02426eaaec7))
- Google Analytics トラッキングコードをテンプレートオーバーライドで直接埋め込む ([4a6fbe3](https://github.com/Moto1829/maharit-db/commit/4a6fbe3887d1937f9a3f4b6d95428343a3f8abe7))
- Task 82 - maharit-viz Web アプリ実装タスクを追加 ([f7fcea5](https://github.com/Moto1829/maharit-db/commit/f7fcea54e81b735bd70eba7d763db059b9f4d38e))
- **advanced**: WALグループコミットと並列化ドキュメントを追加 (Task 80) ([f427cc9](https://github.com/Moto1829/maharit-db/commit/f427cc910b8543597400d189c9dcb4af43f46a53))
- 81-cycle-detection-docs チェックボックスを完了に更新 ([f4a078e](https://github.com/Moto1829/maharit-db/commit/f4a078e2c27e3d06582f7c2c525f4c099778fdb3))
- add CLAUDE.md for Claude Code navigation ([5c85a8e](https://github.com/Moto1829/maharit-db/commit/5c85a8e05f0776dfff5d188b0abc375350002366))
- **viz**: Web アプリの内容を反映 (Task 82) ([2985da4](https://github.com/Moto1829/maharit-db/commit/2985da44563a8ae33e23c2b406ab3bb511b2d139))
- **quickstart**: Web UI (maharit-viz) のセクションを追加 (Task 82) ([2b9bfa8](https://github.com/Moto1829/maharit-db/commit/2b9bfa8a49997e89e7b59d372a788705d1ba50db))
- クレート依存関係図を README に追加し実態に合わせて修正 ([fe428b8](https://github.com/Moto1829/maharit-db/commit/fe428b82ab9aa417cb51384f0cfb526ad7190336))
- 日本語FTSの仕様を全文検索ドキュメントに追記 ([7a82e38](https://github.com/Moto1829/maharit-db/commit/7a82e384634faace6e48ec293f9d9a8672ae9439))
- CLAUDE.md の全テストコマンドを cargo test --workspace に修正 ([22f49fb](https://github.com/Moto1829/maharit-db/commit/22f49fb98daab559729a32dca941e844500d8b7f))

### テスト

- **maharit-server**: TCP統合テストを追加 (Task 75) ([e9a2e35](https://github.com/Moto1829/maharit-db/commit/e9a2e353f5d0570a3d15b0922e7d9ea3511dbb2c))
- **maharit-server**: レプリケーション統合テストを追加 (Task 76) ([e58a124](https://github.com/Moto1829/maharit-db/commit/e58a124e4bc0fbe7cfc25f14f7aab284f4e5f839))
- 永続化ラウンドトリップテストを追加 (Task 78) ([7af7b21](https://github.com/Moto1829/maharit-db/commit/7af7b2156452c59f1661b9f67bd241002590db33))
- **server**: 同時接続テストを追加 (Task 79) ([6c9f02e](https://github.com/Moto1829/maharit-db/commit/6c9f02e63a499e41c567fd918e7b3c84d2044cd6))
- **e2e**: 同時接続テストスクリプトを追加 (Task 79) ([fde657f](https://github.com/Moto1829/maharit-db/commit/fde657f742a416042ac4a08f43e5babf6df4a5af))
- **replication**: フェイルオーバーテストを追加 (Task 80) ([96a2eb4](https://github.com/Moto1829/maharit-db/commit/96a2eb4499a280da6c8d254a379270a14acf4229))
- **bench**: 大規模ノードのスケーリングベンチ項目を追加 ([69022a2](https://github.com/Moto1829/maharit-db/commit/69022a2ba5ac8f13703e204730a72ab1467bf67b))
- **bench**: スケーリング項目のノード数を --scaling-nodes で独立指定可能に ([e3065db](https://github.com/Moto1829/maharit-db/commit/e3065db22df1fc7214d0215f9fc4a8a225340852))
- **replication**: リーダー/フォロワーのランダム差分テストを追加 (task121-1) ([a2dba4d](https://github.com/Moto1829/maharit-db/commit/a2dba4da046972ee5c41088f7e8c6f1613d32350))
- **server**: 並行読み書きのストレステストとグラフ/索引の不変条件チェックを追加 (task121-2) ([b6939fa](https://github.com/Moto1829/maharit-db/commit/b6939fa663bd0ad52126b0c3818271bcf8d4a78d))

### ベンチマーク

- 大量データ性能確認スクリプト scripts/benchmark.py を追加 ([3f4ed39](https://github.com/Moto1829/maharit-db/commit/3f4ed39218c8b0bee45a2ef6ee493fbb4ff4a93d))
- ベンチマーク結果を Markdown レポートとして自動保存 ([1b6d499](https://github.com/Moto1829/maharit-db/commit/1b6d499197892d7ab6a47ea05a72b6597f1b9d93))
- ベンチ結果を Claude API で自動分析してレポートに追加 ([bacce36](https://github.com/Moto1829/maharit-db/commit/bacce3667433b7e3e7efa28fe4ed8a6c4d9c8b72))

### ビルド / CI

- 既定バイナリを18MB→2.62MBに削減（IPADIC辞書をオプション化） ([df64746](https://github.com/Moto1829/maharit-db/commit/df64746e1e2f15776cfee44195a1d37c2fd06928))
- fmt/clippy・全テスト・E2E を実行する GitHub Actions を追加 (task77) ([e8729c6](https://github.com/Moto1829/maharit-db/commit/e8729c6e8b83bce594563ffe2062bc477f885afa))
- Rust ツールチェーンを 1.93.0 に固定し、soak を並列ジョブに分離 (task77) ([81b6d85](https://github.com/Moto1829/maharit-db/commit/81b6d857942032f6b3d493f898dbc2672dde059b))

### その他

- バージョニング・タグ規則を rules として追加 ([c71a981](https://github.com/Moto1829/maharit-db/commit/c71a981dbdfe09e8a99a6cd0aeb846214f2b7f8b))
- .DS_Store を untrack して .gitignore に追加 ([97d0923](https://github.com/Moto1829/maharit-db/commit/97d0923b135be056cabb2c45ab4f5d2a2d4c9890))
- **bench**: baseline を現環境・最新コードで更新 ([6c0f6c7](https://github.com/Moto1829/maharit-db/commit/6c0f6c73075c26b27b5cd910171feafcf8143000))
- **deps**: workspace tokio を full から実使用featureに絞る(task111) ([764df1e](https://github.com/Moto1829/maharit-db/commit/764df1ef5674c2ab793eab252d6a98a9c55d30da))
- Rust ツールチェーンを 1.98.1 に更新 ([b89e2a7](https://github.com/Moto1829/maharit-db/commit/b89e2a7300b7fe43bfe1423be5e40c342e21bd8e))

[Unreleased]: https://github.com/Moto1829/maharit-db/compare/v0.1.0...HEAD

## [0.1.0] - 2026-03-07

- 初回公開リリース（Conventional Commits 導入前のため個別項目なし）



