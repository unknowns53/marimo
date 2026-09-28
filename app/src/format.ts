import type { SessionState } from "./types";

type Named = Pick<SessionState, "cwd" | "session_id" | "repo" | "title" | "provider">;

// Codex のデスクトップアプリは、プロジェクトを選ばずに始めた会話ごとに ~/Documents/Codex/<日付>/<名前> の
// 作業フォルダを作り、その名前を最初のプロンプトから付ける。フォルダ名は題名の言い換えでしかないので、
// 行の名前には題名を使う。
const CODEX_SCRATCH = /[\\/]Documents[\\/]Codex[\\/]\d{4}-\d{2}-\d{2}[\\/][^\\/]+[\\/]?$/;

export function isCodexScratch(session: Pick<SessionState, "cwd" | "provider">): boolean {
  return session.provider === "codex" && CODEX_SCRATCH.test(session.cwd ?? "");
}

export function folderName(session: Named): string {
  if (session.repo) return session.repo;
  if (isCodexScratch(session)) return session.title || "Codex";
  const parts = (session.cwd ?? "").split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? session.session_id.slice(0, 8);
}

export function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k` : String(n);
}
