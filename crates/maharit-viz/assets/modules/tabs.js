// タブ切替モジュール
//
// 渡された tabs / panels マップを切り替える。タブ切替時に onSwitch コールバックを呼び、
// 切替後の再描画 (Tabulator.redraw, cy.resize 等) を行う機会を提供する。

/**
 * @param {Object} opts
 * @param {NodeListOf<HTMLElement>} opts.tabs - data-tab 属性を持つタブ要素
 * @param {Record<string, HTMLElement>} opts.panels - tab name → panel 要素のマップ
 * @param {(name: string) => void} [opts.onSwitch] - 切替時のコールバック
 */
export function attach({ tabs, panels, onSwitch }) {
  function switchTo(name) {
    tabs.forEach((t) => t.classList.toggle("active", t.dataset.tab === name));
    for (const [k, el] of Object.entries(panels)) {
      el.classList.toggle("active", k === name);
    }
    onSwitch && onSwitch(name);
  }
  tabs.forEach((tab) => {
    tab.addEventListener("click", () => switchTo(tab.dataset.tab));
  });
  return { switchTo };
}
