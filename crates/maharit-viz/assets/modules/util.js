// 共通ユーティリティ

export function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** maharit-server が返す文字列値は `"alice"` のようにダブルクォートで
 * 包まれているため、表示用に外側のクォートを剥がす。 */
export function stripQuotes(v) {
  if (v === null || v === undefined) return v;
  const s = String(v);
  if (s.length >= 2 && s.startsWith('"') && s.endsWith('"')) {
    return s.slice(1, -1);
  }
  return s;
}

/** Unix ms タイムスタンプを「3 分前」形式の相対表記に変換する。 */
export function formatRelativeTime(ts) {
  const diff = Date.now() - ts;
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s} 秒前`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} 分前`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} 時間前`;
  const d = Math.floor(h / 24);
  return `${d} 日前`;
}
