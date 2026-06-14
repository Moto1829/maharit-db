// maharit-viz バックエンド API クライアント

export async function info() {
  const r = await fetch("/api/info");
  return r.json();
}

export async function health() {
  const r = await fetch("/api/health");
  return r.text();
}

/** クエリを実行する。失敗時は { ok: false, error, status } 形式で返し、
 * 成功時は { ok: true, ...body } を返す（呼び出し側で if (resp.ok) で判定）。 */
export async function query(cypher) {
  const r = await fetch("/api/query", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query: cypher }),
  });
  const body = await r.json();
  if (!r.ok || body.error) {
    return { ok: false, error: body.error || `HTTP ${r.status}`, status: r.status, body };
  }
  return { ok: true, ...body };
}
