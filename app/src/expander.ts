import type { Rect } from "./hitArea";

export interface Point {
  x: number;
  y: number;
}

export interface ExpandTarget {
  id: string;
  /** 畳んだ状態の行（件数の行）。ここにとどまると開く。 */
  anchor: Rect;
  /** 開いたときの層。開いている間、anchor と合わせた範囲にカーソルがあれば開いたままにする。 */
  layer: Rect | null;
}

export interface ExpanderTiming {
  openDelayMs: number;
  closeDelayMs: number;
}

export const DEFAULT_TIMING: ExpanderTiming = { openDelayMs: 100, closeDelayMs: 300 };

export function contains(r: Rect, p: Point): boolean {
  return p.x >= r.x && p.y >= r.y && p.x < r.x + r.w && p.y < r.y + r.h;
}

// 行と層の間に隙間があっても、そこを通る間に閉じないよう、両方を囲む矩形で判定する。
export function union(a: Rect, b: Rect | null): Rect {
  if (!b) return a;
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.max(a.x + a.w, b.x + b.w) - x, h: Math.max(a.y + a.h, b.y + b.h) - y };
}

/**
 * 畳んだ行を広げるかどうかを、時刻とカーソル位置と領域から決める。DOM のホバーには頼らない。
 * 窓は透明な部分でマウスのイベントを受け取らないので、層が広がった直後や層へ移る途中に DOM が
 * mouseleave と判断してしまい、開閉を繰り返すからである。
 * 横切っただけで開かないよう、行に openDelayMs とどまってから開き、開いた範囲を出ても
 * closeDelayMs たつまでは閉じない。
 */
export class Expander {
  // outsideSince は、開いた範囲の外で最初にカーソルを見た時刻。閉じるまでの時間はここから数える。
  private open: { id: string; outsideSince: number | null } | null = null;
  private pending: { id: string; since: number } | null = null;

  constructor(private readonly timing: ExpanderTiming = DEFAULT_TIMING) {}

  get openId(): string | null {
    return this.open?.id ?? null;
  }

  /** 開いている層の範囲（行と層を囲む矩形）。親の層を開いたままにする判定にも使う。 */
  keepRegion(targets: ExpandTarget[]): Rect | null {
    const t = this.open && targets.find((x) => x.id === this.open?.id);
    return t ? union(t.anchor, t.layer) : null;
  }

  /**
   * extraKeep は、この層の中でさらに開いている子の層の範囲。子の層が親の外へ伸びても、
   * その上にカーソルがある間は親を閉じない。
   */
  update(now: number, cursor: Point | null, targets: ExpandTarget[], extraKeep: Rect | null = null): string | null {
    if (this.open) {
      const t = targets.find((x) => x.id === this.open?.id);
      const inside =
        !!t && !!cursor && (contains(union(t.anchor, t.layer), cursor) || (!!extraKeep && contains(extraKeep, cursor)));
      if (!t) {
        this.open = null;
      } else if (inside) {
        this.open.outsideSince = null;
        this.pending = null;
        return this.open.id;
      } else {
        this.open.outsideSince ??= now;
        if (now - this.open.outsideSince < this.timing.closeDelayMs) return this.open.id;
        this.open = null;
      }
    }
    const hovered = cursor ? targets.find((x) => contains(x.anchor, cursor)) : undefined;
    if (!hovered) {
      this.pending = null;
      return null;
    }
    if (this.pending?.id !== hovered.id) {
      this.pending = { id: hovered.id, since: now };
    }
    if (now - this.pending.since >= this.timing.openDelayMs) {
      this.open = { id: hovered.id, outsideSince: null };
      this.pending = null;
      return hovered.id;
    }
    return null;
  }

  /** 次に判定をやり直すべき時刻。カーソルが止まっていても、開く・閉じるの時間切れを拾うため。 */
  nextDeadline(): number | null {
    if (this.pending) return this.pending.since + this.timing.openDelayMs;
    if (this.open?.outsideSince != null) return this.open.outsideSince + this.timing.closeDelayMs;
    return null;
  }
}
