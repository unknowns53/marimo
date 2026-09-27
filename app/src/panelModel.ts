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
