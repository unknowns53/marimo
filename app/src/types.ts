// marimo-core の Snapshot を serde で直列化した形に合わせる。

export type Status = "idle" | "working" | "waiting" | "done" | "error";

export interface ContextUsage {
  used_percentage: number | null;
  total_input_tokens: number | null;
  context_window_size: number | null;
  updated_at: number;
}

export interface SessionState {
  session_id: string;
  cwd: string | null;
  status: Status;
  line: string | null;
  last_event: string | null;
  updated_at: number;
  context: ContextUsage | null;
}

export interface RateWindow {
  used_percentage: number;
  resets_at: number | null;
}

export interface RateLimits {
  five_hour: RateWindow | null;
  seven_day: RateWindow | null;
  updated_at: number;
}

export interface Snapshot {
  aggregate: Status;
  sessions: SessionState[];
  rate_limits: RateLimits | null;
}

export type Dialogue = Partial<Record<Status, string[]>>;
