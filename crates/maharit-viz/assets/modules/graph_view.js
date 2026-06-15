// cytoscape.js によるグラフ描画 + 要素クリック時の詳細パネル制御
//
// Task 107 で実装した改善:
// - A. グループ別カラーリング + 凡例
// - B. レイアウト fcose (cose より重なり/交差が少ない)
// - C. ホバー時の隣接ハイライト (focus / dim)
// - F. ズーム/フィット コントロール (+ / − / ⛶)

import { escapeHtml, stripQuotes } from "./util.js";

let cy = null;
let detailEls = null;
let chromeEls = null;

/**
 * カテゴリカルなカラーパレット。グループに登場順で割り当てる。
 * 色覚多様性も考慮して彩度差をつけた 8 色。
 */
const PALETTE = [
  "#5aa9ff", // 青
  "#b489ff", // 紫
  "#51cf66", // 緑
  "#ffb14a", // 橙
  "#ff6b6b", // 赤
  "#4cd4d4", // 水色
  "#ffd166", // 黄
  "#f06595", // 桃
];

let fcoseRegistered = false;
function ensureFcose() {
  if (fcoseRegistered) return;
  if (typeof window !== "undefined" && window.cytoscapeFcose) {
    cytoscape.use(window.cytoscapeFcose);
    fcoseRegistered = true;
  }
}

export function init({ panel, kind, title, body, closeBtn, legend, zoom }) {
  detailEls = { panel, kind, title, body, closeBtn };
  chromeEls = { legend, zoom };
  closeBtn.addEventListener("click", hideDetail);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") hideDetail();
  });
  if (zoom) {
    zoom.zoomIn.addEventListener("click", () => cy && cy.zoom(cy.zoom() * 1.25));
    zoom.zoomOut.addEventListener("click", () => cy && cy.zoom(cy.zoom() / 1.25));
    zoom.zoomFit.addEventListener("click", () => cy && cy.fit(undefined, 30));
  }
}

export function render({ columns, rows }, canvas) {
  canvas.innerHTML = "";
  if (cy) { cy.destroy(); cy = null; }
  hideDetail();
  hideChrome();

  const groups = detectNodeGroups(columns);
  if (groups.length === 0 || !rows.length) {
    canvas.innerHTML =
      '<div class="empty">グラフ表示には "&lt;prefix&gt;.id" を含むクエリが必要です。<br>' +
      '例: MATCH (n)-[r]-&gt;(m) RETURN n.id, n.name, r.type, m.id, m.name</div>';
    return;
  }

  // グループ → 色のマッピング（登場順）
  const colorByGroup = {};
  groups.forEach((g, i) => (colorByGroup[g] = PALETTE[i % PALETTE.length]));

  const nodes = new Map();
  const edges = [];
  const edgePrefixes = collectEdgePrefixes(columns, groups);

  for (const row of rows) {
    const present = [];
    for (const g of groups) {
      const id = stripQuotes(row[`${g}.id`]);
      if (id === undefined || id === null || id === "") continue;
      const nodeId = `${g}:${id}`;
      present.push({ group: g, id, nodeId });
      if (!nodes.has(nodeId)) {
        const labelCandidates = ["name", "title", "label"];
        let label = id;
        for (const k of labelCandidates) {
          const v = stripQuotes(row[`${g}.${k}`]);
          if (v) { label = v; break; }
        }
        nodes.set(nodeId, {
          data: {
            id: nodeId,
            label: `${label}`,
            group: g,
            color: colorByGroup[g],
            kind: "node",
            properties: collectPrefixedProps(row, `${g}.`),
          },
        });
      }
    }
    if (present.length >= 2) {
      const edgeLabel = pickEdgeLabel(row, columns);
      const edgeProps = {};
      for (const p of edgePrefixes) {
        Object.assign(edgeProps, collectPrefixedProps(row, `${p}.`));
      }
      for (let i = 0; i < present.length - 1; i++) {
        const a = present[i], b = present[i + 1];
        edges.push({
          data: {
            id: `${a.nodeId}->${b.nodeId}:${edges.length}`,
            source: a.nodeId,
            target: b.nodeId,
            label: edgeLabel || "",
            kind: "edge",
            properties: edgeProps,
          },
        });
      }
    }
  }

  if (nodes.size === 0) {
    canvas.innerHTML =
      '<div class="empty">グラフデータが見つかりませんでした (id が空または null)</div>';
    return;
  }

  ensureFcore();

  cy = cytoscape({
    container: canvas,
    elements: [...nodes.values(), ...edges],
    minZoom: 0.2,
    maxZoom: 3,
    wheelSensitivity: 0.3,
    style: [
      {
        selector: "node",
        style: {
          "background-color": "data(color)",
          label: "data(label)",
          color: "#e6e8ec",
          "font-size": "11px",
          "text-valign": "center",
          "text-halign": "center",
          "text-outline-color": "#0f1115",
          "text-outline-width": 2,
          width: 36,
          height: 36,
          "border-color": "#0f1115",
          "border-width": 1.5,
          "transition-property": "opacity, border-width, border-color, width, height",
          "transition-duration": "120ms",
        },
      },
      {
        selector: "edge",
        style: {
          width: 2,
          "line-color": "#3d4757",
          "target-arrow-color": "#3d4757",
          "target-arrow-shape": "triangle",
          "curve-style": "bezier",
          label: "data(label)",
          "font-size": "10px",
          color: "#8a93a6",
          "text-rotation": "autorotate",
          "text-margin-y": -8,
          "transition-property": "opacity, line-color, target-arrow-color, width",
          "transition-duration": "120ms",
        },
      },
      // ── ホバーハイライト用 (C) ────────────────────────────────────
      { selector: ".dim", style: { opacity: 0.18 } },
      {
        selector: "node.focus",
        style: {
          "border-width": 3,
          "border-color": "#ffffff",
          width: 42,
          height: 42,
        },
      },
      {
        selector: "edge.focus",
        style: {
          width: 3,
          "line-color": "#e6e8ec",
          "target-arrow-color": "#e6e8ec",
          color: "#e6e8ec",
        },
      },
    ],
    layout: fcoseLayoutConfig(nodes.size),
  });

  // ── ホバーハイライト (C) ──────────────────────────────────────────
  cy.on("mouseover", "node", (e) => highlightNeighborhood(e.target));
  cy.on("mouseout", "node", clearHighlight);
  cy.on("mouseover", "edge", (e) => highlightEdge(e.target));
  cy.on("mouseout", "edge", clearHighlight);

  // ── クリックで詳細パネル ────────────────────────────────────────
  cy.on("tap", "node", (evt) => showDetail(evt.target.data()));
  cy.on("tap", "edge", (evt) => showDetail(evt.target.data()));
  cy.on("tap", (evt) => { if (evt.target === cy) hideDetail(); });
  // ダブルクリックでフィット (F)
  cy.on("dbltap", (evt) => {
    if (evt.target === cy) cy.fit(undefined, 30);
  });

  // ── 凡例 (A) / ズームコントロール (F) を表示 ───────────────────
  renderLegend(colorByGroup);
  if (chromeEls && chromeEls.zoom) chromeEls.zoom.root.hidden = false;
}

export function clearOnError(canvas) {
  canvas.innerHTML = "";
  if (cy) { cy.destroy(); cy = null; }
  hideDetail();
  hideChrome();
}

export function resize() {
  if (cy) cy.resize();
}

// ── レイアウト設定 ─────────────────────────────────────────────────────────

function fcoseLayoutConfig(nodeCount) {
  // fcose が読み込まれていれば使う。読み込み失敗時は cose にフォールバック。
  const hasFcose = typeof window !== "undefined" && window.cytoscapeFcose;
  if (!hasFcose) {
    return { name: "cose", animate: false, fit: true, padding: 30 };
  }
  return {
    name: "fcose",
    quality: "default",
    animate: false,
    fit: true,
    padding: 30,
    nodeRepulsion: 5500,
    idealEdgeLength: nodeCount > 30 ? 80 : 110,
    edgeElasticity: 0.45,
    gravity: 0.25,
    randomize: true,
  };
}

// 互換ガード: fcose 登録は ensureFcose のタイポを後方互換でカバー
function ensureFcore() { ensureFcose(); }

// ── ハイライト処理 (C) ─────────────────────────────────────────────────────

function highlightNeighborhood(node) {
  if (!cy) return;
  cy.elements().addClass("dim");
  node.removeClass("dim").addClass("focus");
  const neighborhood = node.neighborhood();
  neighborhood.removeClass("dim").addClass("focus");
}

function highlightEdge(edge) {
  if (!cy) return;
  cy.elements().addClass("dim");
  edge.removeClass("dim").addClass("focus");
  edge.connectedNodes().removeClass("dim").addClass("focus");
}

function clearHighlight() {
  if (!cy) return;
  cy.elements().removeClass("dim focus");
}

// ── 凡例 (A) ───────────────────────────────────────────────────────────────

function renderLegend(colorByGroup) {
  if (!chromeEls || !chromeEls.legend) return;
  const el = chromeEls.legend;
  const entries = Object.entries(colorByGroup);
  if (entries.length === 0) {
    el.hidden = true;
    return;
  }
  el.innerHTML =
    '<span class="legend-title">グループ</span>' +
    entries
      .map(
        ([g, color]) =>
          `<span class="legend-item"><span class="legend-swatch" style="background:${color}"></span>${escapeHtml(g)}</span>`
      )
      .join("");
  el.hidden = false;
}

function hideChrome() {
  if (chromeEls && chromeEls.legend) chromeEls.legend.hidden = true;
  if (chromeEls && chromeEls.zoom) chromeEls.zoom.root.hidden = true;
}

// ── 内部ヘルパー ───────────────────────────────────────────────────────────

function detectNodeGroups(columns) {
  const groups = new Set();
  for (const c of columns) {
    const m = c.match(/^([A-Za-z_][A-Za-z0-9_]*)\.id$/);
    if (m) groups.add(m[1]);
  }
  return [...groups];
}

function collectPrefixedProps(row, prefix) {
  const out = {};
  for (const [k, v] of Object.entries(row)) {
    if (k.startsWith(prefix)) {
      out[k.slice(prefix.length)] = stripQuotes(v);
    }
  }
  return out;
}

function collectEdgePrefixes(columns, nodeGroups) {
  const nodeSet = new Set(nodeGroups);
  const prefixes = new Set();
  for (const c of columns) {
    const m = c.match(/^([A-Za-z_][A-Za-z0-9_]*)\./);
    if (m && !nodeSet.has(m[1])) prefixes.add(m[1]);
  }
  return [...prefixes];
}

function pickEdgeLabel(row, columns) {
  for (const c of columns) {
    if (/\.(type|label|name)$/.test(c) && !/^([A-Za-z_][A-Za-z0-9_]*)\.id$/.test(c)) {
      const v = stripQuotes(row[c]);
      if (v && (c.startsWith("r.") || c.startsWith("rel.") || c.includes("edge"))) {
        return v;
      }
    }
  }
  return "";
}

// ── 詳細パネル ─────────────────────────────────────────────────────────────

function showDetail(data) {
  if (!detailEls) return;
  const { panel, kind, title, body } = detailEls;
  const isEdge = data.kind === "edge";
  kind.textContent = isEdge ? "EDGE" : "NODE";
  kind.classList.toggle("edge", isEdge);
  title.textContent = data.label || data.id || "(unnamed)";

  const sections = [];
  if (isEdge) {
    sections.push({
      title: "Edge",
      rows: [
        ["id", data.id],
        ["source", data.source],
        ["target", data.target],
        ["label", data.label || ""],
      ],
    });
  } else {
    sections.push({
      title: "Node",
      rows: [
        ["id", data.id],
        ["group", data.group || ""],
        ["label", data.label || ""],
      ],
    });
  }

  const props = data.properties || {};
  const propRows = Object.entries(props).sort(([a], [b]) => a.localeCompare(b));
  sections.push({ title: "Properties", rows: propRows });

  body.innerHTML = sections
    .map((s) => {
      const head = `<div class="detail-section">${escapeHtml(s.title)}</div>`;
      if (!s.rows.length) {
        return head + '<div class="detail-empty">(no values)</div>';
      }
      const inner = s.rows
        .map(([k, v]) => {
          const nullish = v === null || v === undefined || v === "";
          const valHtml = nullish
            ? '<span class="detail-value null">null</span>'
            : `<span class="detail-value">${escapeHtml(String(v))}</span>`;
          return `<div class="detail-row"><span class="detail-key">${escapeHtml(
            String(k)
          )}</span>${valHtml}</div>`;
        })
        .join("");
      return head + inner;
    })
    .join("");

  panel.classList.add("open");
  panel.setAttribute("aria-hidden", "false");
}

function hideDetail() {
  if (!detailEls) return;
  detailEls.panel.classList.remove("open");
  detailEls.panel.setAttribute("aria-hidden", "true");
}
