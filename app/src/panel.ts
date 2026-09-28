import { sessionKey } from "./acknowledged";
import { folderName, formatAge, formatClock, formatTokens, isCodexScratch } from "./format";
import { limitLine, usageNote, type LimitLine, type UsageNote } from "./limits";
import { hasContent, type PanelPlan, type PanelView } from "./panelModel";
import type { AppIcons, CodexRateLimits, Provider, RateLimits, SessionState, Status, UsageStatus } from "./types";

const FALLBACK_SUMMARY: Record<Status, string> = {
  idle: "",
  working: "考え中…",
  waiting: "確認待ち",
  done: "完了",
  error: "エラー",
};

const EMPTY_LIST_TEXT = "動いているセッションはありません";

export interface PanelElements {
  panel: HTMLElement;
  rows: HTMLElement;
  limits: HTMLElement;
  toggle: HTMLElement;
}

const PROVIDER_NAME: Record<Provider, string> = {
  claude: "Claude Code",
  codex: "Codex",
};

// アプリのアイコンは同梱せず利用者のアプリから読むので、読めないときはこの文字で示す。
const PROVIDER_BADGE: Record<Provider, string> = {
  claude: "CC",
  codex: "CX",
};

export interface PanelLimits {
  claude: RateLimits | null;
  codex: CodexRateLimits | null;
  usage: UsageStatus | null;
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
  rateLimits: PanelLimits,
  icons: AppIcons,
  now: number,
  onSelect: (session: SessionState) => void,
): void {
  const line = limitLine(rateLimits.claude, rateLimits.codex, now);
  const note = usageNote(rateLimits.usage);
  if (line || note) renderLimits(limits, line, note, icons);
  let children: HTMLElement[];
  if (!hasContent(view)) children = [el("div", "empty", EMPTY_LIST_TEXT)];
  else if (view.style === "counts") children = [renderCounts(view, icons, now, onSelect)];
  else children = detailChildren(view.plan, icons, now, onSelect);
  rows.replaceChildren(...children);
  limits.hidden = line === null && note === null;
  panel.hidden = false;
  for (const button of toggle.querySelectorAll<HTMLElement>("button[data-style]")) {
    const active = button.dataset.style === view.style;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  }
}

function detailChildren(
  plan: PanelPlan,
  icons: AppIcons,
  now: number,
  onSelect: (session: SessionState) => void,
): HTMLElement[] {
  const children: HTMLElement[] = plan.rows.map((s) =>
    renderRow(s, plan.read.has(sessionKey(s)), icons, now, onSelect),
  );
  if (plan.moreRows > 0) children.push(el("div", "more", `ほか ${plan.moreRows} 件`));
  return children;
}

// 件数だけの表示は 1 行に畳み、マウスを載せたときに詳細の表示を上へ重ねて見せる。
// 押したときは、最も優先度の高い要対応のセッションへ移動する。
function renderCounts(
  view: PanelView,
  icons: AppIcons,
  now: number,
  onSelect: (session: SessionState) => void,
): HTMLElement {
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
  layer.append(...detailChildren(view.plan, icons, now, onSelect), line());
  wrap.append(line(), layer);
  const target = view.target;
  if (target) {
    wrap.classList.add("clickable");
    wrap.addEventListener("click", () => onSelect(target));
  }
  return wrap;
}

// 行は 2 段に詰めたまま広げない。広げる層を重ねると、押したときに層の開閉とクリックが競り、
// 1 回で移動できないことがある。要約の全文とコマンドは title のツールチップで読める。
// どのツールのセッションかの印は、2 段目の左の空いている場所に状態の点と縦に並べ、行の高さを変えない。
// 最後にフックが届いてからの時間は、2 段目の右のコンテキスト使用率の下に置く。
function renderRow(
  s: SessionState,
  read: boolean,
  icons: AppIcons,
  now: number,
  onSelect: (session: SessionState) => void,
): HTMLElement {
  const row = el("div", read ? "row read" : "row");
  const summary = s.activity?.summary || FALLBACK_SUMMARY[s.status];
  const detail = s.activity?.detail ?? "";
  const place = [s.title, s.cwd ?? s.session_id].filter(Boolean).join("\n");
  row.title = [place, summary, detail].filter(Boolean).join("\n\n");
  row.addEventListener("click", (e) => {
    e.stopPropagation();
    onSelect(s);
  });

  const names = el("span", "names");
  names.append(el("span", "folder", folderName(s)));
  if (s.title && (s.repo || !isCodexScratch(s))) names.append(el("span", "chat-title", s.title));
  row.append(
    el("span", `dot ${s.status}`),
    names,
    renderContext(s),
    providerMarker(s.provider ?? "claude", icons),
    el("div", "row-summary", summary),
    renderAge(s.updated_at, now),
  );
  return row;
}

function renderAge(updatedAt: number, now: number): HTMLElement {
  const age = el("span", "row-age", formatAge(updatedAt, now));
  age.title = `${formatClock(updatedAt)} 更新`;
  return age;
}

function providerMarker(provider: Provider, icons: AppIcons): HTMLElement {
  const src = icons[provider];
  if (!src) {
    const badge = el("span", "provider provider-badge", PROVIDER_BADGE[provider]);
    badge.setAttribute("aria-label", PROVIDER_NAME[provider]);
    return badge;
  }
  const icon = el("img", "provider provider-icon");
  icon.src = src;
  icon.alt = PROVIDER_NAME[provider];
  icon.draggable = false;
  return icon;
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

// 古さは組ごとに示す。全部の組が古いときは、以前の版と同じく行全体を薄くする。API から取れていない
// 理由があるときは更新の時刻の代わりに出し、値が一つも無くても行を出して空の理由が分かるようにする。
function renderLimits(container: HTMLElement, line: LimitLine | null, note: UsageNote | null, icons: AppIcons): void {
  const allStale = line !== null && line.groups.every((g) => g.stale);
  let values: HTMLElement;
  if (!line) {
    values = el("span", "limit-values", "利用制限");
  } else if (line.marked) {
    values = el("span", "limit-values marked");
    for (const g of line.groups) {
      const group = el("span", !allStale && g.stale ? "limit-group stale" : "limit-group");
      group.title = `${PROVIDER_NAME[g.provider]} ${formatClock(g.updatedAt)} 更新`;
      group.append(providerMarker(g.provider, icons), el("span", "", g.text));
      values.append(group);
    }
  } else {
    values = el("span", "limit-values", line.groups.map((g) => g.text).join(" · "));
  }
  const time = el("span", "limit-time");
  if (note) {
    time.classList.add("limit-note");
    time.textContent = note.text;
    time.title = line ? `${note.title}\n表示中の値は ${formatClock(line.updatedAt)} 更新` : note.title;
  } else if (line) {
    time.textContent = `${formatClock(line.updatedAt)} 更新`;
  }
  container.replaceChildren(values, time);
  container.classList.toggle("stale", allStale);
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
