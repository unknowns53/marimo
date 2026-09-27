// marimo-core の Snapshot を serde で直列化した形に合わせる。

export type Status = "idle" | "working" | "waiting" | "done" | "error";

export interface ContextUsage {
  used_percentage: number | null;
  total_input_tokens: number | null;
  context_window_size: number | null;
  updated_at: number;
  source?: string | null;
}

export type ActivityKind = "tool" | "message" | "error" | "text";

export interface Activity {
  kind: ActivityKind;
  tool?: string;
  summary: string;
  detail?: string;
}

export interface Origin {
  bundle_id?: string;
  term_program?: string;
  tty?: string;
  entrypoint?: string;
}

export interface SessionState {
  session_id: string;
  cwd: string | null;
  status: Status;
  status_since: number;
  started_at: number;
  status_reason?: string | null;
  turn_started_at?: number | null;
  activity: Activity | null;
  last_event: string | null;
  updated_at: number;
  context: ContextUsage | null;
  origin?: Origin | null;
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

// 分類名はドット区切りで、細かい分類（waiting.permission など）から親の分類（waiting）へ戻れる。
export type Dialogue = Record<string, string[]>;
