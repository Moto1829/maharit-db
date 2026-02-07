# Cypher and Aura

Source: https://neo4j.com/docs/cypher-manual/current/introduction/cypher-aura/

## Aura概要
- AuraはNeo4jのフルマネージドクラウドサービス。
- **AuraDB**: アプリケーション開発向けグラフDBサービス。
- **AuraDS**: Graph Data Science向けサービス。

### AuraDBのティア
- AuraDB Free
- AuraDB Professional
- AuraDB Business Critical
- AuraDB Virtual Dedicated Cloud

### AuraDSのティア
- Graph Data Science Community
- Graph Data Science Enterprise
- AuraDS Professional
- AuraDS Enterprise

## Aura上のCypher利用
- ほとんどのCypher機能は全ティアで利用可能。
- ただし、**データベースの作成/変更/削除**や**サーバーの変更/削除**はAura上で不可。
- 一部の管理/ロールベース機能は**Business Critical**と**Virtual Dedicated Cloud**のみで提供。

## AuraとCheat Sheet
- Auraの各ティアに対応したCheat Sheetが用意され、利用可能な機能のみ表示される。
- https://neo4j.com/docs/cypher-cheat-sheet/25/all からティア/バージョンを切替可能。
