// marimo-core の Snapshot を serde で直列化した形に合わせる。

export type Status = "idle" | "working" | "waiting" | "done" | "error";

// 以前の版のファイルは provider を持たず、Claude Code のセッションとして扱う。
export type Provider = "claude" | "codex";

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
  provider?: Provider;
  cwd: string | null;
  repo?: string;
  title?: string;
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
  own_status?: Status;
  own_activity?: Activity;
  own_status_reason?: string;
  agents?: Record<string, AgentRun>;
}

export interface AgentRun {
  agent_type?: string;
  started_at: number;
  last_seen: number;
  pending?: Activity;
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

// Codex は窓の長さを分で送ってくるので、5 時間や 7 日に決め打ちしない。
export interface CodexRateWindow {
  window_minutes: number | null;
  used_percentage: number;
  resets_at: number | null;
}

export interface CodexRateLimits {
  windows: CodexRateWindow[];
  plan_type?: string | null;
  observed_at: number;
  updated_at: number;
}

export interface Snapshot {
  aggregate: Status;
  sessions: SessionState[];
  rate_limits: RateLimits | null;
  codex_rate_limits?: CodexRateLimits | null;
}

// app_icons コマンドが返す PNG の data URL。アプリが入っていなければ null になる。
export type AppIcons = Record<Provider, string | null>;

// 分類名はドット区切りで、細かい分類（waiting.permission など）から親の分類（waiting）へ戻れる。
export type Dialogue = Record<string, string[]>;
