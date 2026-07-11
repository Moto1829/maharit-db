# query-core/93: Bindings のクローン削減（変数名の Arc<str> インターン）

## 背景
`type Bindings = HashMap<String, BindingValue>`（executor.rs）を候補ノードごとに
`clone` している。多段トラバーサル/JOIN で bindings を複製する際、キー文字列を
毎回再確保するコストがある。変数名を `Arc<str>` にインターンすれば、複製が
参照カウントのインクリメントで済む。

## 事前調査済みの所見（重要）
- 型エイリアスを `HashMap<Arc<str>, BindingValue>` に変えると **約 65 箇所の
  コンパイル修正**が必要（`.get(&String)` → `.as_str()`、`.insert(String)` → `Arc::from`、
  イテレーションの `&Arc<str>` 扱いなど）。すべて機械的だがコンパイラ誘導での修正。
- **効果は多段トラバーサル/JOIN での binding 複製に集中**し、count/scan 経路（入力
  binding が空でキー 1 個を insert するだけ）ではほぼ効かない。
  → 現ベンチのボトルネックには表れにくい高チャーン変更。以前ユーザー判断で保留した経緯あり。

## やること（実施する場合）
- `Bindings` のキーを `Arc<str>` 化し、全 insert/get/iter 箇所を修正。
- 変数名インターナー（`Arc<str>` の再利用）の導入も検討。
- 既存 508 query テストで正当性を担保、ベンチで多段トラバーサル/JOIN の改善を確認。

## 優先度 / 規模
- 低〜中（効果は特定ワークロード限定）。**高チャーン・要慎重検証**。

## ステータス
未着手（バックログ、保留経緯あり）
