import type { RateLimits, RateWindow, SessionState, Snapshot, Status } from "./types";

const MAX_ROWS = 3;
// statusLine はアシスタントの応答ごとに走るので、これより古い値は手元の作業が
// 止まっている間に実際の値から離れている可能性が高い。
const RATE_STALE_MS = 30 * 60 * 1000;

const FALLBACK_LINE: Record<Status, string> = {
  idle: "",
  working: "考え中…",
  waiting: "確認待ち",
  done: "完了",
  error: "エラー",
};

export function renderPanel(
  panel: HTMLElement,
  rows: HTMLElement,
  limits: HTMLElement,
  snapshot: Snapshot | null,
  showRows: boolean,
  now: number,
): void {
  const active = snapshot?.sessions.filter((s) => s.status !== "idle") ?? [];
  const limitText = snapshot?.rate_limits ? renderLimits(limits, snapshot.rate_limits, now) : false;

  rows.replaceChildren(...active.slice(0, MAX_ROWS).map(renderRow));
  if (active.length > MAX_ROWS) {
    const more = el("div", "more", `ほか ${active.length - MAX_ROWS} 件`);
    rows.append(more);
  }
  rows.hidden = active.length === 0;
  limits.hidden = !limitText;
  panel.hidden = !showRows || (active.length === 0 && !limitText);
}

function renderRow(s: SessionState): HTMLElement {
  const row = el("div", "row");
  row.title = s.cwd ?? s.session_id;
  row.append(
    el("span", `dot ${s.status}`),
    el("span", "folder", folderName(s.cwd) || s.session_id.slice(0, 8)),
    el("span", "line", s.line ?? FALLBACK_LINE[s.status]),
    renderContext(s),
  );
  return row;
}

// 使用率が取れないときは、要件どおりトークン数で代用する。
function renderContext(s: SessionState): HTMLElement {
  const ctx = s.context;
  if (ctx?.used_percentage != null) {
    const pct = Math.max(0, Math.min(100, ctx.used_percentage));
    const bar = el("span", "ctx");
    bar.title = `コンテキスト ${Math.round(pct)}%`;
    const fill = el("i", pct >= 80 ? "high" : "");
    fill.style.width = `${pct}%`;
    bar.append(fill);
    return bar;
  }
  if (ctx?.total_input_tokens != null) {
    return el("span", "ctx-tokens", formatTokens(ctx.total_input_tokens));
  }
  return el("span", "ctx-none");
}

function renderLimits(container: HTMLElement, rl: RateLimits, now: number): boolean {
  const parts: string[] = [];
  const add = (label: string, w: RateWindow | null) => {
    // リセット時刻を過ぎた窓の値はもう意味を持たないので出さない。
    if (!w || (w.resets_at != null && w.resets_at * 1000 <= now)) return;
    parts.push(`${label} ${Math.round(w.used_percentage)}%`);
  };
  add("5h", rl.five_hour);
  add("7d", rl.seven_day);
  if (parts.length === 0) return false;
  container.replaceChildren(
    el("span", "limit-values", parts.join(" · ")),
    el("span", "limit-time", `${formatClock(rl.updated_at)} 更新`),
  );
  container.classList.toggle("stale", now - rl.updated_at > RATE_STALE_MS);
  return true;
}

function folderName(cwd: string | null): string {
  if (!cwd) return "";
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? cwd;
}

function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k` : String(n);
}

function formatClock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
