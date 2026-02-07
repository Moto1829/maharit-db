# Cypher and Neo4j

Source: https://neo4j.com/docs/cypher-manual/current/introduction/cypher-neo4j/

## エディション差分
Neo4jはEnterprise EditionとCommunity Editionに分かれ、Cypherはほぼ同一だが次の差分がある。

- **マルチデータベース**
	- Enterprise: system DB + 任意数のユーザーDB
	- Community: system DB + 1ユーザーDBのみ
- **ロールベースセキュリティ**
	- Enterprise: ユーザー/ロール/権限管理、サブグラフアクセス制御
	- Community: 複数ユーザー管理はあるが全ユーザーがフルアクセス
- **制約**
	- Enterprise: ノード/リレーションシップの存在・型・一意・キー制約すべて
	- Community: プロパティ一意制約のみ
- **VECTORプロパティ**
	- Enterprise: ブロックフォーマットDBでVECTORプロパティを保存可能
	- Community: VECTORプロパティ保存不可
- **ランタイム**
	- Enterprise: slotted, pipelined(デフォルト), parallel
	- Community: slottedのみ

## 主要用語
- **DBMS**: 複数データベース/グラフを管理するNeo4j管理システム。クライアントはDBMSに接続しセッションを開く。
- **Graph**: DB内のデータモデル。通常は1DB=1Graphだが、コンポジットDBでは複数グラフが可能。
- **Database**: ディスク/メモリ上にデータを保持するストレージ単位。

## 組み込みデータベース
- すべてのNeo4jサーバーに `system` DBが存在。DBMSメタデータとセキュリティを格納し、グラフクエリは実行不可。
- 初期状態で `system` と `neo4j` の2つが存在。
- 管理コマンドは `system` DB上で実行される。ユーザーDBで実行すると `system` へルーティングされる。

## トランザクション
- **全Cypherクエリはトランザクション内で実行**。
- 更新はコミットまでメモリ上に保持。エラー時は自動ロールバック。
- 大量更新はトランザクションが大きくなるためメモリ消費が増える。

### 明示/暗黙トランザクション
- **明示トランザクション**
	- ユーザーが開始/コミット/ロールバック
	- 複数クエリを順に実行可能
- **暗黙トランザクション**
	- 単一クエリごとに自動開始/コミット
- `CALL { ... } IN TRANSACTIONS` のようにクエリ内でトランザクション分割するものは暗黙モードのみ。

### DBMSトランザクション
- DBMS接続で開始されるトランザクションは**DBMSレベル**。
- DBMSレベル内で、実際のDBに対するトランザクションが開始される。
- 制約:
	- 1つのDBにのみ書き込み可能
	- 同一トランザクション内のクエリは同一ステートメント種別か、スキーマ変更+読み取りの組合せのみ
		- 管理コマンド
		- スキーマ変更（インデックス/制約）
		- 書き込み
		- 読み取り

### ACID
- Atomicity/Consistency/Isolation/Durability を満たす。
