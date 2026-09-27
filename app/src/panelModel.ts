import type { Acknowledged } from "./acknowledged";
import type { SessionState, Snapshot, Status } from "./types";

// 詳細の段階の行数の上限。行はリストの上へ伸びるだけで立ち絵は動かないが、窓の高さに収める。
export const MAX_ROWS = 5;

export interface PanelPlan {
  rows: SessionState[];
  moreRows: number;
  /** 見たと示された完了の行の session_id。薄く描く。 */
  read: ReadonlySet<string>;
}

/**
 * 詳細の段階で出す行を決める。待機以外のセッションを 1 セッション 1 行で並べ、作業中も畳まない。
 * 見たと示された完了も、押した直後に行が消えると何を押したのか見失うので、そのセッションが次に
 * 動き出すまで薄くして残す。場所を譲るよう、未読の行の後ろへ回す。それ以外の並びは snapshot の順
 * （優先度の高い順、同じなら更新の新しい順）を保つ。
 */
export function planPanel(snapshot: Snapshot | null, ack: Acknowledged): PanelPlan {
  const active = (snapshot?.sessions ?? []).filter((s) => s.status !== "idle");
  const isRead = (s: SessionState) => s.status === "done" && ack.has(s);
  const rows = [...active.filter((s) => !isRead(s)), ...active.filter(isRead)];
  return {
    rows: rows.slice(0, MAX_ROWS),
    moreRows: Math.max(0, rows.length - MAX_ROWS),
    read: new Set(active.filter(isRead).map((s) => s.session_id)),
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
export function panelView(snapshot: Snapshot | null, ack: Acknowledged, mode: PanelMode): PanelView {
  const plan = planPanel(snapshot, ack);
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
