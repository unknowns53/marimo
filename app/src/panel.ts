import { MARKABLE, sessionKey } from "./acknowledged";
import { chatTitle, folderName, formatAge, formatTokens, formatUpdated } from "./format";
import { plural, t } from "./i18n";
import { limitLine, limitTooltip, type LimitLine, type LimitSources } from "./limits";
import { isEmpty, panelSignature, type PanelPlan, type PanelView } from "./panelModel";
import type { AppIcons, Provider, SessionState, Status } from "./types";

function fallbackSummary(status: Status): string {
  switch (status) {
    case "idle":
      return "";
    case "working":
      return t("考え中…", "Thinking…");
    case "waiting":
      return t("確認待ち", "Needs your input");
    case "done":
      return t("完了", "Done");
    case "error":
      return t("エラー", "Error");
  }
}

export interface PanelElements {
  panel: HTMLElement;
  rows: HTMLElement;
  limits: HTMLElement;
  drag: HTMLElement;
}

const PROVIDER_NAME: Record<Provider, string> = {
  claude: "Claude Code",
  codex: "Codex",
  hermes: "Hermes Agent",
  opencode: "OpenCode",
  antigravity: "Antigravity",
};

// Claude、Codex、Antigravity のアイコンは同梱できないので利用者のデスクトップアプリから読み、読めないときはこの文字で示す。
const PROVIDER_BADGE: Record<Provider, string> = {
  claude: "CC",
  codex: "CX",
  hermes: "HM",
  opencode: "OC",
  antigravity: "AG",
};

// Hermes Agent と OpenCode は MIT ライセンスで配られているアイコンを同梱し、デスクトップアプリが無くても出す。
// 出どころと許諾は LICENSE-ASSETS に書いている。
const BUNDLED_ICON: Partial<Record<Provider, string>> = {
  hermes: new URL("./assets/providers/hermes.png", import.meta.url).href,
  opencode: new URL("./assets/providers/opencode.png", import.meta.url).href,
};

/** 行を押したときの操作。markRead は移動せずに、そのトリガーを見たものとして扱う。 */
export interface RowActions {
  select: (session: SessionState) => void;
  markRead: (session: SessionState) => void;
  /** 今日の既読の行を押したときに、その下の一覧を開閉する。 */
  toggleToday: () => void;
}

// 移動せずに既読にする修飾キー。macOS の Option と、それ以外の Alt は同じ altKey で届く。
export const READ_KEY = navigator.userAgent.includes("Mac") ? "Option" : "Alt";

function press(e: MouseEvent, session: SessionState, actions: RowActions): void {
  if (e.altKey) actions.markRead(session);
  else actions.select(session);
}

// 行を作り直すと、押している途中の行やボタンが DOM から外れてクリックを取りこぼし、title のツールチップも消える。
// そこで、描き方を決める入力が前と同じなら行を置き換えず、ボタンを押している間は離すまで置き換えを遅らせる。
let shownSignature: string | null = null;
let pressed = false;
let deferred = false;

/** 行の上でボタンを押している間は行を置き換えず、離したら redraw で描き直す。起動時に一度だけ呼ぶ。 */
export function holdRowsWhilePressed(rows: HTMLElement, redraw: () => void): void {
  rows.addEventListener("pointerdown", () => {
    pressed = true;
  });
  const release = (): void => {
    if (!pressed) return;
    pressed = false;
    if (!deferred) return;
    deferred = false;
    // 離した後の click の処理が終わってから描き直す。
    window.setTimeout(redraw, 0);
  };
  // ウィンドウの外で離したときは pointerup が届かないことがあるので、blur でも離したものとする。
  for (const type of ["pointerup", "pointercancel", "blur"]) window.addEventListener(type, release);
}

export function renderPanel(
  { panel, rows, limits, drag }: PanelElements,
  view: PanelView,
  rateLimits: LimitSources,
  icons: AppIcons,
  now: number,
  actions: RowActions,
  todayOpen: boolean,
  characterShown: boolean,
): void {
  const line = limitLine(rateLimits, now);
  if (line) renderLimits(limits, line, icons, now);
  // パネルを隠す間も、立ち絵を出していれば立ち絵を押して戻せるので、パネルごと隠す。立ち絵も隠しているときは、
  // 何も残らないと掴んで動かせず、どこにあるかも分からなくなるので、利用制限の欄だけを残す。利用制限の欄も
  // 無いときは、隠していることを示す 1 行を出す。
  // 利用制限の欄の有無は行の描き方を決める入力に含めないので、下の早期の return より前で決める。
  const collapsed = view.collapsed;
  panel.hidden = collapsed && characterShown;
  limits.hidden = line === null;
  rows.hidden = collapsed && line !== null;
  drag.title = t("ドラッグしてウィンドウを移動", "Drag to move window");
  const iconsShown = Object.entries(icons).map(([provider, src]) => [provider, src !== null]);
  const signature = panelSignature(view, now, todayOpen) + JSON.stringify(iconsShown);
  if (signature === shownSignature) return;
  if (pressed) {
    deferred = true;
    return;
  }
  const { plan } = view;
  let children: HTMLElement[];
  if (collapsed) children = [el("div", "empty", t("パネルを隠しています", "Panel hidden"))];
  else {
    const today = todayChildren(plan, icons, now, actions, todayOpen);
    children = isEmpty(plan)
      ? [el("div", "empty", t("進行中のセッションはありません", "No active sessions")), ...today]
      : [...plan.rows.map((s) => renderRow(s, plan.read.has(sessionKey(s)), icons, now, actions)), ...today];
  }
  // 描き直すたびに行を作り直すので、何もしなければスクロールの位置が一番上へ戻ってしまう。行が減って
  // 届かなくなった位置は、ブラウザが一番下へ詰める。
  const top = rows.scrollTop;
  rows.replaceChildren(...children);
  rows.scrollTop = top;
  shownSignature = signature;
}

/** スクロールバーの操作を、行を押したことやウィンドウのドラッグと取り違えないために見分ける。 */
export function onScrollbar(e: MouseEvent): boolean {
  const target = e.target;
  return target instanceof HTMLElement && target.classList.contains("row-scroll") && e.offsetX >= target.clientWidth;
}

// 折りたたんだ既読の完了とエラーは 1 行の件数にまとめ、押すとその下に既読の行と同じ薄さで並べる。もう既読なので
// 既読にするボタンは出さない。
function todayChildren(
  plan: PanelPlan,
  icons: AppIcons,
  now: number,
  actions: RowActions,
  open: boolean,
): HTMLElement[] {
  if (plan.today.length === 0) return [];
  const line = el("div", "today-line");
  line.title = t(
    "今日完了かエラーになって既読にしたセッション",
    "Sessions that finished or failed today and were marked read",
  );
  line.setAttribute("role", "button");
  line.setAttribute("aria-expanded", String(open));
  const count = plan.today.length;
  line.append(
    el("span", "today-marker", open ? "▾" : "▸"),
    el("span", "", t(`今日の既読 ${count} 件`, `Read today: ${count}`)),
  );
  line.addEventListener("click", (e) => {
    e.stopPropagation();
    actions.toggleToday();
  });
  if (!open) return [line];
  return [line, ...plan.today.map((s) => renderRow(s, true, icons, now, actions))];
}

// 状態は点ではなく、行の左から右へ薄れていく色で示す。文字を読まなくても、視界の端で何色の行が何本あるかが分かる。
// 作業中の行は、眺めるときに要る名前、要約、経過時間、コンテキスト使用率だけを 1 段に並べ、手を貸す必要のある
// 承認待ち、エラー、完了の行だけを 2 段にして目立たせる。
// 行は広げない。広げるレイヤーを重ねると、押したときにレイヤーの開閉とクリックが競り、1 回で移動できないことがある。
// チャットのタイトル、ツール、要約の全文、コマンドは title のツールチップで読める。
// 2 段の行では、どのツールのセッションかを示すマークを 2 段目の左に置き、名前より一段下げて行の高さを変えない。
// まだ見ていない完了とエラーの行は、マウスを載せている間だけ時間の場所に既読にするボタンを出し、アプリを開かずに折りたためるようにする。
function renderRow(
  s: SessionState,
  read: boolean,
  icons: AppIcons,
  now: number,
  actions: RowActions,
): HTMLElement {
  const compact = s.status === "working";
  const classes = ["row", `s-${s.status}`];
  if (compact) classes.push("compact");
  if (read) classes.push("read");
  const row = el("div", classes.join(" "));
  row.dataset.sessionKey = sessionKey(s);
  const provider = s.provider ?? "claude";
  const summary = s.activity?.summary || fallbackSummary(s.status);
  row.title = rowTooltip(s, provider, summary);
  row.addEventListener("click", (e) => {
    e.stopPropagation();
    press(e, s, actions);
  });

  const folder = el("span", "folder", folderName(s));
  if (compact) {
    row.append(folder, renderSummary(s, summary), renderAge(s.updated_at, now), renderContext(s));
    return row;
  }
  const names = el("span", "names");
  names.append(folder);
  const title = chatTitle(s);
  if (title) names.append(el("span", "chat-title", title));
  row.append(
    names,
    renderContext(s),
    providerMarker(provider, icons),
    renderSummary(s, summary),
    renderAge(s.updated_at, now),
  );
  if (MARKABLE.has(s.status) && !read) {
    row.classList.add("markable");
    row.append(markReadButton(s, actions));
  }
  return row;
}

function rowTooltip(s: SessionState, provider: Provider, summary: string): string {
  const place = [PROVIDER_NAME[provider], s.title, s.cwd ?? s.session_id].filter(Boolean).join("\n");
  return [place, summary, s.activity?.detail ?? ""].filter(Boolean).join("\n\n");
}

// 動いているサブエージェントの数は、要約の頭に小さく出す。要約は右から省略されるので、数は隠れない。
// Hermes Agent のサブエージェントは別の行になるので、ここには数えない。
function renderSummary(s: SessionState, summary: string): HTMLElement {
  const line = el("div", "row-summary");
  const agents = Object.keys(s.agents ?? {}).length;
  if (agents > 0) {
    const badge = el("span", "row-agents", t(`サブ ${agents}`, `Sub ${agents}`));
    badge.title = t(`サブエージェント ${agents} 件が動いています`, `${plural(agents, "subagent", "subagents")} running`);
    line.append(badge);
  }
  line.append(summary);
  return line;
}

function markReadButton(s: SessionState, actions: RowActions): HTMLElement {
  const button = el("button", "row-read", "✓");
  button.type = "button";
  button.title = t(
    `既読にする（アプリは開きません。${READ_KEY} を押しながらセッションをクリックしても同じです）`,
    `Mark as read without opening the app (same as ${READ_KEY}-clicking the session)`,
  );
  button.setAttribute("aria-label", t("既読にする", "Mark as read"));
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    actions.markRead(s);
  });
  return button;
}

function renderAge(updatedAt: number, now: number): HTMLElement {
  const age = el("span", "row-age", formatAge(updatedAt, now));
  age.title = formatUpdated(updatedAt);
  return age;
}

function providerMarker(provider: Provider, icons: AppIcons): HTMLElement {
  const src = icons[provider] ?? BUNDLED_ICON[provider];
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
    wrap.title = t(`コンテキスト ${Math.round(pct)}%`, `Context ${Math.round(pct)}%`);
    const bar = el("span", "ctx-bar");
    const fill = el("i", pct >= 80 ? "high" : "");
    fill.style.width = `${pct}%`;
    bar.append(fill);
    wrap.append(bar, el("span", "ctx-label", `${Math.round(pct)}%`));
    return wrap;
  }
  if (ctx?.total_input_tokens != null) {
    const tokens = el("span", "ctx-tokens", formatTokens(ctx.total_input_tokens));
    const count = ctx.total_input_tokens.toLocaleString();
    tokens.title = t(
      `コンテキスト ${count} トークン（上限が不明なため % は表示していません）`,
      `Context ${count} tokens (limit unknown, so no %)`,
    );
    return tokens;
  }
  return el("span", "ctx-none");
}

// 各ツールの値は、ウィンドウごとの細いバーを上から 5h、7d の順に積み、横に数字を添える。80% を超えたバーは
// コンテキスト使用率と同じく色を変え、文字を読まなくても残りの少なさに気づけるようにする。5h や 7d の見出しと
// 更新の時刻は、三つのツールが並ぶと行に収まらないので、各グループのツールチップに任せる。
// 古さはグループごとに示す。全部のグループが古いときは、以前のバージョンと同じく行全体を薄くする。
function renderLimits(container: HTMLElement, line: LimitLine, icons: AppIcons, now: number): void {
  const allStale = line.groups.every((g) => g.stale);
  const values = el("span", "limit-values");
  const nums = line.groups.map((g) => `${g.used.map((u) => Math.round(u)).join(" · ")}%`);
  // 数字の欄を、いちばん長い数字の幅にそろえる。グループは欄の幅を等分するので、数字の幅が違うと、同じ使用率でも
  // グループごとにバーの長さが変わって見える。数字はどれも tabular-nums なので、文字数が多いほど幅も広い。
  const widest = nums.reduce((a, b) => (b.length > a.length ? b : a), "");
  for (const [i, g] of line.groups.entries()) {
    const group = el("span", !allStale && g.stale ? "limit-group stale" : "limit-group");
    group.title = limitTooltip(g, PROVIDER_NAME[g.provider], now);
    // Claude Code の値しか無いときは、どのツールの値か迷わないのでアイコンを付けない。
    if (line.marked) group.append(providerMarker(g.provider, icons));
    const stack = el("span", "limit-stack");
    for (const used of g.used) {
      const pct = Math.max(0, Math.min(100, used));
      const bar = el("span", "ctx-bar");
      const fill = el("i", pct >= 80 ? "high" : "");
      fill.style.width = `${pct}%`;
      bar.append(fill);
      stack.append(bar);
    }
    const label = el("span", "limit-nums", nums[i]);
    label.dataset.widest = widest;
    group.append(stack, label);
    values.append(group);
  }
  container.replaceChildren(values);
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
