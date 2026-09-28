import type { ExpressionRules } from "./manifest";
import type { Status } from "./types";

export interface DirectorInput {
  status: Status;
  tool: string | null;
}

/**
 * 今どの表情を出すかを決める。DOM と時計に触れず、時刻は呼び出し側が渡すので単体で試せる。
 * 優先度は、押したときの反応、マウスを載せたときの表情、待機中の仕草、状態の基本の表情の順。
 * 素材の無い表情は選ばず、状態の基本の表情で代用する。
 */
export class ExpressionDirector {
  private input: DirectorInput = { status: "idle", tool: null };
  private workingExpression: string | null = null;
  private workingSince = Number.NEGATIVE_INFINITY;
  private gesture: { expression: string; until: number } | null = null;
  private lastGesture: string | null = null;
  private nextGestureAt: number | null = null;
  private hovered = false;
  private hoverUntil = 0;
  private reactUntil = 0;

  constructor(
    private readonly rules: ExpressionRules,
    private readonly available: (name: string) => boolean,
    private readonly random: () => number = Math.random,
  ) {}

  setInput(input: DirectorInput): void {
    if (input.status !== this.input.status) {
      this.workingExpression = null;
      this.gesture = null;
      this.nextGestureAt = null;
    }
    this.input = input;
  }

  setHover(on: boolean, now: number): void {
    if (this.hovered && !on) this.hoverUntil = now + this.rules.hoverReleaseMs;
    this.hovered = on;
  }

  /** 反応の表情を出せたら true を返す。吹き出しを出すかどうかは呼び出し側が決める。 */
  react(now: number): boolean {
    const r = this.rules.reaction;
    if (!r || !this.available(r.expression)) return false;
    this.reactUntil = now + r.durationMs;
    this.gesture = null;
    return true;
  }

  current(now: number): string | null {
    const base = this.baseExpression(now);
    this.advanceGesture(now);
    const r = this.rules.reaction;
    if (r && now < this.reactUntil) return r.expression;
    const hover = this.rules.hover[this.input.status];
    if (hover && this.hoverActive(now) && this.available(hover)) return hover;
    if (this.gesture) return this.gesture.expression;
    return base;
  }

  private hoverActive(now: number): boolean {
    return this.hovered || now < this.hoverUntil;
  }

  private statusExpression(status: Status): string | null {
    for (const name of [this.rules.status[status], status, this.rules.status.idle, "idle"]) {
      if (name && this.available(name)) return name;
    }
    return null;
  }

  // ツールは数秒ごとに変わるので、作業内容の表情は切り替えの間隔に下限を置いてちらつかせない。
  private baseExpression(now: number): string | null {
    const base = this.statusExpression(this.input.status);
    if (this.input.status !== "working") return base;
    const mapped = this.input.tool
      ? this.rules.workingTools.get(this.input.tool)
      : (this.rules.workingNoTool ?? undefined);
    const candidate = mapped && this.available(mapped) ? mapped : base;
    if (this.workingExpression === null) {
      this.workingExpression = candidate;
      this.workingSince = now;
    } else if (
      candidate !== this.workingExpression &&
      now - this.workingSince >= this.rules.minSwitchMs
    ) {
      this.workingExpression = candidate;
      this.workingSince = now;
    }
    return this.workingExpression;
  }

  private advanceGesture(now: number): void {
    const g = this.rules.idleGestures;
    const candidates = g?.gestures.filter((x) => this.available(x.expression)) ?? [];
    if (!g || this.input.status !== "idle" || candidates.length === 0) {
      this.gesture = null;
      this.nextGestureAt = null;
      return;
    }
    if (this.gesture && now >= this.gesture.until) {
      this.gesture = null;
      this.nextGestureAt = now + this.between(g.intervalMs);
    }
    if (this.gesture) return;
    if (this.nextGestureAt === null) {
      this.nextGestureAt = now + this.between(g.intervalMs);
      return;
    }
    if (now < this.nextGestureAt || this.hoverActive(now) || now < this.reactUntil) return;
    // 候補が複数あるときは、直前と同じ仕草が続かないようにする。
    const pool =
      candidates.length > 1 ? candidates.filter((x) => x.expression !== this.lastGesture) : candidates;
    const pick = pool[Math.floor(this.random() * pool.length)];
    this.gesture = { expression: pick.expression, until: now + this.between(pick.durationMs) };
    this.lastGesture = pick.expression;
  }

  private between([min, max]: [number, number]): number {
    return min + this.random() * (max - min);
  }
}
