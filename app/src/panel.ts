import { sessionKey } from "./acknowledged";
import { folderName, formatAge, formatClock, formatTokens, titleIsName } from "./format";
import { limitLine, type LimitLine } from "./limits";
import { hasContent, type PanelPlan, type PanelView } from "./panelModel";
import type { AppIcons, CodexRateLimits, Provider, RateLimits, SessionState, Status } from "./types";

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
  order: HTMLElement;
}

const PROVIDER_NAME: Record<Provider, string> = {
  claude: "Claude Code",
  codex: "Codex",
  hermes: "Hermes Agent",
};

// アプリのアイコンは同梱せず利用者のアプリから読むので、読めないときはこの文字で示す。
// Hermes はデスクトップアプリを持たないので、いつもこの文字になる。
const PROVIDER_BADGE: Record<Provider, string> = {
  claude: "CC",
  codex: "CX",
  hermes: "HM",
};

export interface PanelLimits {
  claude: RateLimits | null;
  codex: CodexRateLimits | null;
}

const STATUS_LABEL: Record<Status, string> = {
  idle: "待機",
  working: "作業中",
  waiting: "承認待ち",
  done: "完了",
  error: "エラー",
};

export function renderPanel(
  { panel, rows, limits, toggle, order }: PanelElements,
  view: PanelView,
  rateLimits: PanelLimits,
  icons: AppIcons,
  now: number,
  onSelect: (session: SessionState) => void,
): void {
  const line = limitLine(rateLimits.claude, rateLimits.codex, now);
  if (line) renderLimits(limits, line, icons);
  let children: HTMLElement[];
  if (!hasContent(view)) children = [el("div", "empty", EMPTY_LIST_TEXT)];
  else if (view.style === "counts") children = [renderCounts(view, icons, now, onSelect)];
  else children = detailChildren(view.plan, icons, now, onSelect);
  const kept = keepScroll(rows);
  rows.replaceChildren(...children);
  // 件数の行の層は #rows の外へ伸びるので、件数だけの表示では #rows をスクロールの枠にしない。
  // スクロールの枠は中身を枠の外へはみ出させず、層が切れてしまうからである。
  rows.classList.toggle("row-scroll", view.style === "detail");
  limits.hidden = line === null;
  panel.hidden = false;
  fitLayers(rows);
  restoreScroll(rows, kept);
  markActive(toggle, "style", view.style);
  markActive(order, "order", view.order);
}

function markActive(group: HTMLElement, key: string, value: string): void {
  for (const button of group.querySelectorAll<HTMLElement>(`button[data-${key}]`)) {
    const active = button.dataset[key] === value;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  }
}

interface KeptScroll {
  top: number;
  /** 開いていた件数の行の印ごとに、その層の中のスクロールの位置を持つ。 */
  layers: Map<string, number>;
}

// 描き直すたびに行を作り直すので、何もしなければスクロールの位置が一番上へ戻ってしまう。作り直す前に
// 位置を覚えておき、作り直した後に戻す。行が減って届かなくなった位置は、ブラウザが一番下へ詰める。
function keepScroll(rows: HTMLElement): KeptScroll {
  const layers = new Map<string, number>();
  for (const group of rows.querySelectorAll<HTMLElement>("[data-expand-id].open")) {
    layers.set(group.dataset.expandId ?? "", group.querySelector(".row-scroll")?.scrollTop ?? 0);
  }
  return { top: rows.scrollTop, layers };
}

// 開いていた層は、登録を待たずにそのまま開いて見せる。描き直すたびに測り直すと層がちらつき、行の増減で
// 大きさが変わった分は、描き直しの後に送り直す領域で追いつくからである。見えていない層にはスクロールの
// 位置を戻せないので、先に開いてから戻す。
function restoreScroll(rows: HTMLElement, kept: KeptScroll): void {
  rows.scrollTop = kept.top;
  for (const group of rows.querySelectorAll<HTMLElement>("[data-expand-id]")) {
    const top = kept.layers.get(group.dataset.expandId ?? "");
    if (top === undefined) continue;
    group.classList.add("open");
    const scroller = group.querySelector<HTMLElement>(".row-scroll");
    if (scroller) scroller.scrollTop = top;
  }
}

// 層は件数の行の下端から上へ伸びるので、窓の上端までに収め、収まらない行は層の中でスクロールさせる。
// 窓の高さが変わっても、パネルは下端を固定しているので層の下端から窓の下端までの距離は変わらない。
// その距離を CSS へ渡し、窓の高さからの計算は CSS に任せる。
function fitLayers(rows: HTMLElement): void {
  for (const layer of rows.querySelectorAll<HTMLElement>(".hover-layer")) {
    const anchor = layer.parentElement?.getBoundingClientRect();
    if (anchor) layer.style.setProperty("--layer-below", `${window.innerHeight - anchor.bottom}px`);
  }
}

/** スクロールバーの操作を、行を押したことや窓のドラッグと取り違えないために見分ける。 */
export function onScrollbar(e: MouseEvent): boolean {
  const target = e.target;
  return target instanceof HTMLElement && target.classList.contains("row-scroll") && e.offsetX >= target.clientWidth;
}

function detailChildren(
  plan: PanelPlan,
  icons: AppIcons,
  now: number,
  onSelect: (session: SessionState) => void,
): HTMLElement[] {
  return plan.rows.map((s) => renderRow(s, plan.read.has(sessionKey(s)), icons, now, onSelect));
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
  // 件数の行まで一緒に流れないよう、スクロールの枠はその上の行だけにする。
  const list = el("div", "layer-rows row-scroll");
  list.append(...detailChildren(view.plan, icons, now, onSelect));
  layer.append(list, line());
  wrap.append(line(), layer);
  const target = view.target;
  if (target) {
    wrap.classList.add("clickable");
    wrap.addEventListener("click", (e) => {
      if (!onScrollbar(e)) onSelect(target);
    });
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
  const row = el("div", read ? "row read" : s.cloud?.expired ? "row stale" : "row");
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
  if (s.title && (s.repo || !titleIsName(s))) names.append(el("span", "chat-title", s.title));
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

// 古さは組ごとに示す。全部の組が古いときは、以前の版と同じく行全体を薄くする。
function renderLimits(container: HTMLElement, line: LimitLine, icons: AppIcons): void {
  const allStale = line.groups.every((g) => g.stale);
  let values: HTMLElement;
  if (line.marked) {
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
  container.replaceChildren(values, el("span", "limit-time", `${formatClock(line.updatedAt)} 更新`));
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
