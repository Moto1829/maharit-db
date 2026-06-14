// maharit-viz アプリのエントリポイント
//
// 各モジュールに DOM 参照を渡して配線し、Run query のフローを束ねる。

import * as api from "./modules/api.js";
import * as table from "./modules/table_view.js";
import * as graph from "./modules/graph_view.js";
import * as history from "./modules/history.js";
import { attach as attachTabs } from "./modules/tabs.js";

const $ = (id) => document.getElementById(id);

const els = {
  meta: $("meta"),
  status: $("status"),
  run: $("run"),
  query: $("query"),
  rawContent: $("raw-content"),
  tableHost: $("table-host"),
  graphCanvas: $("graph-canvas"),
  tablePanel: $("table-panel"),
  graphPanel: $("graph-panel"),
  rawPanel: $("raw-panel"),
  // 詳細パネル
  detailPanel: $("graph-detail"),
  detailKind: $("detail-kind"),
  detailTitle: $("detail-title"),
  detailBody: $("detail-body"),
  detailClose: $("detail-close"),
  // 履歴
  historyButton: $("history-button"),
  historyDropdown: $("history-dropdown"),
  historyList: $("history-list"),
  historyClear: $("history-clear"),
};

// ── サーバー情報 ───────────────────────────────────────────────────────────
api.info()
  .then((info) => {
    els.meta.textContent = `server: ${info.server_addr}  ·  viz: v${info.version}`;
  })
  .catch(() => {
    els.meta.textContent = "server: (unknown)";
  });

// ── タブ切替 ──────────────────────────────────────────────────────────────
attachTabs({
  tabs: document.querySelectorAll(".tab"),
  panels: {
    table: els.tablePanel,
    graph: els.graphPanel,
    raw: els.rawPanel,
  },
  onSwitch: (name) => {
    if (name === "graph") graph.resize();
    if (name === "table") table.redraw();
  },
});

// ── グラフ詳細パネルの初期化 ─────────────────────────────────────────────
graph.init({
  panel: els.detailPanel,
  kind: els.detailKind,
  title: els.detailTitle,
  body: els.detailBody,
  closeBtn: els.detailClose,
});

// ── 履歴ドロップダウン ────────────────────────────────────────────────────
history.attach({
  button: els.historyButton,
  dropdown: els.historyDropdown,
  list: els.historyList,
  clearBtn: els.historyClear,
  onSelect: (q) => {
    els.query.value = q;
    els.query.focus();
  },
});

// ── 実行フロー ────────────────────────────────────────────────────────────
els.run.addEventListener("click", runQuery);
els.query.addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key === "Enter") runQuery();
});

async function runQuery() {
  const q = els.query.value.trim();
  if (!q) return;
  els.run.disabled = true;
  els.status.textContent = "running…";
  els.status.className = "status";
  try {
    const resp = await api.query(q);
    const rawJson = resp.ok
      ? { columns: resp.columns, rows: resp.rows, elapsed_ms: resp.elapsed_ms }
      : (resp.body ?? { error: resp.error });
    els.rawContent.textContent = JSON.stringify(rawJson, null, 2);
    if (!resp.ok) {
      table.renderError(els.tableHost, resp.error);
      graph.clearOnError(els.graphCanvas);
      els.status.className = "status err";
      els.status.textContent = "error";
    } else {
      table.render(resp, els.tableHost);
      graph.render(resp, els.graphCanvas);
      els.status.className = "status ok";
      els.status.textContent = `${resp.rows.length} rows · ${resp.elapsed_ms} ms`;
      history.push(q);
    }
  } catch (e) {
    table.renderError(els.tableHost, String(e));
    graph.clearOnError(els.graphCanvas);
    els.status.className = "status err";
    els.status.textContent = "network error";
  } finally {
    els.run.disabled = false;
  }
}
