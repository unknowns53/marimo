import { folderName, formatTokens } from "./format";
import { hasContent, type PanelPlan, type PanelView } from "./panelModel";
import type { RateLimits, RateWindow, SessionState, Status } from "./types";

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
  toggle: HTMLElement;
}

const STATUS_LABEL: Record<Status, string> = {
  idle: "待機",
  working: "作業中",
  waiting: "承認待ち",
  done: "完了",
  error: "エラー",
};

export function renderPanel(
  { panel, rows, limits, toggle }: PanelElements,
  view: PanelView,
  rateLimits: RateLimits | null,
  now: number,
  onSelect: (session: SessionState) => void,
): void {
  if (view.mode === "picture") {
    panel.hidden = true;
    return;
  }
  const limitText = rateLimits ? renderLimits(limits, rateLimits, now) : false;
  let children: HTMLElement[] = [];
  if (hasContent(view)) {
    children =
      view.mode === "detail" ? detailChildren(view.plan, onSelect) : [renderCounts(view, onSelect)];
  }
  rows.replaceChildren(...children);
  rows.hidden = children.length === 0;
  limits.hidden = !limitText;
  panel.hidden = children.length === 0 && !limitText;
  // 今の段階を短い文字で示し、押すと詳細と件数だけを行き来する。
  toggle.textContent = view.mode === "detail" ? "詳細" : "件数";
  toggle.title = view.mode === "detail" ? "件数だけの表示に切り替える" : "詳細の表示に切り替える";
  toggle.hidden = children.length === 0;
}

function detailChildren(plan: PanelPlan, onSelect: (session: SessionState) => void): HTMLElement[] {
  const children: HTMLElement[] = plan.rows.map((s) => renderRow(s, onSelect));
  if (plan.moreRows > 0) children.push(el("div", "more", `ほか ${plan.moreRows} 件`));
  return children;
}

// 件数だけの段階は 1 行に畳み、マウスを載せたときに詳細の表示を上へ重ねて見せる。
// 押したときは、最も優先度の高い要対応のセッションへ移動する。
function renderCounts(view: PanelView, onSelect: (session: SessionState) => void): HTMLElement {
  const line = () => {
    const node = el("div", "counts-line");
    view.counts.forEach((c, i) => {
      if (i > 0) node.append(el("span", "counts-sep", "·"));
      const item = el("span", "counts-item");
      item.append(el("span", `dot ${c.status}`), el("span", "", `${STATUS_LABEL[c.status]} ${c.count}`));
      node.append(item);
    });
    return node;
  };
  const wrap = el("div", "counts-group");
  wrap.dataset.expandId = "counts";
  const layer = el("div", "counts-detail hover-layer");
  layer.append(...detailChildren(view.plan, onSelect), line());
  wrap.append(line(), layer);
  const target = view.target;
  if (target) {
    wrap.classList.add("clickable");
    wrap.addEventListener("click", () => onSelect(target));
  }
  return wrap;
}

// 行はふだん 2 段に詰め、マウスを載せたときだけ、要約の全文とコマンドの段を持つ層を行の上へ重ねて
// 広げる。層は行の下端に揃えて上へ伸ばし、パネルの高さを変えないので、立ち絵が上下に動かない。
function renderRow(s: SessionState, onSelect: (session: SessionState) => void): HTMLElement {
  const row = el("div", "row");
  row.dataset.expandId = `row:${s.session_id}`;
  const summary = s.activity?.summary || FALLBACK_SUMMARY[s.status];
  const detail = s.activity?.detail ?? "";
  row.title = [s.cwd ?? s.session_id, summary, detail].filter(Boolean).join("\n\n");
  row.addEventListener("click", (e) => {
    e.stopPropagation();
    onSelect(s);
  });

  const full = el("div", "row-full hover-layer");
  full.append(renderHead(s), el("div", "row-summary full", summary));
  if (detail) full.append(el("div", "row-detail", detail));
  row.append(renderHead(s), el("div", "row-summary", summary), full);
  return row;
}

function renderHead(s: SessionState): HTMLElement {
  const head = el("div", "row-head");
  head.append(el("span", `dot ${s.status}`), el("span", "folder", folderName(s)), renderContext(s));
  return head;
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
