import type { SessionState } from "./types";

export function folderName(session: Pick<SessionState, "cwd" | "session_id">): string {
  const parts = (session.cwd ?? "").split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? session.session_id.slice(0, 8);
}

export function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k` : String(n);
}
