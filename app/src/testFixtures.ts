import type { SessionState, Snapshot, Status } from "./types";

const PRIORITY: Record<Status, number> = { idle: 0, working: 1, done: 2, error: 3, waiting: 4 };

export function session(id: string, status: Status, since: number, updated = since): SessionState {
  return {
    session_id: id,
    cwd: `/Users/me/MyApp/${id}`,
    status,
    status_since: since,
    activity: null,
    last_event: null,
    updated_at: updated,
    context: null,
  };
}

// marimo-core の load_snapshot と同じ並び（優先度の高い順、同じなら更新の新しい順）にする。
export function snap(...sessions: SessionState[]): Snapshot {
  const sorted = [...sessions].sort(
    (a, b) => PRIORITY[b.status] - PRIORITY[a.status] || b.updated_at - a.updated_at,
  );
  const aggregate = sorted[0]?.status ?? "idle";
  return { aggregate, sessions: sorted, rate_limits: null };
}
