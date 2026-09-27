import { folderName } from "./format";
import type { Dialogue, SessionState, Status } from "./types";

const SHORT_MS = 2 * 60 * 1000;
const LONG_MS = 15 * 60 * 1000;

// StopFailure の error のうち利用制限を表すのは rate_limit だけである（hooks のドキュメントの
// StopFailure input に列挙された値のうち、overloaded はサーバー側の混雑、billing_error は請求の問題）。
const ERROR_CATEGORIES: Record<string, string> = { rate_limit: "error.rate_limit" };
const WAITING_CATEGORIES: Record<string, string> = {
  permission: "waiting.permission",
  question: "waiting.question",
  plan: "waiting.plan",
};

/** 完了までにかかった時間。ターンの開始時刻が分からなければ null。 */
export function turnDuration(s: SessionState): number | null {
  if (s.turn_started_at == null || s.status_since < s.turn_started_at) return null;
  return s.status_since - s.turn_started_at;
}

export function categoryFor(s: SessionState): string {
  const status: Status = s.status;
  if (status === "waiting") return WAITING_CATEGORIES[s.status_reason ?? ""] ?? "waiting";
  if (status === "error") return ERROR_CATEGORIES[s.status_reason ?? ""] ?? "error";
  if (status === "done") {
    const d = turnDuration(s);
    if (d === null) return "done";
    if (d < SHORT_MS) return "done.short";
    return d < LONG_MS ? "done" : "done.long";
  }
  return status;
}

export function reactionCategory(aggregate: Status): string {
  return aggregate === "working" ? "reaction.working" : "reaction";
}

/** 細かい分類にセリフがなければ、ドットで区切られた親の分類へ順に戻る。 */
export function linesFor(dialogue: Dialogue, category: string): string[] {
  for (let c: string | null = category; c; c = parent(c)) {
    const lines = dialogue[c];
    if (lines && lines.length > 0) return lines;
  }
  return [];
}

function parent(category: string): string | null {
  const i = category.lastIndexOf(".");
  return i > 0 ? category.slice(0, i) : null;
}

export function formatDuration(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "1 分足らず";
  if (minutes < 60) return `${minutes} 分`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} 時間` : `${hours} 時間 ${rest} 分`;
}

export function fillTemplate(template: string, s: SessionState | null): string {
  const d = s ? turnDuration(s) : null;
  return template
    .split("{folder}")
    .join(s ? folderName(s) : "")
    .split("{duration}")
    .join(d === null ? "" : formatDuration(d));
}

/** 分類ごとに、利用者のファイルにあればそれを、なければ組み込みの既定を使う。 */
export function mergeDialogue(defaults: Dialogue, user: unknown): Dialogue {
  const merged: Dialogue = { ...defaults };
  if (!user || typeof user !== "object" || Array.isArray(user)) return merged;
  for (const [key, value] of Object.entries(user)) {
    if (Array.isArray(value) && value.length > 0 && value.every((v) => typeof v === "string")) {
      merged[key] = value;
    }
  }
  return merged;
}
