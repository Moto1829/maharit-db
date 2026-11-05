// Tabulator によるテーブル描画
//
// 列名に "." が含まれる Cypher の RETURN 結果に対応するため、
// nestedFieldSeparator: false を指定して文字列キーとして扱う。

import { escapeHtml } from "./util.js";

let tabulator = null;

export function render({ columns, rows }, host) {
  host.innerHTML = "";
  if (!rows.length) {
    host.innerHTML = '<div class="empty">0 rows</div>';
    tabulator = null;
    return;
  }
  const tabCols = columns.map((c) => ({
    title: c,
    field: c,
    headerFilter: "input",
    formatter: (cell) => {
      const v = cell.getValue();
      if (v === null || v === undefined) {
        return '<span style="color:#666">null</span>';
      }
      // Lists (e.g. collect()) and maps are shown as JSON, not "a,b".
      return escapeHtml(typeof v === "object" ? JSON.stringify(v) : String(v));
    },
  }));
  tabulator = new Tabulator(host, {
    data: rows,
    columns: tabCols,
    nestedFieldSeparator: false,
    layout: "fitDataStretch",
    pagination: true,
    paginationSize: 50,
    paginationSizeSelector: [25, 50, 100, 250],
    height: "100%",
    movableColumns: true,
    reactiveData: false,
  });
}

export function renderError(host, msg) {
  host.innerHTML = `<div class="error-box">${escapeHtml(msg)}</div>`;
  tabulator = null;
}

export function redraw() {
  if (tabulator) tabulator.redraw(true);
}
