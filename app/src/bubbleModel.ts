import { sessionKey, triggerKey, type Acknowledged } from "./acknowledged";
import { categoryFor, fillTemplate, linesFor } from "./dialogue";
import type { Dialogue, SessionState, Snapshot, Status } from "./types";

const SPEAKING: ReadonlySet<Status> = new Set(["waiting", "done", "error"]);

export interface BubbleView {
  key: string;
  sessionKey: string;
  status: Status;
  text: string;
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

  constructor(
    private readonly ack: Acknowledged,
    private readonly pick: Pick = randomPick,
  ) {}

  get view(): BubbleView | null {
    return this.current;
  }

  // 新しいきっかけに切り替わるときだけ、呼び出し側がセリフを読み直せばよい。
  needsText(snapshot: Snapshot | null): boolean {
    const next = this.candidate(snapshot);
    return next !== null && triggerKey(next) !== this.current?.key;
  }

  update(snapshot: Snapshot | null, dialogue: Dialogue): BubbleView | null {
    const next = this.candidate(snapshot);
    if (!next) {
      this.current = null;
      return null;
    }
    const key = triggerKey(next);
    if (this.current?.key === key) return this.current;
    const template = this.pick(linesFor(dialogue, categoryFor(next)));
    this.current = template
      ? { key, sessionKey: sessionKey(next), status: next.status, text: fillTemplate(template, next) }
      : null;
    return this.current;
  }

  dismiss(): void {
    if (this.current) this.ack.add(this.current.key);
    this.current = null;
  }

  // 出している吹き出しのきっかけが続いていればそれを保つ。同じ状態のセッションが複数あると、
  // 更新の新しい順は通知などで入れ替わり、吹き出しが行き来してしまうからである。
  // それ以外は集約で選ばれる順（優先度、同じなら新しい順）に、閉じられていない最初のものを選ぶ。
  private candidate(snapshot: Snapshot | null): SessionState | null {
    if (!snapshot || !SPEAKING.has(snapshot.aggregate)) return null;
    const eligible = snapshot.sessions.filter(
      (s) => s.status === snapshot.aggregate && !this.ack.has(s),
    );
    return eligible.find((s) => triggerKey(s) === this.current?.key) ?? eligible[0] ?? null;
  }
}
