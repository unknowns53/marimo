import { folderName, formatTokens } from "./format";
import type { RateLimits, RateWindow, SessionState, Snapshot, Status } from "./types";

const MAX_ROWS = 3;
// statusLine はアシスタントの応答ごとに走るので、これより古い値は手元の作業が
// 止まっている間に実際の値から離れている可能性が高い。
const RATE_STALE_MS = 30 * 60 * 1000;

const FALLBACK_SUMMARY: Record<Status, string> = {
  idle: "",
  working: "考え中…",
  waiting: "確認待ち",
  done: "完了",
  error: "エラー",
};

export interface PanelElements {
  panel: HTMLElement;
  rows: HTMLElement;
  limits: HTMLElement;
}

export function renderPanel(
  { panel, rows, limits }: PanelElements,
  snapshot: Snapshot | null,
  showRows: boolean,
  now: number,
  onSelect: (sessionId: string) => void,
): void {
  const active = snapshot?.sessions.filter((s) => s.status !== "idle") ?? [];
  const limitText = snapshot?.rate_limits ? renderLimits(limits, snapshot.rate_limits, now) : false;

  rows.replaceChildren(...active.slice(0, MAX_ROWS).map((s) => renderRow(s, onSelect)));
  if (active.length > MAX_ROWS) {
    rows.append(el("div", "more", `ほか ${active.length - MAX_ROWS} 件`));
  }
  rows.hidden = active.length === 0;
  limits.hidden = !limitText;
  panel.hidden = !showRows || (active.length === 0 && !limitText);
}

function renderRow(s: SessionState, onSelect: (sessionId: string) => void): HTMLElement {
  const row = el("div", "row");
  const summary = s.activity?.summary || FALLBACK_SUMMARY[s.status];
  const detail = s.activity?.detail ?? "";
  row.title = [s.cwd ?? s.session_id, summary, detail].filter(Boolean).join("\n\n");
  row.addEventListener("click", () => onSelect(s.session_id));

  const head = el("div", "row-head");
  head.append(el("span", `dot ${s.status}`), el("span", "folder", folderName(s)), renderContext(s));
  row.append(head, el("div", "row-summary", summary));
  if (detail) row.append(el("div", "row-detail", detail));
  return row;
}

// % が分かるときはバー、上限が分からずトークン数だけのときは数値だけを出し、見分けられるようにする。
export function renderContext(s: SessionState): HTMLElement {
  const ctx = s.context;
  if (ctx?.used_percentage != null) {
    const pct = Math.max(0, Math.min(100, ctx.used_percentage));
    const wrap = el("span", "ctx");
    wrap.title = `コンテキスト ${Math.round(pct)}%`;
    const bar = el("span", "ctx-bar");
    const fill = el("i", pct >= 80 ? "high" : "");
    fill.style.width = `${pct}%`;
    bar.append(fill);
    wrap.append(bar, el("span", "ctx-label", `${Math.round(pct)}%`));
    return wrap;
  }
  if (ctx?.total_input_tokens != null) {
    const tokens = el("span", "ctx-tokens", formatTokens(ctx.total_input_tokens));
    tokens.title = `コンテキスト ${ctx.total_input_tokens.toLocaleString()} トークン（上限が分からないため % は出していません）`;
    return tokens;
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
