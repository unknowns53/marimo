import type { SessionState, Status } from "./types";

type Named = Pick<SessionState, "cwd" | "session_id" | "repo" | "title" | "provider" | "cloud">;

// Codex のデスクトップアプリは、プロジェクトを選ばずに始めた会話ごとに ~/Documents/Codex/<日付>/<名前> の
// 作業フォルダを作り、その名前を最初のプロンプトから付ける。フォルダ名はタイトルの言い換えでしかないので、
// 行の名前にはタイトルを使う。
const CODEX_SCRATCH = /[\\/]Documents[\\/]Codex[\\/]\d{4}-\d{2}-\d{2}[\\/][^\\/]+[\\/]?$/;

export function isCodexScratch(session: Pick<SessionState, "cwd" | "provider">): boolean {
  return session.provider === "codex" && CODEX_SCRATCH.test(session.cwd ?? "");
}

// Hermes の gateway の会話は作業フォルダを持たない（gateway の作業フォルダは marimo-hook が捨てる）ので、
// チャンネル名などのタイトルを行の名前にする。CLI の会話は作業フォルダを持つので、ほかのツールと同じく扱う。
function isChatSession(session: Pick<SessionState, "cwd" | "provider">): boolean {
  return session.provider === "hermes" && !session.cwd;
}

// タイトルが行の名前の役をしているセッション。パネルは同じタイトルを名前の横に重ねて出さない。
export function titleIsName(session: Pick<SessionState, "cwd" | "provider" | "cloud">): boolean {
  return !!session.cloud || isCodexScratch(session) || isChatSession(session);
}

/** 件数の行やトレイのメニューで、状態を示す語。 */
export const STATUS_LABEL: Record<Status, string> = {
  idle: "待機",
  working: "作業中",
  waiting: "承認待ち",
  done: "完了",
  error: "エラー",
};

// 名前の横に添える会話のタイトル。repo が名前のときはタイトルが名前の役をしていないので、添える。
export function chatTitle(session: Named): string | null {
  return session.title && (session.repo || !titleIsName(session)) ? session.title : null;
}

export function folderName(session: Named): string {
  if (session.cloud) return `Cloud・手動 | ${session.title || session.cloud.thread_id.slice(0, 8)}`;
  if (session.repo) return session.repo;
  if (isCodexScratch(session)) return session.title || "Codex";
  if (isChatSession(session)) return session.title || "Hermes";
  const parts = (session.cwd ?? "").split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? session.session_id.slice(0, 8);
}

export function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k` : String(n);
}

export function formatClock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// パネルは 30 秒ごとにしか描き直さないので、1 分より細かくは出さない。
export function formatAge(updatedAt: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - updatedAt) / 60_000);
  if (minutes < 1) return "今";
  if (minutes < 60) return `${minutes}分前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}時間前`;
  return `${Math.floor(hours / 24)}日前`;
}
