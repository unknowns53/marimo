import type { CodexRateLimits, Provider, RateLimits } from "./types";

// statusLine はアシスタントの応答ごとに走り、Codex の値も応答ごとに rollout へ書かれるので、
// これより古い値は手元の作業が止まっている間に実際の値から離れている可能性が高い。
export const RATE_STALE_MS = 30 * 60 * 1000;

export interface LimitGroup {
  provider: Provider;
  text: string;
  /** 値が最後に届いた時刻（Unix ミリ秒）。Codex では rollout の行の時刻を使う。 */
  updatedAt: number;
  stale: boolean;
}

export interface LimitLine {
  groups: LimitGroup[];
  /** Codex の値があるときだけ、どちらのツールの値かをアイコンで示す。無ければ以前の版と同じ見た目にする。 */
  marked: boolean;
  /** 出す組のうち最も新しい更新の時刻。古い組は組ごとに薄く示すので、行の時刻は最後に値が届いた時刻にする。 */
  updatedAt: number;
}

/** Codex の窓の長さを 5h や 7d のような短い名前にする。長さが分からない窓には名前を付けない。 */
export function windowLabel(minutes: number | null): string | null {
  if (minutes == null || minutes <= 0) return null;
  if (minutes % 1440 === 0) return `${minutes / 1440}d`;
  if (minutes % 60 === 0) return `${minutes / 60}h`;
  return `${minutes}m`;
}

// リセット時刻を過ぎた窓の値はもう意味を持たないので出さない。
function live(resetsAt: number | null, now: number): boolean {
  return resetsAt == null || resetsAt * 1000 > now;
}

function percent(label: string | null, used: number): string {
  const value = `${Math.round(used)}%`;
  return label ? `${label} ${value}` : value;
}

export function limitLine(rl: RateLimits | null, codex: CodexRateLimits | null, now: number): LimitLine | null {
  const groups: LimitGroup[] = [];
  const add = (provider: Provider, parts: string[], updatedAt: number) => {
    if (parts.length > 0) {
      groups.push({ provider, text: parts.join(" · "), updatedAt, stale: now - updatedAt > RATE_STALE_MS });
    }
  };
  if (rl) {
    const claude = [
      ["5h", rl.five_hour],
      ["7d", rl.seven_day],
    ] as const;
    add(
      "claude",
      claude.flatMap(([label, w]) => (w && live(w.resets_at, now) ? [percent(label, w.used_percentage)] : [])),
      rl.updated_at,
    );
  }
  if (codex) {
    add(
      "codex",
      codex.windows
        .filter((w) => live(w.resets_at, now))
        .map((w) => percent(windowLabel(w.window_minutes), w.used_percentage)),
      codex.observed_at,
    );
  }
  if (groups.length === 0) return null;
  return { groups, marked: codex != null, updatedAt: Math.max(...groups.map((g) => g.updatedAt)) };
}
