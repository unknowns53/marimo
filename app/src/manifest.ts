import type { Manifest } from "./renderer";
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
  /** メニューにはこの名前を出し、無ければ name を使う。 */
  display_name?: string;
  /** ドット絵の素材なら true にし、拡大縮小しても画素をぼかさずに描かせる。 */
  pixelated?: boolean;
  canvas: { width: number; height: number };
  /** 状態ごとの画像だけを持つ古い形式。expressions が無ければこれを使う。 */
  states?: Partial<Record<Status, ExpressionAssets>>;
  expressions?: Record<string, ExpressionAssets>;
  rules?: ManifestRules;
}

export interface ManifestRules {
  status?: Partial<Record<Status, string>>;
  working_tools?: { tools: string[]; expression: string; max_ms?: number }[];
  working_no_tool?: string;
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
  /** 作業内容の表情ごとの、続けて見せる時間の上限。過ぎたら作業中の基本の表情へ戻す。 */
  workingMaxMs: Map<string, number>;
  workingNoTool: string | null;
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

/** index.json を読めないときにも、この名前の素材を試す。 */
export const DEFAULT_CHARACTER = "koharu";
// 縦横比の書かれていない素材の枠は、既定のキャラクターと同じにする。scale.rs の DEFAULT_ASPECT と揃える。
const DEFAULT_ASPECT = 1.5;

export interface CharacterInfo {
  id: string;
  displayName: string;
  pixelated: boolean;
  /** 縦横比は高さ ÷ 幅で持つ。枠の幅は固定なので、高さをこれで決める。 */
  aspect: number;
}

export function characterInfo(id: string, manifest: Manifest): CharacterInfo {
  const size = manifest.mode === "sprite" ? manifest.frame : manifest.canvas;
  const aspect = size.width > 0 && size.height > 0 ? size.height / size.width : DEFAULT_ASPECT;
  const name = manifest.display_name?.trim() || manifest.name?.trim() || id;
  return { id, displayName: name, pixelated: manifest.pixelated === true, aspect };
}

/**
 * 保存されたキャラクターを読めなかったときに立ち絵の無い状態で起動しないよう、一覧の先頭（既定の
 * キャラクター）を後に続ける。
 */
export function characterCandidates(saved: string | null, index: unknown): string[] {
  const ids = Array.isArray(index) ? index.filter((x): x is string => typeof x === "string") : [];
  const fallback = ids[0] ?? DEFAULT_CHARACTER;
  const first = saved && ids.includes(saved) ? saved : fallback;
  return first === fallback ? [first] : [first, fallback];
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
  const workingMaxMs = new Map<string, number>();
  for (const rule of r.working_tools ?? []) {
    for (const tool of rule.tools) workingTools.set(tool, rule.expression);
    if (rule.max_ms != null) workingMaxMs.set(rule.expression, rule.max_ms);
  }
  return {
    expressions,
    rules: {
      status,
      workingTools,
      workingMaxMs,
      workingNoTool: r.working_no_tool ?? null,
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
