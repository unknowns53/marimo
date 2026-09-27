import type { Status } from "./types";

export interface ExpressionAssets {
  image: string;
  blink?: string;
}

// manifest.json に書く形。表情の名前はここに書くだけで、コードには持たない。別のキャラクターの素材
// フォルダでも、manifest を差し替えれば同じ仕組みで動くようにするためである。
export interface StandingManifest {
  mode: "standing";
  name?: string;
  canvas: { width: number; height: number };
  /** 状態ごとの画像だけを持つ古い形式。expressions が無ければこれを使う。 */
  states?: Partial<Record<Status, ExpressionAssets>>;
  expressions?: Record<string, ExpressionAssets>;
  rules?: ManifestRules;
}

export interface ManifestRules {
  status?: Partial<Record<Status, string>>;
  working_tools?: { tools: string[]; expression: string }[];
  min_switch_ms?: number;
  idle_gestures?: {
    interval_ms: [number, number];
    gestures: { expression: string; duration_ms: [number, number] }[];
  };
  reaction?: { expression: string; duration_ms: number };
  hover?: Partial<Record<Status, string>>;
  hover_release_ms?: number;
}

export interface ExpressionRules {
  status: Partial<Record<Status, string>>;
  workingTools: Map<string, string>;
  minSwitchMs: number;
  idleGestures: {
    intervalMs: [number, number];
    gestures: { expression: string; durationMs: [number, number] }[];
  } | null;
  reaction: { expression: string; durationMs: number } | null;
  hover: Partial<Record<Status, string>>;
  hoverReleaseMs: number;
}

export interface Character {
  expressions: Record<string, ExpressionAssets>;
  rules: ExpressionRules;
}

const DEFAULT_MIN_SWITCH_MS = 4000;
const DEFAULT_HOVER_RELEASE_MS = 600;
const STATUSES: Status[] = ["idle", "working", "waiting", "done", "error"];

export function normalize(manifest: StandingManifest): Character {
  const legacy = !manifest.expressions;
  const expressions: Record<string, ExpressionAssets> = { ...(manifest.expressions ?? {}) };
  if (legacy) {
    for (const s of STATUSES) {
      const assets = manifest.states?.[s];
      if (assets) expressions[s] = assets;
    }
  }
  const r = manifest.rules ?? {};
  const status: Partial<Record<Status, string>> = {};
  for (const s of STATUSES) {
    const name = r.status?.[s] ?? (expressions[s] ? s : undefined);
    if (name) status[s] = name;
  }
  const workingTools = new Map<string, string>();
  for (const rule of r.working_tools ?? []) {
    for (const tool of rule.tools) workingTools.set(tool, rule.expression);
  }
  return {
    expressions,
    rules: {
      status,
      workingTools,
      minSwitchMs: r.min_switch_ms ?? DEFAULT_MIN_SWITCH_MS,
      idleGestures: r.idle_gestures
        ? {
            intervalMs: r.idle_gestures.interval_ms,
            gestures: r.idle_gestures.gestures.map((g) => ({
              expression: g.expression,
              durationMs: g.duration_ms,
            })),
          }
        : null,
      reaction: r.reaction
        ? { expression: r.reaction.expression, durationMs: r.reaction.duration_ms }
        : null,
      hover: r.hover ?? {},
      hoverReleaseMs: r.hover_release_ms ?? DEFAULT_HOVER_RELEASE_MS,
    },
  };
}
