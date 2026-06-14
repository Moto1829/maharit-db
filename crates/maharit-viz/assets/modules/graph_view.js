// cytoscape.js によるグラフ描画 + 要素クリック時の詳細パネル制御

import { escapeHtml, stripQuotes } from "./util.js";

let cy = null;
let detailEls = null;

export function init({ panel, kind, title, body, closeBtn }) {
  detailEls = { panel, kind, title, body, closeBtn };
  closeBtn.addEventListener("click", hideDetail);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") hideDetail();
  });
}

export function render({ columns, rows }, canvas) {
  canvas.innerHTML = "";
  if (cy) { cy.destroy(); cy = null; }
  hideDetail();

  const groups = detectNodeGroups(columns);
  if (groups.length === 0 || !rows.length) {
    canvas.innerHTML =
      '<div class="empty">グラフ表示には "&lt;prefix&gt;.id" を含むクエリが必要です。<br>' +
      '例: MATCH (n)-[r]-&gt;(m) RETURN n.id, n.name, r.type, m.id, m.name</div>';
    return;
  }

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

  cy = cytoscape({
    container: canvas,
    elements: [...nodes.values(), ...edges],
    style: [
      {
        selector: "node",
        style: {
          "background-color": "#5aa9ff",
          label: "data(label)",
          color: "#e6e8ec",
          "font-size": "11px",
          "text-valign": "center",
          "text-halign": "center",
          "text-outline-color": "#0f1115",
          "text-outline-width": 2,
          width: 36,
          height: 36,
        },
      },
      {
        selector: "edge",
        style: {
          width: 2,
          "line-color": "#4a8ad8",
          "target-arrow-color": "#4a8ad8",
          "target-arrow-shape": "triangle",
          "curve-style": "bezier",
          label: "data(label)",
          "font-size": "10px",
          color: "#8a93a6",
          "text-rotation": "autorotate",
          "text-margin-y": -8,
        },
      },
    ],
    layout: { name: "cose", animate: false, fit: true, padding: 30 },
  });

  cy.on("tap", "node", (evt) => showDetail(evt.target.data()));
  cy.on("tap", "edge", (evt) => showDetail(evt.target.data()));
  cy.on("tap", (evt) => { if (evt.target === cy) hideDetail(); });
}

export function clearOnError(canvas) {
  canvas.innerHTML = "";
  if (cy) { cy.destroy(); cy = null; }
  hideDetail();
}

export function resize() {
  if (cy) cy.resize();
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
