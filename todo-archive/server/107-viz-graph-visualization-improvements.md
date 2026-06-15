# Task 107: Graph 描画の見やすさ改善

## 背景

現状の Graph タブ（`crates/maharit-viz/assets/modules/graph_view.js`）は
動作はするが、以下の点で「読みづらい」:

- 全ノードが同じ色（青 `#5aa9ff`）・同じサイズ（36px）で、Person / City /
  Company のような種別の違いが視覚的にわからない
- エッジが全て同じ細い線で、リレーション種別の差も色で区別できない
- レイアウト (`cose`) はノード数が増えるとぐちゃっとなりやすい
- ホバーでの隣接ハイライトがなく、密なグラフだと関係を追いづらい
- ズーム/フィットの操作が分かりにくく、初期表示で見切れることがある
- 凡例（どの色が何のグループか）がない

## 改善案（優先度順）

### A. グループ別カラーリング + 凡例（HIGH）

検出済みの `<prefix>.id` のグループに、カテゴリカルなカラーパレットを
順番に割り当てる。グラフキャンバスの左上などに「a: 青 / b: 紫 / r: 赤」
のような凡例を浮かべる。

```js
const PALETTE = ["#5aa9ff", "#b489ff", "#51cf66", "#ffb14a", "#ff6b6b", ...];
```

凡例はクリックで該当グループのノードをハイライト or 一時非表示にできると
理想的だが、最小実装は表示のみで OK。

### B. レイアウトを fcose / cola に切り替え（HIGH）

`cose` は cytoscape 標準だがクラスタリングが弱い。**`cose-bilkent`** や
**`fcose`** は同じインターフェースで使えてレイアウトが格段にきれい。
CDN 追加だけで導入できる:

```html
<script src="https://unpkg.com/layout-base@2.0.1/layout-base.js"></script>
<script src="https://unpkg.com/cose-base@2.2.0/cose-base.js"></script>
<script src="https://unpkg.com/cytoscape-fcose@2.2.0/cytoscape-fcose.js"></script>
```

`fcose` はノード重なりが少なく、エッジ交差も減る。

### C. ホバー時の隣接ハイライト（HIGH）

cytoscape の `mouseover` / `mouseout` でノード/エッジに `highlighted` クラスを
付け、それ以外を半透明にする CSS を当てる。

```js
cy.on("mouseover", "node", (e) => {
  const node = e.target;
  cy.elements().addClass("dim");
  node.removeClass("dim").addClass("focus");
  node.connectedEdges().removeClass("dim").addClass("focus");
  node.neighborhood("node").removeClass("dim").addClass("focus");
});
```

### D. ノードサイズを次数に反映（MEDIUM）

次数（接続エッジ数）に応じてノードサイズを段階的に変える（例: 24 〜 56px）。
中心的なノードが視覚的にわかりやすくなる。

```js
const degree = node.degree();
const size = 24 + Math.min(degree * 4, 32);
```

### E. エッジラベルの可読性向上（MEDIUM）

- 背景色をつけて読みやすくする
- 長いラベルは省略
- hover で全文表示

### F. ズーム/フィット コントロール（MEDIUM）

右下に「+ / − / ⛶ (フィット)」のミニツールバー。`cy.fit()` / `cy.zoom()` を呼ぶ。
ダブルクリックでもフィット動作にする。

### G. 双方向 / 多重エッジの自動カール（MEDIUM）

同じ source/target ペアの複数エッジを並行カーブで描画する
（`curve-style: bezier` + `control-point-step-size`）。

### H. ラベル表示 ON/OFF トグル（LOW）

ヘッダにチェックボックスを追加して、ノードラベル / エッジラベルの表示を
切り替えられる。ノード数が多いときに有用。

### I. ノード形状の使い分け（LOW）

グループごとに `ellipse` / `roundrectangle` / `hexagon` / `diamond` を割り当て。
色だけでなく形でも判別できるとアクセシビリティが上がる。

## 推奨スコープ（最小実装）

**A + B + C + F** を 1 commit にまとめると最も体感が変わる。
`D` / `E` / `G` は追加コミットで段階的に。`H` / `I` は別タスク化してもよい。

## 検証

- 現在投入されている Person/City/Company データで以下のクエリを試す:

```cypher
MATCH (a)-[]->(b) RETURN a.id, a.name, b.id, b.name LIMIT 50
MATCH (p:Person)-[r:WORKS_AT]->(c:Company)
RETURN p.id, p.name, r.role, c.id, c.name
```

- ノード数 6〜20、エッジ数 5〜30 程度で見やすさを確認
- ホバー時に隣接ノードが浮かび上がり、無関係なノードが薄くなる
- 凡例が左上に表示され、グループと色の対応がわかる

## 関連ファイル

- `crates/maharit-viz/assets/modules/graph_view.js` (描画ロジック)
- `crates/maharit-viz/assets/styles.css` (詳細パネル / レイアウトの CSS)
- `crates/maharit-viz/assets/index.html` (CDN スクリプトの追加)

## 優先度

MEDIUM（機能ではなく UX 改善、すぐに実害があるわけではないが体感差が大きい）

## 解決済み (2026-06-15, A+B+C+F のみ) — commit 6a02c37b

最小スコープ A+B+C+F を 1 コミットで実装。D/E/G/H/I は別タスク化候補として残す。

### 実装内容

- **A**: PALETTE 8 色をグループ登場順で割り当て、左上に凡例
- **B**: cytoscape-fcose を CDN ロードし、cose から差し替え（フォールバック付き）
- **C**: hover で隣接要素を focus、他を dim (opacity 0.18) + 120ms transition
- **F**: 右下に +/−/⛶ ミニツールバー、ダブルクリックでもフィット

### 残課題（次タスク化候補）

- D: ノードサイズを次数に反映
- E: エッジラベルの背景・省略・hover 拡大
- G: 双方向 / 多重エッジの自動カール
- H: ラベル表示 ON/OFF トグル
- I: ノード形状の使い分け（色覚多様性対応）
