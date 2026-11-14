import type { CodexRateLimits, LimitTool, RateLimits } from "./types";

// statusLine はアシスタントの応答ごとに走り、Codex の値も応答ごとに rollout へ書かれるので、
// これより古い値はローカルの作業が止まっている間に実際の値から離れている可能性が高い。
// Antigravity の値は再取得のときにしか届かないが、同じ基準で薄くする。
export const RATE_STALE_MS = 30 * 60 * 1000;

export interface LimitGroup {
  provider: LimitTool;
  text: string;
  /** 値が最後に届いた時刻（Unix ミリ秒）。Codex では rollout の行の時刻を使う。 */
  updatedAt: number;
  stale: boolean;
}

export interface LimitLine {
  groups: LimitGroup[];
  /** Claude Code 以外の値があるときだけ、どのツールの値かをアイコンで示す。無ければ以前のバージョンと同じ見た目にする。 */
  marked: boolean;
  /** 出すグループのうち最も新しい更新の時刻。古いグループはグループごとに薄く示すので、行の時刻は最後に値が届いた時刻にする。 */
  updatedAt: number;
}

/** Codex のウィンドウの長さを 5h や 7d のような短い名前にする。長さが分からないウィンドウには名前を付けない。 */
export function windowLabel(minutes: number | null): string | null {
  if (minutes == null || minutes <= 0) return null;
  if (minutes % 1440 === 0) return `${minutes / 1440}d`;
  if (minutes % 60 === 0) return `${minutes / 60}h`;
  return `${minutes}m`;
}

// リセット時刻を過ぎたウィンドウの値はもう意味を持たないので出さない。
function live(resetsAt: number | null, now: number): boolean {
  return resetsAt == null || resetsAt * 1000 > now;
}

function percent(label: string | null, used: number): string {
  const value = `${Math.round(used)}%`;
  return label ? `${label} ${value}` : value;
}

/** 出さないツールの値は、呼ぶ側が null にして渡す。 */
export interface LimitSources {
  claude: RateLimits | null;
  codex: CodexRateLimits | null;
  antigravity: RateLimits | null;
}

// Claude Code と Antigravity は 5 時間と 7 日のウィンドウを決まった名前で持つ。
function fixedWindows(rl: RateLimits, now: number): string[] {
  const windows = [
    ["5h", rl.five_hour],
    ["7d", rl.seven_day],
  ] as const;
  return windows.flatMap(([label, w]) => (w && live(w.resets_at, now) ? [percent(label, w.used_percentage)] : []));
}

export function limitLine({ claude: rl, codex, antigravity }: LimitSources, now: number): LimitLine | null {
  const groups: LimitGroup[] = [];
  const add = (provider: LimitTool, parts: string[], updatedAt: number) => {
    if (parts.length > 0) {
      groups.push({ provider, text: parts.join(" · "), updatedAt, stale: now - updatedAt > RATE_STALE_MS });
    }
  };
  if (rl) add("claude", fixedWindows(rl, now), rl.updated_at);
  if (codex) {
    add(
      "codex",
      codex.windows
        .filter((w) => live(w.resets_at, now))
        .map((w) => percent(windowLabel(w.window_minutes), w.used_percentage)),
      codex.observed_at,
    );
  }
  if (antigravity) add("antigravity", fixedWindows(antigravity, now), antigravity.updated_at);
  if (groups.length === 0) return null;
  return {
    groups,
    marked: codex != null || antigravity != null,
    updatedAt: Math.max(...groups.map((g) => g.updatedAt)),
  };
}
