import { sessionKey, type Acknowledged } from "./acknowledged";
import type { SessionState, Snapshot, Status } from "./types";

// 詳細の表示の行数の上限。行はリストの上へ伸びるだけで立ち絵は動かないが、窓の高さに収める。
export const MAX_ROWS = 5;
// 見たと示した完了の行を、薄くして残しておく時間。
export const READ_LINGER_MS = 3 * 60 * 1000;

export interface PanelPlan {
  rows: SessionState[];
  moreRows: number;
  /** 見たと示された直後の完了の行の sessionKey。薄く描く。 */
  read: ReadonlySet<string>;
}

// 承認待ち、エラー、作業中、完了の順に重要とみなす。marimo-core の Status::priority と同じ順である。
const COUNT_ORDER: Status[] = ["waiting", "error", "working", "done"];

/**
 * 詳細の表示で出す行を決める。待機以外のセッションを 1 セッション 1 行で並べ、作業中も畳まない。
 * 見たと示された完了は畳むが、押した直後に行が消えると何を押したのか見失うので、READ_LINGER_MS の
 * 間だけその場で薄くして残す。完了のまま放っておかれるセッションは多く、いつまでも残すと古い既読で埋まる。
 *
 * 行が MAX_ROWS に収まらないときは、未読を既読より、状態の優先度の高いものを低いものより、同じなら
 * 更新の新しいものを先に選び、承認待ちが「ほか n 件」に隠れないようにする。選んだ行は started_at の
 * 新しい順に並べる。パネルは下端を固定して上へ伸びるので、新しいセッションが一番上に加わっても
 * 既にある行は画面上の位置が変わらず、状態が変わっても行は動かない。started_at を持たない古い
 * ファイルのセッションは最も古いものとして一番下に置き、更新のたびに動かないようにする。
 */
export function planPanel(snapshot: Snapshot | null, ack: Acknowledged, now: number = Date.now()): PanelPlan {
  const active = (snapshot?.sessions ?? []).filter((s) => s.status !== "idle");
  const isRead = (s: SessionState) => s.status === "done" && ack.has(s);
  const lingering = active.filter((s) => {
    const at = isRead(s) ? ack.seenAt(s) : undefined;
    return at !== undefined && now - at < READ_LINGER_MS;
  });
  const read = new Set(lingering.map(sessionKey));
  const candidates = [...active.filter((s) => !isRead(s)), ...lingering];
  const importance = (a: SessionState, b: SessionState) =>
    Number(read.has(sessionKey(a))) - Number(read.has(sessionKey(b))) ||
    COUNT_ORDER.indexOf(a.status) - COUNT_ORDER.indexOf(b.status) ||
    b.updated_at - a.updated_at;
  const rows = candidates
    .sort(importance)
    .slice(0, MAX_ROWS)
    .sort((a, b) => b.started_at - a.started_at || compareIds(sessionKey(a), sessionKey(b)));
  return {
    rows,
    moreRows: candidates.length - rows.length,
    read,
  };
}

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

export function isEmpty(plan: PanelPlan): boolean {
  return plan.rows.length === 0;
}

/** パネルの行の出し方。立ち絵を出すかどうかとは独立に選ぶ。 */
export type PanelStyle = "detail" | "counts";

export const PANEL_STYLES: readonly PanelStyle[] = ["detail", "counts"];

/** Rust の scale::PanelDisplay と同じ形で、display.json に保存する。 */
export interface PanelDisplay {
  show_character: boolean;
  panel_style: PanelStyle;
}

export const DEFAULT_PANEL_DISPLAY: PanelDisplay = { show_character: true, panel_style: "detail" };

export interface StatusCount {
  status: Status;
  count: number;
}

export interface PanelView {
  style: PanelStyle;
  plan: PanelPlan;
  counts: StatusCount[];
  /** 件数の行を押したときに移動する、最も優先度の高い要対応のセッション。 */
  target: SessionState | null;
}

const ATTENTION: ReadonlySet<Status> = new Set(["waiting", "error", "done"]);

/**
 * 行の出し方ごとに何を出すかを決める。件数だけの表示でも数え方は詳細と同じにし、見たと示された完了は
 * 数えない。吹き出しや表情を決める仕組みは表示によらず同じで、立ち絵を隠している間はそれを見せないだけにする。
 */
export function panelView(
  snapshot: Snapshot | null,
  ack: Acknowledged,
  style: PanelStyle,
  now: number = Date.now(),
): PanelView {
  const plan = planPanel(snapshot, ack, now);
  const sessions = snapshot?.sessions ?? [];
  const counted = sessions.filter((s) => !(s.status === "done" && ack.has(s)));
  const counts = COUNT_ORDER.map((status) => ({
    status,
    count: counted.filter((s) => s.status === status).length,
  })).filter((c) => c.count > 0);
  const target = counted.find((s) => ATTENTION.has(s.status)) ?? null;
  return { style, plan, counts, target };
}

/**
 * 今の出し方で出す行があるかどうか。無いときも、パネルは無いことを示す 1 行を出して残す。
 * パネルごと消えると、詳細と件数を切り替える場所も、立ち絵を隠しているときに右クリックやドラッグを
 * 受ける場所も無くなるからである。
 */
export function hasContent(view: PanelView): boolean {
  return view.style === "detail" ? !isEmpty(view.plan) : view.counts.length > 0;
}
