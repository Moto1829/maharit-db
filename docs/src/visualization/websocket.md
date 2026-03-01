# WebSocket リアルタイム表示

MaharitDB の `maharit-viz` クレートは WebSocket を使ったリアルタイムのグラフ更新配信をサポートしています。グラフの変更（ノード/エッジの追加・削除・更新）をブラウザにリアルタイムで反映できます。

## WebSocket サーバーの起動

```bash
# WebSocket サーバーを有効にして起動
maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --websocket-port 7690
```

WebSocket エンドポイント: `ws://localhost:7690/graph`

## 配信されるイベント

グラフの変更はすべて WebSocket を通じてブラウザに配信されます。

### ノード作成イベント

```json
{
  "event": "node_created",
  "timestamp": "2024-01-01T10:00:00Z",
  "data": {
    "id": 42,
    "labels": ["Person"],
    "properties": {
      "name": "Alice",
      "age": 30
    }
  }
}
```

### エッジ作成イベント

```json
{
  "event": "edge_created",
  "timestamp": "2024-01-01T10:00:01Z",
  "data": {
    "id": 100,
    "source": 42,
    "target": 43,
    "type": "KNOWS",
    "properties": {
      "since": 2021
    }
  }
}
```

### プロパティ更新イベント

```json
{
  "event": "property_updated",
  "timestamp": "2024-01-01T10:00:02Z",
  "data": {
    "node_id": 42,
    "property": "age",
    "old_value": 30,
    "new_value": 31
  }
}
```

### ノード削除イベント

```json
{
  "event": "node_deleted",
  "timestamp": "2024-01-01T10:00:03Z",
  "data": {
    "id": 42
  }
}
```

## ブラウザ側の実装

### Vanilla JavaScript（D3.js と組み合わせ）

```html
<!DOCTYPE html>
<html>
<head>
  <script src="https://d3js.org/d3.v7.min.js"></script>
</head>
<body>
  <svg id="graph-canvas" width="1200" height="800"></svg>
  <script>
    const ws = new WebSocket('ws://localhost:7690/graph');

    // グラフデータ
    let nodes = [];
    let links = [];

    // D3.js 力学シミュレーション
    const svg = d3.select('#graph-canvas');
    const simulation = d3.forceSimulation(nodes)
      .force('link', d3.forceLink(links).id(d => d.id))
      .force('charge', d3.forceManyBody().strength(-300))
      .force('center', d3.forceCenter(600, 400));

    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);

      switch (msg.event) {
        case 'node_created':
          nodes.push(msg.data);
          updateGraph();
          break;

        case 'edge_created':
          links.push({
            source: msg.data.source,
            target: msg.data.target,
            type: msg.data.type
          });
          updateGraph();
          break;

        case 'node_deleted':
          nodes = nodes.filter(n => n.id !== msg.data.id);
          links = links.filter(l => l.source !== msg.data.id && l.target !== msg.data.id);
          updateGraph();
          break;
      }
    };

    function updateGraph() {
      simulation.nodes(nodes);
      simulation.force('link').links(links);
      simulation.alpha(0.3).restart();

      // ノードの描画
      const node = svg.selectAll('.node')
        .data(nodes, d => d.id)
        .join('circle')
        .attr('class', 'node')
        .attr('r', 15)
        .attr('fill', d => d.labels.includes('Person') ? '#3498DB' : '#2ECC71');

      // エッジの描画
      const link = svg.selectAll('.link')
        .data(links)
        .join('line')
        .attr('class', 'link')
        .attr('stroke', '#999');

      simulation.on('tick', () => {
        link
          .attr('x1', d => d.source.x)
          .attr('y1', d => d.source.y)
          .attr('x2', d => d.target.x)
          .attr('y2', d => d.target.y);

        node
          .attr('cx', d => d.x)
          .attr('cy', d => d.y);
      });
    }
  </script>
</body>
</html>
```

### Rust クライアントからの WebSocket 購読

```rust
use tokio_tungstenite::connect_async;
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (ws_stream, _) = connect_async("ws://localhost:7690/graph").await?;
    let (_, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        let msg = msg?;
        let text = msg.to_text()?;
        let event: serde_json::Value = serde_json::from_str(text)?;

        match event["event"].as_str() {
            Some("node_created") => {
                println!("Node created: {:?}", event["data"]);
            }
            Some("edge_created") => {
                println!("Edge created: {:?}", event["data"]);
            }
            Some("node_deleted") => {
                println!("Node deleted: id={}", event["data"]["id"]);
            }
            _ => {}
        }
    }

    Ok(())
}
```

## フィルタリング

購読するイベントをフィルタリングできます。

```javascript
// 接続時にフィルタを送信
ws.onopen = () => {
  ws.send(JSON.stringify({
    "action": "subscribe",
    "filter": {
      "labels": ["Person", "Company"],  // 特定ラベルのノードのみ
      "events": ["node_created", "edge_created"]  // 特定イベントのみ
    }
  }));
};
```

## スナップショットの取得

初回接続時に現在のグラフ状態を取得します。

```javascript
ws.onopen = () => {
  // 現在の状態のスナップショットをリクエスト
  ws.send(JSON.stringify({"action": "snapshot"}));
};

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.event === 'snapshot') {
    // 全ノードとエッジで初期描画
    nodes = msg.data.nodes;
    links = msg.data.edges;
    renderGraph();
  }
};
```

## パフォーマンスの考慮事項

- 大規模グラフ（数万ノード以上）のリアルタイム表示には、クライアント側でのフィルタリングを推奨します
- 高頻度の更新がある場合はバッチ更新（100ms ごとにまとめて送信）を検討してください
- WebSocket 接続数の上限はサーバー設定で調整できます（`--ws-max-connections`）
