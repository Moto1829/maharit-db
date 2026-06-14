# Task 105: maharit-viz フロントエンド JS をモジュール化

## 背景

`crates/maharit-viz/assets/index.html` の `<script>` 内 JS は、**1 つのトップ
レベルスクリプトに 300+ 行**が詰め込まれている。Task 82 (axum + クエリ実行) →
Task 99 (グラフ要素詳細パネル) → Task 100 (履歴ドロップダウン) で機能が積み
重なるたびにグローバル変数とトップレベル関数が増え、関心の分離ができていない。

具体的な問題:

- グローバル変数: `tabulator`, `cy`, `lastResponse`, `detailPanel`,
  `historyButton`, `historyDropdown`, ... が同じスコープに散在
- 状態と DOM 操作とイベント配線が同じ関数内に混在
  （例: `runQuery` がフェッチ・renderTable・renderGraph・pushHistory を直呼び）
- 履歴 / 詳細 / グラフ / テーブル のロジックが独立しているのに、ファイル内で
  順番に並んでいて構造が見えにくい

## 提案

ES Modules + 関心の分離。ファイル分割の前提として **Task 101 と組み合わせる**:

```
assets/
  index.html
  styles.css
  app.js              # エントリポイント (DOM 初期化 + 依存配線)
  modules/
    api.js            # POST /api/query / GET /api/info / GET /api/health
    table_view.js     # Tabulator 描画 (renderTable)
    graph_view.js     # cytoscape + 詳細パネル (renderGraph, showDetail)
    history.js        # localStorage 履歴 (pushHistory, render, clear)
    tabs.js           # タブ切替
    util.js           # escapeHtml, stripQuotes, formatRelativeTime
```

`<script type="module" src="app.js"></script>` でロード。各モジュールが import
で依存を明示する。

### 例

```js
// modules/history.js
const HISTORY_KEY = "maharit-viz:query-history";
const HISTORY_LIMIT = 10;

export function push(query) { ... }
export function load() { ... }
export function clear() { ... }
export function attach({ button, dropdown, list, clearBtn, onSelect }) {
  // ドロップダウン開閉と外側クリックの配線をここで完結
}
```

```js
// app.js
import * as api from "./modules/api.js";
import * as history from "./modules/history.js";
import * as table from "./modules/table_view.js";
import * as graph from "./modules/graph_view.js";
// ...

const elements = { /* DOM 参照を一箇所に集める */ };
history.attach({
  button: elements.historyButton,
  // ...
  onSelect: (q) => { elements.query.value = q; elements.query.focus(); },
});

async function runQuery() {
  const body = await api.query(elements.query.value);
  table.render(body, elements.tableHost);
  graph.render(body, elements.graphCanvas, elements.detailPanel);
  if (!body.error) history.push(elements.query.value);
}
```

## 期待効果

- 各モジュールが 100 行以下に収まる
- 履歴・グラフ・テーブルの仕様変更時に該当モジュールのみ修正
- 将来テストを追加するときに各モジュールを独立して動かせる

## 検証

- 全機能（Table / Graph / Raw JSON / 詳細パネル / 履歴）が回帰なし
- ブラウザ Network タブで個別 JS が 200 で配信されるか確認

## 優先度

LOW（保守性向上、機能追加の頻度が下がってきたら実施）

## 関連タスク

- **Task 101** (assets ファイル分離) と組み合わせて実施するのが効率的

## 関連ファイル

- `crates/maharit-viz/assets/index.html`
