import type { Acknowledged } from "./acknowledged";
import type { SessionState, Snapshot, Status } from "./types";

// 詳細の段階の行数の上限。行はリストの上へ伸びるだけで立ち絵は動かないが、窓の高さに収める。
export const MAX_ROWS = 5;
// 見たと示した完了の行を、薄くして残しておく時間。
export const READ_LINGER_MS = 3 * 60 * 1000;

export interface PanelPlan {
  rows: SessionState[];
  moreRows: number;
  /** 見たと示された直後の完了の行の session_id。薄く描く。 */
  read: ReadonlySet<string>;
}

/**
 * 詳細の段階で出す行を決める。待機以外のセッションを 1 セッション 1 行で並べ、作業中も畳まない。
 * 見たと示された完了は畳むが、押した直後に行が消えると何を押したのか見失うので、READ_LINGER_MS の
 * 間だけ薄くして未読の行の後ろに残す。完了のまま放っておかれるセッションは多く、いつまでも残すと
 * 古い既読で埋まる。それ以外の並びは snapshot の順（優先度の高い順、同じなら更新の新しい順）を保つ。
 */
export function planPanel(snapshot: Snapshot | null, ack: Acknowledged, now: number = Date.now()): PanelPlan {
  const active = (snapshot?.sessions ?? []).filter((s) => s.status !== "idle");
  const isRead = (s: SessionState) => s.status === "done" && ack.has(s);
  const lingering = active.filter((s) => {
    const at = isRead(s) ? ack.seenAt(s) : undefined;
    return at !== undefined && now - at < READ_LINGER_MS;
  });
  const rows = [...active.filter((s) => !isRead(s)), ...lingering];
  return {
    rows: rows.slice(0, MAX_ROWS),
    moreRows: Math.max(0, rows.length - MAX_ROWS),
    read: new Set(lingering.map((s) => s.session_id)),
  };
}

export function isEmpty(plan: PanelPlan): boolean {
  return plan.rows.length === 0;
}

export type PanelMode = "detail" | "counts" | "picture";

export const PANEL_MODES: readonly PanelMode[] = ["detail", "counts", "picture"];

export interface StatusCount {
  status: Status;
  count: number;
}

export interface PanelView {
  mode: PanelMode;
  plan: PanelPlan;
  counts: StatusCount[];
  /** 件数の行を押したときに移動する、最も優先度の高い要対応のセッション。 */
  target: SessionState | null;
}

const COUNT_ORDER: Status[] = ["waiting", "error", "working", "done"];
const ATTENTION: ReadonlySet<Status> = new Set(["waiting", "error", "done"]);

/**
 * 段階ごとに何を出すかを決める。件数だけの段階でも数え方は詳細と同じにし、見たと示された完了は
 * 数えない。どの段階でも吹き出しや表情の扱いは変えない。
 */
export function panelView(
  snapshot: Snapshot | null,
  ack: Acknowledged,
  mode: PanelMode,
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
  return { mode, plan, counts, target };
}

export function hasContent(view: PanelView): boolean {
  switch (view.mode) {
    case "detail":
      return !isEmpty(view.plan);
    case "counts":
      return view.counts.length > 0;
    case "picture":
      return false;
  }
}
