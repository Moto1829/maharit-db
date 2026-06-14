// クエリ履歴 (localStorage)

import { escapeHtml, formatRelativeTime } from "./util.js";

const HISTORY_KEY = "maharit-viz:query-history";
const HISTORY_LIMIT = 10;

let dom = null;

/**
 * 履歴ドロップダウンを DOM に配線する。
 * @param {Object} els
 * @param {HTMLElement} els.button - 開閉トリガー
 * @param {HTMLElement} els.dropdown - ドロップダウン本体
 * @param {HTMLElement} els.list - 履歴リストの描画先
 * @param {HTMLElement} els.clearBtn - 履歴クリアボタン
 * @param {(query: string) => void} els.onSelect - 履歴選択時のコールバック
 */
export function attach({ button, dropdown, list, clearBtn, onSelect }) {
  dom = { button, dropdown, list, clearBtn, onSelect };

  button.addEventListener("click", (e) => {
    e.stopPropagation();
    const open = dropdown.classList.toggle("open");
    if (open) render();
  });

  list.addEventListener("click", (e) => {
    const item = e.target.closest(".history-item");
    if (!item) return;
    const items = load();
    const idx = Number(item.dataset.idx);
    if (Number.isInteger(idx) && items[idx]) {
      onSelect(items[idx].query);
      dropdown.classList.remove("open");
    }
  });

  clearBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    clear();
  });

  document.addEventListener("click", (e) => {
    if (!dropdown.contains(e.target) && e.target !== button) {
      dropdown.classList.remove("open");
    }
  });
}

export function push(q) {
  const trimmed = q.trim();
  if (!trimmed) return;
  const items = load().filter((e) => e.query !== trimmed);
  items.unshift({ query: trimmed, timestamp: Date.now() });
  if (items.length > HISTORY_LIMIT) items.length = HISTORY_LIMIT;
  save(items);
  if (dom && dom.dropdown.classList.contains("open")) render();
}

export function clear() {
  if (!confirm("クエリ履歴をすべて削除しますか？")) return;
  try { localStorage.removeItem(HISTORY_KEY); } catch {}
  render();
}

// ── 内部 ───────────────────────────────────────────────────────────────────

function load() {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    return Array.isArray(arr) ? arr : [];
  } catch {
    return [];
  }
}

function save(items) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(items));
  } catch (e) {
    console.warn("history save failed", e);
  }
}

function previewQuery(q) {
  const oneLine = q.replace(/\s+/g, " ").trim();
  return oneLine.length > 80 ? oneLine.slice(0, 77) + "…" : oneLine;
}

function render() {
  if (!dom) return;
  const items = load();
  dom.clearBtn.disabled = items.length === 0;
  if (items.length === 0) {
    dom.list.innerHTML = '<div class="history-empty">履歴がありません</div>';
    return;
  }
  dom.list.innerHTML = items
    .map(
      (it, idx) =>
        `<div class="history-item" data-idx="${idx}" role="menuitem">` +
        `<span class="history-query">${escapeHtml(previewQuery(it.query))}</span>` +
        `<span class="history-meta">${escapeHtml(formatRelativeTime(it.timestamp))}</span>` +
        `</div>`
    )
    .join("");
}
