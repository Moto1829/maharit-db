# Task 99: maharit-viz Graph タブで要素クリック時にプロパティを表示

## 背景・目的

Task 82 で maharit-viz の Graph タブ（cytoscape.js）を実装したが、
現状は描画のみで、ノード／エッジをクリックしても何も起こらない。
ユーザーが「このノード／エッジはどんなプロパティを持っているのか」を
確認できる手段がなく、グラフを「見る」だけで「探る」ことができない。

Table タブと併用すれば値は確認できるが、グラフ可視化の本来の体験を
損ねている。

## 実装内容

### フロントエンド (`crates/maharit-viz/assets/index.html`)

#### グラフ要素にプロパティを保持

現状ノードには `{ id, label, group }`、エッジには `{ id, source, target, label }`
しか持たせていない。クリック時に表示するため、各要素に **その元行のすべての
`<prefix>.*` プロパティ** を含めるよう拡張する。

```js
nodes.set(nodeId, {
  data: {
    id: nodeId,
    label,
    group: g,
    kind: "node",
    properties: { id, name, age, city, ... },  // <g>.* を集約
  }
});

edges.push({
  data: {
    id: ...,
    source: ...,
    target: ...,
    label: edgeLabel,
    kind: "edge",
    properties: { type, since, role, ... },    // r.* / rel.* を集約
  }
});
```

#### クリックイベントとサイドパネル

cytoscape の `tap` イベントを listen し、フローティングまたはサイドの
詳細パネルにプロパティを key-value 表示する。

- ノードクリック → ノードのプロパティ + group + id をパネル表示
- エッジクリック → エッジのプロパティ + source/target/label を表示
- 背景クリック（`evt.target === cy`） → パネルを閉じる
- パネルの閉じる(✕)ボタンでも閉じる
- Esc キーでも閉じる

#### UI 設計

- グラフキャンバスの**右側にオーバーレイ表示**（absolute positioned）
- 幅: 280px 前後、最大高さ: グラフ表示領域の 80% （スクロール可能）
- ダークテーマ準拠
- key は muted color、value は通常 color、null/undefined は薄色で「null」表示
- 長い文字列は折り返し

### バックエンド

変更なし。既存の `/api/query` レスポンスをそのまま使う。

## 検証

- Docker 再デプロイ + 投入済みデータで動作確認:
  - `MATCH (n:Person) RETURN n.id, n.name, n.age, n.city` → ノードクリック
  - `MATCH (a)-[r:KNOWS]->(b) RETURN a.id, a.name, b.id, b.name` → ノード/背景クリック
  - `MATCH (a)-[r:WORKS_AT]->(b) RETURN a.id, a.name, r.role, b.id, b.name` → エッジクリック
- 背景クリック / ✕ ボタン / Esc キーで閉じることの確認

## 優先度

MEDIUM（Web UI の本来の体験向上）

## 関連ファイル

- `crates/maharit-viz/assets/index.html` (renderGraph / cytoscape セットアップ / CSS)

## 解決済み (2026-06-14)

### 実装内容

`crates/maharit-viz/assets/index.html` のみ修正:

1. **CSS**: `#graph-detail` 詳細パネル（右上オーバーレイ、幅 300px、最大高さ
   グラフ領域の calc(100% - 24px)、スクロール可能、ダークテーマ準拠）
2. **マークアップ**: Graph タブ内に `<aside id="graph-detail">` を追加
   （ヘッダにバッジ NODE/EDGE + タイトル + ✕ ボタン、本文に詳細リスト）
3. **`renderGraph` 拡張**:
   - 各ノードに `kind: "node"` と `properties: { ... }` を保持
     （`<g>.*` 列を集約）
   - 各エッジに `kind: "edge"` と `properties: { ... }` を保持
     （ノードグループ以外の prefix 列、典型的には `r.*` を集約）
4. **ヘルパー追加**: `collectPrefixedProps()` / `collectEdgePrefixes()`
5. **イベントハンドラ**:
   - cytoscape の `tap` イベント（node/edge/背景）
   - ✕ ボタンクリック
   - Esc キー
6. **表示ロジック** (`showDetail`/`hideDetail`):
   - Node/Edge セクション（id / group or source・target / label）
   - Properties セクション（キーをソートして表示、null は薄色イタリック）

### 反映方法

`docker cp ./crates/maharit-viz/assets/index.html maharit-db-viz:/app/assets/index.html`
で hot-patch（Rust バイナリ再ビルド不要）。再デプロイ時には Docker イメージに
含めるためそのまま `docker compose build` で反映される。
