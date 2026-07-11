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

## 対応（完了）
- `type Bindings = HashMap<String, BindingValue>` → `HashMap<Arc<str>, BindingValue>`。
- コンパイラ誘導で **65 箇所**を機械修正:
  - `.get(x)` / `.contains_key(x)` の `&String` 引数 → `x.as_str()`（`Arc<str>: Borrow<str>`）。
  - `.insert(key.clone(), ..)` / リテラルキー → `Arc::from(key.as_str())` / `Arc::from("lit")`。
- 変数名インターナーは導入せず（insert 時に `Arc::from` で確保）。多段パターン/JOIN で
  binding を複製する際、キーが String 再確保ではなく Arc 参照カウント増で済む（clone が浅い）。

## 効果 / 検証
- **正当性**: query 508 テスト + workspace 全16バイナリパス（意味論不変）。
- **性能**: binding 複製が発生する多段トラバーサル/JOIN で確保コスト減。
  単発 scan（insert 1 回・以降複製なし）ではほぼ中立。効果は絶対値で小さく、
  dev マシンのベンチばらつき（TRAV 項目は run 間 ±30〜40%）に埋もれるため
  数値での明確な before/after 提示は困難。アーキテクチャ上の確保削減として実施。
- 追加余地: 変数名の真のインターン（`Arc<str>` 再利用）で insert 側の確保も削減可能（別タスク候補）。

## ステータス
完了
