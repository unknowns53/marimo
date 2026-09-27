import { folderName } from "./format";
import type { Dialogue, SessionState, Snapshot, Status } from "./types";

const SPEAKING: ReadonlySet<Status> = new Set(["waiting", "done", "error"]);

export interface BubbleView {
  key: string;
  sessionId: string;
  text: string;
}

// 吹き出しのきっかけは「どのセッションが、いつから、どの状態か」で見分ける。status_since を
// 含めるので、同じセッションが一度別の状態を経て同じ状態へ戻れば、新しいきっかけになる。
export function triggerKey(s: SessionState): string {
  return `${s.session_id}:${s.status}:${s.status_since}`;
}

export function fillFolder(template: string, folder: string): string {
  return template.split("{folder}").join(folder);
}

type Pick = (lines: string[]) => string | undefined;

const randomPick: Pick = (lines) => lines[Math.floor(Math.random() * lines.length)];

/**
 * 集約状態の変化から、吹き出しを出す・保つ・消すを決める。DOM に触れないので単体で試せる。
 * 承認待ち、エラー、完了は別の作業から戻ったときに気づくためのものなので、きっかけの状態が
 * 集約状態として続く間は出したままにする。
 */
export class BubbleModel {
  private current: BubbleView | null = null;
  private dismissed = new Set<string>();

  constructor(private readonly pick: Pick = randomPick) {}

  get view(): BubbleView | null {
    return this.current;
  }

  // 新しいきっかけに切り替わるときだけ、呼び出し側がセリフを読み直せばよい。
  needsText(snapshot: Snapshot | null): boolean {
    const next = this.candidate(snapshot);
    return next !== null && triggerKey(next) !== this.current?.key;
  }

  update(snapshot: Snapshot | null, dialogue: Dialogue): BubbleView | null {
    this.forgetStale(snapshot);
    const next = this.candidate(snapshot);
    if (!next) {
      this.current = null;
      return null;
    }
    const key = triggerKey(next);
    if (this.current?.key === key) return this.current;
    const template = this.pick(dialogue[next.status] ?? []);
    this.current = template
      ? { key, sessionId: next.session_id, text: fillFolder(template, folderName(next)) }
      : null;
    return this.current;
  }

  dismiss(): void {
    if (this.current) this.dismissed.add(this.current.key);
    this.current = null;
  }

  // 出している吹き出しのきっかけが続いていればそれを保つ。同じ状態のセッションが複数あると、
  // 更新の新しい順は通知などで入れ替わり、吹き出しが行き来してしまうからである。
  // それ以外は集約で選ばれる順（優先度、同じなら新しい順）に、閉じられていない最初のものを選ぶ。
  private candidate(snapshot: Snapshot | null): SessionState | null {
    if (!snapshot || !SPEAKING.has(snapshot.aggregate)) return null;
    const eligible = snapshot.sessions.filter(
      (s) => s.status === snapshot.aggregate && !this.dismissed.has(triggerKey(s)),
    );
    return eligible.find((s) => triggerKey(s) === this.current?.key) ?? eligible[0] ?? null;
  }

  // 閉じた記録は、そのきっかけが続いている間だけ要る。
  private forgetStale(snapshot: Snapshot | null): void {
    const live = new Set((snapshot?.sessions ?? []).map(triggerKey));
    for (const key of this.dismissed) {
      if (!live.has(key)) this.dismissed.delete(key);
    }
  }
}
