// 認証フロー: ログイン画面の表示・ログイン処理・ロール反映・ログアウト

import * as api from "./api.js";

let role = null;

/**
 * 初期化: /api/info の auth_enabled に応じて UI を切り替える。
 *
 * @param {Object} els  認証関連の DOM 参照
 * @param {() => void} onAuthenticated  認証済み状態に遷移したときのコールバック
 */
export async function init(els, onAuthenticated) {
  const info = await api.info().catch(() => null);
  if (!info?.auth_enabled) {
    // 認証無効: ログイン画面は出さず、ログアウト/バッジも非表示のまま
    onAuthenticated(null);
    return;
  }

  // 認証必要: クエリを 1 回叩いてセッション有効か確認する
  const probe = await api.query("RETURN 1");
  if (probe.ok) {
    showAuthenticated(els);
    onAuthenticated(role);
    return;
  }
  showLogin(els, onAuthenticated);
}

function showLogin(els, onAuthenticated) {
  els.loginOverlay.hidden = false;
  els.loginError.textContent = "";
  els.loginUsername.focus();

  els.loginForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    els.loginError.textContent = "";
    const username = els.loginUsername.value.trim();
    const password = els.loginPassword.value;
    if (!username || !password) return;
    const result = await api.login(username, password);
    if (!result.ok) {
      els.loginError.textContent = result.error || "ログインに失敗しました";
      return;
    }
    role = result.role;
    els.loginOverlay.hidden = true;
    showAuthenticated(els);
    onAuthenticated(role);
  });
}

function showAuthenticated(els) {
  if (role) {
    els.roleBadge.textContent = role;
    els.roleBadge.className = `role-badge ${role}`;
    els.roleBadge.hidden = false;
  }
  els.logoutButton.hidden = false;
  els.logoutButton.addEventListener("click", async () => {
    await api.logout();
    location.reload();
  });
}

/** 401 を検知したらログイン画面に戻す。 */
export function handleUnauthorized() {
  location.reload();
}

export function currentRole() {
  return role;
}

/** クエリが書き込み系かどうかをラフに判定する（UI 上の警告用、実エラーは server が返す）。 */
const WRITE_PATTERN = /\b(CREATE|MERGE|DELETE|SET|REMOVE|DROP|ALTER|FOREACH)\b/i;
export function isWriteQuery(q) {
  return WRITE_PATTERN.test(q);
}
