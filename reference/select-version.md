# Select Cypher version

Source: https://neo4j.com/docs/cypher-manual/current/queries/select-version/

## 版の位置づけ
- Neo4j 2025.06でCypher 25が導入。Cypher 5をクローンとして開始し、新機能・更新・削除が加えられる。
- 2025.06以降の新機能はCypher 25にのみ追加。Cypher 5は凍結（性能改善やバグ修正はあり得る）。
- Cypher 5のサポートは将来的に終了予定（少なくとも2回のLTSサイクル後）。

## データベースのデフォルト言語
- 2025.06以降に作成/移行されたDBでも、既定はCypher 5（`db.query.default_language`が`CYPHER_25`でない限り）。
- **CREATE DATABASE**時に `DEFAULT LANGUAGE <language version>` を指定して既定言語を決められる。
- **ALTER DATABASE**で既存DBの既定言語を変更可能。
- 既定言語は`CYPHER 25`または`CYPHER 5`。

## 個別クエリの言語指定
- クエリ冒頭に `CYPHER <version>` を付けて実行言語を指定。
- DBの既定言語を上書きする。
- クエリオプション（例: runtime指定）と併用可能。

## Cypher 5 → 25 への移行注意
- Cypher 25では `SET n = r` のようにNODE/RELATIONSHIPをMAPとして扱う書き方が禁止。
	- `properties()`関数でMAP化して `SET n = properties(r)` のように書き換える。

## プロシージャ/関数との関係
- プロシージャ/関数（APOC含む）は言語バージョンに依存。
- 例: APOC 2025.06では `apoc.create.uuids()` がCypher 25では利用不可だが、`CYPHER 5`指定なら使用可能。
