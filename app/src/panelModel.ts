import type { Acknowledged } from "./acknowledged";
import type { SessionState, Snapshot, Status } from "./types";

export const MAX_ATTENTION_ROWS = 3;
// 作業中の一覧は立ち絵の上へ重ねて広げるので、窓の高さに収まる数で止める。
export const MAX_WORKING_ROWS = 5;

const ATTENTION: ReadonlySet<Status> = new Set(["waiting", "error", "done"]);

export interface PanelPlan {
  attention: SessionState[];
  moreAttention: number;
  working: SessionState[];
  moreWorking: number;
}

/**
 * パネルに何を出すかを決める。行を出すのは利用者の対応が要るセッション（承認待ち、エラー、完了）
 * だけで、作業中は件数の要約にまとめる。完了は見たと示されたら畳み、承認待ちとエラーは
 * 解決するまで残す。並びは snapshot の順（優先度、同じなら新しい順）を保つ。
 */
export function planPanel(snapshot: Snapshot | null, ack: Acknowledged): PanelPlan {
  const sessions = snapshot?.sessions ?? [];
  const attention = sessions.filter(
    (s) => ATTENTION.has(s.status) && !(s.status === "done" && ack.has(s)),
  );
  const working = sessions.filter((s) => s.status === "working");
  return {
    attention: attention.slice(0, MAX_ATTENTION_ROWS),
    moreAttention: Math.max(0, attention.length - MAX_ATTENTION_ROWS),
    working: working.slice(0, MAX_WORKING_ROWS),
    moreWorking: Math.max(0, working.length - MAX_WORKING_ROWS),
  };
}

export function isEmpty(plan: PanelPlan): boolean {
  return plan.attention.length === 0 && plan.working.length === 0;
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

const COUNT_ORDER: Status[] = ["waiting", "error", "done", "working"];

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
  return { mode, plan, counts, target: plan.attention[0] ?? null };
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
