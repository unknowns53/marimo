use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::activity::{self, Activity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Idle,
    Working,
    Waiting,
    Done,
    Error,
}

impl Status {
    /// 承認待ち、エラー、作業中、完了、待機の順に高い。完了を作業中より下に置くのは、作業を見ている間に
    /// 他のセッションが終わるたびに表情が切り替わらないようにするためで、完了はパネルの行の色で知らせる。
    pub fn priority(self) -> u8 {
        match self {
            Status::Idle => 0,
            Status::Done => 1,
            Status::Working => 2,
            Status::Error => 3,
            Status::Waiting => 4,
        }
    }
}

/// フックを送ってきたツール。この項目を持たない以前の版のファイルは Claude Code のものとして読む。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
}

pub fn aggregate<I: IntoIterator<Item = Status>>(statuses: I) -> Status {
    statuses
        .into_iter()
        .max_by_key(|s| s.priority())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextUsage {
    #[serde(default)]
    pub used_percentage: Option<f64>,
    #[serde(default)]
    pub total_input_tokens: Option<u64>,
    #[serde(default)]
    pub context_window_size: Option<u64>,
    pub updated_at: u64,
    /// 値の出どころ。statusLine のほかに会話ログから計算した値も入りうるので、
    /// 読む側が新しさや確からしさを比べられるようにしておく。
    #[serde(default)]
    pub source: Option<String>,
}

/// セッションを起動したアプリの手がかり。行を押したときに、そのアプリやタブへ移動するのに使う。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    /// macOS がアプリから起動したプロセスに渡す `__CFBundleIdentifier`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_program: Option<String>,
    /// `/dev/ttys003` の形。フックは制御端末を持たずに動くので、祖先のプロセスから得る。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    /// 会話ログの各行にある `entrypoint`。値の意味が確かめられるまでは、移動の判断には使わず記録だけする。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// Windows で、フックの親のコンソールからたどった最上位のウィンドウ。Windows でだけ記録する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowRef>,
    /// Windows で、フックの祖先のプロセスを近い順に最大 8 個。フック自身は含めない。Windows でだけ記録する。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ancestors: Vec<ProcessRef>,
}

/// Windows のウィンドウと、それを持つプロセス。ウィンドウのハンドルとプロセス ID はどちらも使い回されるので、
/// プロセスの作成時刻と組にして、移動するときに同じものかを確かめる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRef {
    pub hwnd: u64,
    pub pid: u32,
    /// GetProcessTimes の作成時刻（FILETIME）を 64 ビットの整数にしたもの。
    pub created: u64,
}

/// Windows のプロセス。プロセス ID は使い回されるので、作成時刻と組にして同じものかを確かめる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessRef {
    pub pid: u32,
    /// GetProcessTimes の作成時刻（FILETIME）を 64 ビットの整数にしたもの。
    pub created: u64,
    /// 実行ファイルの完全なパス。
    pub exe: String,
}

impl Origin {
    pub fn is_empty(&self) -> bool {
        !self.has_env_clues() && !self.has_window_clues() && self.entrypoint.is_none()
    }

    fn has_env_clues(&self) -> bool {
        self.bundle_id.is_some() || self.term_program.is_some() || self.tty.is_some()
    }

    fn has_window_clues(&self) -> bool {
        self.window.is_some() || !self.ancestors.is_empty()
    }

    // CLI で --resume すると、デスクトップアプリの会話と同じ session_id のまま別のアプリへ移る。
    // 環境変数と端末の手がかりは毎回の実行から取り直し、得られたときだけ置き換える。
    // Windows のウィンドウと祖先のプロセスは、調べるのに時間がかかるので一部のイベントでしか取らない。
    // それを含む実行からは全体を置き換え、環境変数だけの実行ではウィンドウと祖先を前の値のまま残す。
    // 会話ログの entrypoint は読める契機が限られるので、新しい値がなければ前の値を残す。
    pub fn merge(
        existing: Option<&Origin>,
        process: Option<&Origin>,
        entrypoint: Option<&str>,
    ) -> Option<Origin> {
        let previous = existing.cloned().unwrap_or_default();
        let mut next = match process {
            Some(p) if p.has_window_clues() => Origin {
                entrypoint: previous.entrypoint,
                ..p.clone()
            },
            Some(p) if p.has_env_clues() => Origin {
                bundle_id: p.bundle_id.clone(),
                term_program: p.term_program.clone(),
                tty: p.tty.clone(),
                ..previous
            },
            _ => previous,
        };
        if let Some(e) = entrypoint {
            next.entrypoint = Some(e.to_owned());
        }
        (!next.is_empty()).then_some(next)
    }
}

/// SubagentStop は、サブエージェントが失敗したときや取り消されたときに届くとは hooks のドキュメントに
/// 書かれていない。届かないまま作業中に見え続けないよう、最後のイベントからこれだけ経ったサブエージェントは外す。
pub const AGENT_STALE_MS: u64 = 30 * 60 * 1000;

// statusLine は応答ごとに走るので、この時間より新しい値があれば会話ログからの計算より優先する。
const STATUSLINE_FRESH_MS: u64 = 5 * 60 * 1000;

/// Codex の rollout の token_count から読んだ値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexTokenUsage {
    /// 最後の API 呼び出しの total_tokens。
    pub last_total_tokens: i64,
    pub model_context_window: Option<i64>,
}

// codex-rs/protocol/src/protocol.rs の BASELINE_TOKENS。システムプロンプトやツールの説明のように
// 常にコンテキストにある分として、Codex が % の分子と分母の両方から引く。
const CODEX_BASELINE_TOKENS: i64 = 12000;

/// Codex の TUI の表示と同じ % にする。TUI は codex-rs/tui/src/chatwidget.rs の
/// context_remaining_percent で last_token_usage の percent_of_context_window_remaining
/// （codex-rs/protocol/src/protocol.rs）を残りの % として出すので、同じ式で残りを求め、
/// 100 から引いて使用中の % にする。
pub fn context_from_codex(usage: &CodexTokenUsage, now_ms: u64) -> ContextUsage {
    let used_percentage = usage.model_context_window.map(|window| {
        let remaining = if window <= CODEX_BASELINE_TOKENS {
            0
        } else {
            let effective = window - CODEX_BASELINE_TOKENS;
            let used = (usage.last_total_tokens - CODEX_BASELINE_TOKENS).max(0);
            let left = (effective - used).max(0);
            ((left as f64 / effective as f64) * 100.0)
                .clamp(0.0, 100.0)
                .round() as i64
        };
        (100 - remaining) as f64
    });
    ContextUsage {
        used_percentage,
        total_input_tokens: u64::try_from(usage.last_total_tokens).ok(),
        context_window_size: usage
            .model_context_window
            .and_then(|w| u64::try_from(w).ok()),
        updated_at: now_ms,
        source: Some("codex-rollout".to_owned()),
    }
}

/// 会話ログから数えたトークン数で、保存済みのコンテキスト使用量を更新する。上限は statusLine から
/// 得た値だけを使い、無ければ % を出さない。モデル名から上限を推し量ると、1M の版かどうかが
/// 分からず誤った % を出すからである。
pub fn context_from_transcript(
    existing: Option<&ContextUsage>,
    tokens: u64,
    now_ms: u64,
) -> Option<ContextUsage> {
    if let Some(e) = existing
        && e.source.as_deref() == Some("statusline")
        && e.used_percentage.is_some()
        && now_ms.saturating_sub(e.updated_at) < STATUSLINE_FRESH_MS
    {
        return None;
    }
    let window = existing
        .and_then(|e| e.context_window_size)
        .filter(|w| *w > 0);
    Some(ContextUsage {
        used_percentage: window.map(|w| tokens as f64 / w as f64 * 100.0),
        total_input_tokens: Some(tokens),
        context_window_size: window,
        updated_at: now_ms,
        source: Some("transcript".to_owned()),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    /// セッションは (provider, session_id) で一意になる。二つのツールの session_id は別々に振られる。
    #[serde(default)]
    pub provider: Provider,
    #[serde(default)]
    pub cwd: Option<String>,
    /// cwd を含む git リポジトリの名前。linked worktree では元のリポジトリの名前になる。
    /// パネルの行を worktree のフォルダ名ではなくリポジトリで見分けられるようにする。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// 会話の題名。statusLine の session_name か、会話ログの custom-title の行から取る。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Status,
    /// 今の status になった時刻。吹き出しが「同じ状態が続いている間」を見分けるのに使う。
    #[serde(default)]
    pub status_since: u64,
    /// marimo が最初にこのセッションのファイルを書いた時刻。状態が変わっても動かないので、パネルの行を
    /// セッションごとに同じ位置へ並べ続けるのに使う。この項目を持たない古いファイルでは 0 で、次に書き直すときに埋める。
    #[serde(default)]
    pub started_at: u64,
    /// 状態になった理由の手がかり。承認待ちでは permission、question、plan のどれか、エラーでは
    /// StopFailure の error の値（hooks のドキュメントの StopFailure input に列挙がある）を入れる。
    /// 吹き出しのセリフを状況に合わせて選ぶのに使う。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// 今のターンが始まった時刻（UserPromptSubmit を受けた時刻）。完了までにかかった時間を出すのに使う。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_started_at: Option<u64>,
    #[serde(
        default,
        alias = "line",
        deserialize_with = "activity::deserialize_compat"
    )]
    pub activity: Option<Activity>,
    #[serde(default)]
    pub last_event: Option<String>,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub context: Option<ContextUsage>,
    #[serde(default)]
    pub origin: Option<Origin>,
    /// 親の会話だけで決まる状態。status、activity、status_reason は、これに動いているサブエージェントを
    /// 重ねて表示用に組み立てた値である。サブエージェントのフックは親と同じ session_id で届くので、
    /// 分けて持たないと親が終えた後もサブエージェントのイベントで作業中へ戻ってしまう。
    /// この項目を持たない古いファイルでは、表示用の値を親の会話の状態として読む。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_activity: Option<Activity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_status_reason: Option<String>,
    /// 動いているサブエージェント。キーは agent_id。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    pub started_at: u64,
    /// このサブエージェントのイベントを最後に受けた時刻。SubagentStop が届かなかったものを外すのに使う。
    pub last_seen: u64,
    /// 利用者の承認を待っているツールの要約。PermissionRequest で入れ、このサブエージェントのツールの結果か拒否が届いたら消す。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Activity>,
}

impl SessionState {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            provider: Provider::Claude,
            cwd: None,
            repo: None,
            title: None,
            status: Status::Idle,
            status_since: 0,
            started_at: 0,
            status_reason: None,
            turn_started_at: None,
            activity: None,
            last_event: None,
            updated_at: 0,
            context: None,
            origin: None,
            own_status: None,
            own_activity: None,
            own_status_reason: None,
            agents: BTreeMap::new(),
        }
    }

    // own_status を持たない古いファイルの表示用の値を、親の会話の状態として引き継ぐ。
    fn fill_own(&mut self) {
        if self.own_status.is_none() {
            self.own_status = Some(self.status);
            self.own_activity = self.activity.clone();
            self.own_status_reason = self.status_reason.clone();
        }
    }

    fn own(&self) -> Status {
        self.own_status.unwrap_or(self.status)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateWindow {
    pub used_percentage: f64,
    /// statusLine の `resets_at` と同じく Unix 秒で持つ。
    #[serde(default)]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimits {
    #[serde(default)]
    pub five_hour: Option<RateWindow>,
    #[serde(default)]
    pub seven_day: Option<RateWindow>,
    pub updated_at: u64,
}

/// Codex の利用制限の窓。Codex は窓の長さを値として送ってくるので、5 時間や 7 日に決め打ちせず、
/// 送られてきた長さのまま持つ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexRateWindow {
    #[serde(default)]
    pub window_minutes: Option<u64>,
    pub used_percentage: f64,
    /// Unix 秒。
    #[serde(default)]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexRateLimits {
    pub windows: Vec<CodexRateWindow>,
    #[serde(default)]
    pub plan_type: Option<String>,
    /// 値を読んだ rollout の行の timestamp（Unix ミリ秒）。複数のセッションの rollout の
    /// 末尾はそれぞれ別の時点の値を持つので、これで新しさを比べる。
    pub observed_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub aggregate: Status,
    /// 優先度の高い順に並べ、同じ優先度では更新の新しい順に並べる。
    pub sessions: Vec<SessionState>,
    pub rate_limits: Option<RateLimits>,
    #[serde(default)]
    pub codex_rate_limits: Option<CodexRateLimits>,
}

/// 項目名は https://code.claude.com/docs/en/hooks の Common input fields と、
/// 各イベントの input の節に従う。Codex のフックの入力も同じ名前の項目を持つ
/// （codex-rs/hooks/src/schema.rs の各 CommandInput）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookInput {
    /// 入力には含まれず、どのサブコマンドで受けたかで決まる。
    #[serde(skip)]
    pub provider: Provider,
    pub session_id: String,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    #[serde(default)]
    pub last_assistant_message: Option<String>,
    #[serde(default)]
    pub notification_type: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
}

impl HookInput {
    /// サブエージェントの中で発火したフックなら、その agent_id を返す。hooks のドキュメントの
    /// Common input fields によると、agent_id はサブエージェントの中で発火したときだけ入る。
    /// agent_type は `--agent` で起動した親の会話にも入るので、見分けには使わない。
    pub fn subagent(&self) -> Option<&str> {
        self.agent_id.as_deref().filter(|a| !a.is_empty())
    }

    /// Codex の会話の題名を session_index.jsonl から取り直す契機。題名は利用者が付け直すか、
    /// Codex が最初のやりとりの後に付けるので、ターンの始まりと終わりで見れば足りる。
    pub fn wants_codex_title(&self) -> bool {
        self.provider == Provider::Codex
            && self.subagent().is_none()
            && matches!(
                self.hook_event_name.as_str(),
                "SessionStart" | "UserPromptSubmit" | "Stop"
            )
    }

    // ツールの実行ごとに API の呼び出しが一回挟まるので、PostToolUse と Stop で数え直せば
    // 使用量の変化に追いつける。サブエージェントのツールの実行は親のコンテキストの使用量を変えないので数えない。
    // Codex では resume した会話の使用量と利用制限をすぐ出せるよう、SessionStart でも読む。
    pub fn wants_transcript_usage(&self) -> bool {
        let events: &[&str] = match self.provider {
            Provider::Claude => &["PostToolUse", "Stop"],
            Provider::Codex => &["PostToolUse", "Stop", "SessionStart"],
        };
        self.subagent().is_none()
            && events.contains(&self.hook_event_name.as_str())
            && self
                .transcript_path
                .as_deref()
                .is_some_and(|p| !p.is_empty())
    }
}

/// 保存する題名の長さの上限。パネルでは一行に切り詰めて見せるので、これを超える部分は使わない。
pub const TITLE_MAX_CHARS: usize = 200;

/// 前後の空白を除き、長すぎる題名を文字の境目で切る。空なら None を返す。
pub fn clean_title(raw: &str) -> Option<String> {
    let t = raw.trim();
    (!t.is_empty()).then(|| t.chars().take(TITLE_MAX_CHARS).collect())
}

/// 親の会話のイベントで cwd が変わったときか、まだリポジトリ名を持たないときだけ `lookup` で
/// 調べ直し、next.repo を置き換える。フックのたびにファイルを調べないためである。
/// サブエージェントのイベントは cwd を変えないので、リポジトリ名も変えない。
pub fn refresh_repo(
    input: &HookInput,
    current: Option<&SessionState>,
    next: &mut SessionState,
    lookup: impl FnOnce(&str) -> Option<String>,
) {
    if input.subagent().is_some() {
        return;
    }
    let Some(cwd) = next.cwd.as_deref().filter(|c| !c.is_empty()) else {
        return;
    };
    let moved = current.and_then(|c| c.cwd.as_deref()) != Some(cwd);
    if moved || next.repo.is_none() {
        next.repo = lookup(cwd);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    Write(Box<SessionState>),
    Delete,
    Nothing,
}

enum Act {
    Keep,
    Clear,
    Set(Activity),
}

pub fn transition(input: &HookInput, current: Option<&SessionState>, now_ms: u64) -> Transition {
    let next = match input.subagent() {
        Some(agent) => subagent_update(input, agent, current, now_ms),
        None => match main_update(input, current, now_ms) {
            Ok(next) => Some(next),
            Err(t) => return t,
        },
    };
    let Some(mut next) = next else {
        return Transition::Nothing;
    };
    next.provider = input.provider;
    if next.started_at == 0 {
        next.started_at = now_ms;
    }
    settle(&mut next, current.map(|c| c.status), now_ms, now_ms);
    next.updated_at = now_ms;
    Transition::Write(Box::new(next))
}

// 親の会話のイベントで own_status と own_activity を決める。表示用の値は settle が組み立てる。
fn main_update(
    input: &HookInput,
    current: Option<&SessionState>,
    now_ms: u64,
) -> Result<SessionState, Transition> {
    let cur_status = current.map(SessionState::own);
    let cwd = input
        .cwd
        .as_deref()
        .or_else(|| current.and_then(|c| c.cwd.as_deref()));
    let tool = || {
        let name = input.tool_name.as_deref().unwrap_or("");
        (
            name,
            activity::tool_activity(name, input.tool_input.as_ref(), cwd),
        )
    };
    let agent_pending = current.is_some_and(|c| c.agents.values().any(|a| a.pending.is_some()));
    let (status, act) = match input.hook_event_name.as_str() {
        "SessionEnd" => return Err(Transition::Delete),
        // compaction が作業の途中で起きても状態を戻さないよう、compact だけは現状を保つ。
        "SessionStart" if input.source.as_deref() == Some("compact") => {
            (cur_status.unwrap_or_default(), Act::Keep)
        }
        "SessionStart" => (Status::Idle, Act::Clear),
        "UserPromptSubmit" => (Status::Working, Act::Clear),
        "PreToolUse" => {
            let (name, a) = tool();
            // これらのツールは、呼ばれた時点でユーザーの回答や承認を待つ。request_user_input は
            // Codex の質問のツールである。Codex の request_permissions は承認を求めても PermissionRequest を
            // 通らない（codex-rs/core/src/session/mod.rs の request_permissions_for_environment が
            // request_guardian_approval を直接呼ぶ）ので、呼ばれた時点で承認待ちにする。
            if matches!(
                name,
                "AskUserQuestion" | "ExitPlanMode" | "request_user_input" | "request_permissions"
            ) {
                (Status::Waiting, Act::Set(a))
            } else {
                (Status::Working, Act::Set(a))
            }
        }
        "PermissionRequest" => (Status::Waiting, Act::Set(tool().1)),
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" | "ElicitationResult" => {
            (Status::Working, Act::Keep)
        }
        "Elicitation" => (Status::Waiting, Act::Keep),
        "Notification" => match input.notification_type.as_deref() {
            // Notification の入力に agent_id が入るとはドキュメントに書かれていない。親の会話が承認を
            // 待っていないのにサブエージェントが承認を待っているなら、この通知はサブエージェントの承認の
            // 画面のものなので、親の会話の状態にしない。
            Some("permission_prompt") if agent_pending && cur_status != Some(Status::Waiting) => {
                return Err(Transition::Nothing);
            }
            Some("permission_prompt" | "elicitation_dialog" | "elicitation_url_dialog") => {
                (Status::Waiting, Act::Keep)
            }
            Some("quota_auto_resume_fired") => (Status::Working, Act::Keep),
            // idle_prompt は応答を終えて 60 秒ほど経ったときに届く。ユーザーの割り込みで
            // Stop が発火せず作業中のまま残ったセッションを、ここで待機へ戻す。
            Some("idle_prompt") if cur_status == Some(Status::Working) => (Status::Idle, Act::Keep),
            _ => return Err(Transition::Nothing),
        },
        // 応答の文章が流れるたびに届く。状態は変えず、いま表示された最後の行を一行表示に使う。
        "MessageDisplay" => {
            let last = input
                .delta
                .as_deref()
                .and_then(|d| d.lines().rev().find(|l| !l.trim().is_empty()))
                .and_then(Activity::message);
            match last {
                Some(a) => (cur_status.unwrap_or_default(), Act::Set(a)),
                None => return Err(Transition::Nothing),
            }
        }
        "Stop" => {
            let act = match input
                .last_assistant_message
                .as_deref()
                .and_then(Activity::message)
            {
                Some(a) => Act::Set(a),
                None => Act::Keep,
            };
            (Status::Done, act)
        }
        // Codex で利用者がターンを中断したときに届く。中断では Stop が発火しないので、ここで待機へ戻す。
        "Interrupt" => (Status::Idle, Act::Keep),
        "StopFailure" => {
            let text = input
                .last_assistant_message
                .as_deref()
                .or(input.error.as_deref())
                .unwrap_or_default();
            (Status::Error, Act::Set(Activity::error(text)))
        }
        _ => return Err(Transition::Nothing),
    };

    let reason: Option<String> = match input.hook_event_name.as_str() {
        "PermissionRequest" => Some("permission".to_owned()),
        "PreToolUse" => match input.tool_name.as_deref() {
            Some("AskUserQuestion" | "request_user_input") => Some("question".to_owned()),
            Some("ExitPlanMode") => Some("plan".to_owned()),
            Some("request_permissions") => Some("permission".to_owned()),
            _ => None,
        },
        "StopFailure" => input.error.clone().filter(|e| !e.is_empty()),
        _ => None,
    };

    let mut next = current
        .cloned()
        .unwrap_or_else(|| SessionState::new(&input.session_id));
    next.fill_own();
    // 同じ状態のまま理由を持たないイベント（承認待ちの Notification など）が来ても、先に分かった理由を残す。
    if cur_status != Some(status) || reason.is_some() {
        next.own_status_reason = reason;
    }
    match input.hook_event_name.as_str() {
        "UserPromptSubmit" => next.turn_started_at = Some(now_ms),
        // startup、resume、clear では Claude Code のプロセスか会話が新しくなり、前のサブエージェントは続かない。
        "SessionStart" if input.source.as_deref() != Some("compact") => {
            next.turn_started_at = None;
            next.agents.clear();
        }
        _ => {}
    }
    next.own_status = Some(status);
    match act {
        Act::Keep => {}
        Act::Clear => next.own_activity = None,
        Act::Set(a) if a.summary.is_empty() => next.own_activity = None,
        Act::Set(a) => next.own_activity = Some(a),
    }
    if let Some(cwd) = input.cwd.as_ref().filter(|c| !c.is_empty()) {
        next.cwd = Some(cwd.clone());
    }
    next.last_event = Some(input.hook_event_name.clone());
    Ok(next)
}

// サブエージェントのイベントは、動いているサブエージェントの一覧と承認待ちだけを変える。
// worktree で隔離されたサブエージェントの cwd を行の名前にしないよう、cwd も親の会話の状態も触らない。
fn subagent_update(
    input: &HookInput,
    agent: &str,
    current: Option<&SessionState>,
    now_ms: u64,
) -> Option<SessionState> {
    if input.hook_event_name == "SubagentStop" {
        let mut next = current.filter(|c| c.agents.contains_key(agent))?.clone();
        next.fill_own();
        next.agents.remove(agent);
        return Some(next);
    }
    // ファイルがまだ無くても作る。サブエージェントが動いている間は行を作業中として出すので、
    // フックを入れる前から動いていた会話でも、ここで行が現れる。行の名前は親の会話の次の
    // イベントが cwd を持ってくるまで出せない。
    let mut next = current
        .cloned()
        .unwrap_or_else(|| SessionState::new(&input.session_id));
    next.fill_own();
    let cwd = next.cwd.clone();
    let run = next.agents.entry(agent.to_owned()).or_insert(AgentRun {
        agent_type: None,
        started_at: now_ms,
        last_seen: now_ms,
        pending: None,
    });
    run.last_seen = now_ms;
    if run.agent_type.is_none() {
        run.agent_type = input.agent_type.clone().filter(|t| !t.is_empty());
    }
    let asks_permission = match input.hook_event_name.as_str() {
        "PermissionRequest" => true,
        "PreToolUse" => input.tool_name.as_deref() == Some("request_permissions"),
        _ => false,
    };
    match input.hook_event_name.as_str() {
        _ if asks_permission => {
            let name = input.tool_name.as_deref().unwrap_or("");
            run.pending = Some(activity::tool_activity(
                name,
                input.tool_input.as_ref(),
                cwd.as_deref(),
            ));
        }
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => run.pending = None,
        _ => {}
    }
    Some(next)
}

/// `AGENT_STALE_MS` のあいだイベントのないサブエージェントを外す。表示が変わるなら新しい状態を返す。
/// updated_at は変えない。セッションを消すまでの 24 時間を、最後のフックから数え続けるためである。
pub fn expire_agents(state: &SessionState, now_ms: u64) -> Option<SessionState> {
    let last_stale = state
        .agents
        .values()
        .filter(|a| is_stale(a, now_ms))
        .map(|a| a.last_seen)
        .max()?;
    let mut next = state.clone();
    next.fill_own();
    // 表示が変わる時刻は、外したサブエージェントが最後に動いた時刻にする。外した時刻にすると、
    // 完了までにかかった時間に AGENT_STALE_MS と見回りの間隔ぶんが足されてしまう。
    settle(
        &mut next,
        Some(state.status),
        now_ms,
        last_stale.max(state.status_since),
    );
    Some(next)
}

fn is_stale(agent: &AgentRun, now_ms: u64) -> bool {
    now_ms.saturating_sub(agent.last_seen) >= AGENT_STALE_MS
}

// 期限切れのサブエージェントを外してから、親の会話の状態とサブエージェントを重ねて表示用の値を決める。
// 承認待ちをいちばん優先し、親の会話の承認待ちを先に見せる。親が完了や待機でも、サブエージェントが
// 動いていれば作業中として見せる。
fn settle(next: &mut SessionState, before: Option<Status>, now_ms: u64, changed_at: u64) {
    next.agents.retain(|_, a| !is_stale(a, now_ms));
    let own = next.own();
    let pending = next
        .agents
        .values()
        .filter_map(|a| a.pending.as_ref().map(|p| (a.last_seen, p)))
        .max_by_key(|(seen, _)| *seen)
        .map(|(_, p)| p.clone());
    let (status, activity, reason) = match pending {
        Some(p) if own != Status::Waiting => {
            (Status::Waiting, Some(p), Some("permission".to_owned()))
        }
        _ if matches!(own, Status::Idle | Status::Done) => match agents_activity(&next.agents) {
            Some(a) => (Status::Working, Some(a), None),
            None => (
                own,
                next.own_activity.clone(),
                next.own_status_reason.clone(),
            ),
        },
        _ => (
            own,
            next.own_activity.clone(),
            next.own_status_reason.clone(),
        ),
    };
    if before != Some(status) {
        next.status_since = changed_at;
    }
    next.status = status;
    next.activity = activity;
    next.status_reason = reason;
}

// 親の会話の Agent ツールの一行表示と同じ形で、動いているサブエージェントの種類を並べる。
fn agents_activity(agents: &BTreeMap<String, AgentRun>) -> Option<Activity> {
    if agents.is_empty() {
        return None;
    }
    let mut runs: Vec<&AgentRun> = agents.values().collect();
    runs.sort_by_key(|a| a.started_at);
    let mut types: Vec<&str> = Vec::new();
    for t in runs.iter().filter_map(|a| a.agent_type.as_deref()) {
        if !types.contains(&t) {
            types.push(t);
        }
    }
    let input = (!types.is_empty()).then(|| json!({ "description": types.join(", ") }));
    Some(activity::tool_activity("Agent", input.as_ref(), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{ActivityKind, TEXT_MAX_CHARS};
    use serde_json::json;

    fn input(v: Value) -> HookInput {
        serde_json::from_value(v).unwrap()
    }

    fn status_after(event: Value, current: Option<Status>) -> Option<Status> {
        let cur = current.map(|s| SessionState {
            status: s,
            ..SessionState::new("s1")
        });
        match transition(&input(event), cur.as_ref(), 1) {
            Transition::Write(s) => Some(s.status),
            Transition::Delete | Transition::Nothing => None,
        }
    }

    fn ev(name: &str) -> Value {
        json!({ "session_id": "s1", "hook_event_name": name, "cwd": "/w/proj" })
    }

    fn write(t: Transition) -> SessionState {
        match t {
            Transition::Write(s) => *s,
            other => panic!("expected write, got {other:?}"),
        }
    }

    #[test]
    fn event_to_status_table() {
        use Status::*;
        let cases: Vec<(Value, Option<Status>, Option<Status>)> = vec![
            (ev("SessionStart"), None, Some(Idle)),
            (ev("UserPromptSubmit"), Some(Done), Some(Working)),
            (
                json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}),
                Some(Working),
                Some(Working),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which?"}]}}),
                Some(Working),
                Some(Waiting),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"ExitPlanMode","tool_input":{}}),
                Some(Working),
                Some(Waiting),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm x"}}),
                Some(Working),
                Some(Waiting),
            ),
            (ev("PostToolUse"), Some(Waiting), Some(Working)),
            (ev("PostToolUseFailure"), Some(Working), Some(Working)),
            (ev("PermissionDenied"), Some(Working), Some(Working)),
            (ev("Elicitation"), Some(Working), Some(Waiting)),
            (ev("ElicitationResult"), Some(Waiting), Some(Working)),
            (
                json!({"session_id":"s1","hook_event_name":"Notification","notification_type":"permission_prompt"}),
                Some(Working),
                Some(Waiting),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"Notification","notification_type":"idle_prompt"}),
                Some(Working),
                Some(Idle),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"Notification","notification_type":"idle_prompt"}),
                Some(Done),
                None,
            ),
            (
                json!({"session_id":"s1","hook_event_name":"Notification","notification_type":"auth_success"}),
                Some(Working),
                None,
            ),
            (
                json!({"session_id":"s1","hook_event_name":"Stop","last_assistant_message":"Finished."}),
                Some(Working),
                Some(Done),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"StopFailure","error":"rate_limit"}),
                Some(Working),
                Some(Error),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"SessionStart","source":"compact"}),
                Some(Working),
                Some(Working),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"MessageDisplay","delta":"a\nb\n","final":false}),
                Some(Working),
                Some(Working),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"MessageDisplay","delta":"","final":true}),
                Some(Working),
                None,
            ),
            (
                json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"request_user_input","tool_input":{"questions":[]}}),
                Some(Working),
                Some(Waiting),
            ),
            (
                json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"request_permissions","tool_input":{"permissions":{}}}),
                Some(Working),
                Some(Waiting),
            ),
            (ev("Interrupt"), Some(Working), Some(Idle)),
            (ev("Interrupt"), Some(Waiting), Some(Idle)),
            (ev("SubagentStop"), Some(Working), None),
            (ev("SomeFutureEvent"), Some(Working), None),
        ];
        for (event, current, expected) in cases {
            assert_eq!(
                status_after(event.clone(), current),
                expected,
                "event {event} from {current:?}"
            );
        }
    }

    #[test]
    fn started_at_is_set_once_and_kept() {
        let s = write(transition(&input(ev("SessionStart")), None, 10));
        assert_eq!(s.started_at, 10);
        let s = write(transition(&input(ev("UserPromptSubmit")), Some(&s), 20));
        let perm = json!({"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm"}});
        let s = write(transition(&input(perm), Some(&s), 30));
        let stop =
            json!({"session_id":"s1","hook_event_name":"Stop","last_assistant_message":"ok"});
        let s = write(transition(&input(stop), Some(&s), 40));
        assert_eq!(
            (s.status, s.started_at, s.updated_at),
            (Status::Done, 10, 40)
        );
        assert_eq!(
            transition(&input(ev("SessionEnd")), Some(&s), 50),
            Transition::Delete
        );

        // started_at を持たない古いファイルは、次のイベントの時刻で埋める。
        let legacy: SessionState = serde_json::from_value(
            json!({"session_id": "s1", "status": "working", "updated_at": 5}),
        )
        .unwrap();
        assert_eq!(legacy.started_at, 0);
        let s = write(transition(&input(ev("PostToolUse")), Some(&legacy), 60));
        assert_eq!(s.started_at, 60);
    }

    #[test]
    fn activity_follows_events() {
        let pre = input(json!({
            "session_id": "s1", "hook_event_name": "PreToolUse", "cwd": "/w/proj",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test --workspace", "description": "Run tests"}
        }));
        let s = write(transition(&pre, None, 5));
        let a = s.activity.clone().unwrap();
        assert_eq!(
            (a.kind, a.summary.as_str(), a.detail.as_deref()),
            (
                ActivityKind::Tool,
                "Run tests",
                Some("cargo test --workspace")
            )
        );
        assert_eq!(s.cwd.as_deref(), Some("/w/proj"));

        let s = write(transition(&input(ev("PostToolUse")), Some(&s), 6));
        assert_eq!(s.activity.as_ref().unwrap().summary, "Run tests");

        // cwd を持たないイベントでも、保存済みの cwd からの相対パスで要約を作る。
        let edit = input(json!({
            "session_id": "s1", "hook_event_name": "PreToolUse",
            "tool_name": "Edit", "tool_input": {"file_path": "/w/proj/src/a.rs"}
        }));
        let s = write(transition(&edit, Some(&s), 7));
        assert_eq!(s.activity.as_ref().unwrap().summary, "編集: src/a.rs");

        let stop = input(json!({
            "session_id": "s1", "hook_event_name": "Stop",
            "last_assistant_message": "リファクタリングが終わりました。\n\n## 変更点\n- a"
        }));
        let s = write(transition(&stop, Some(&s), 8));
        let a = s.activity.clone().unwrap();
        assert_eq!(a.kind, ActivityKind::Message);
        assert_eq!(a.summary, "リファクタリングが終わりました。 ## 変更点 - a");

        let s = write(transition(&input(ev("UserPromptSubmit")), Some(&s), 9));
        assert_eq!(s.activity, None);
    }

    #[test]
    fn status_since_changes_only_with_status() {
        let s = write(transition(&input(ev("UserPromptSubmit")), None, 10));
        assert_eq!(s.status_since, 10);
        let pre = json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}});
        let s = write(transition(&input(pre), Some(&s), 20));
        assert_eq!(s.status_since, 10);
        let perm = json!({"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm"}});
        let s = write(transition(&input(perm.clone()), Some(&s), 30));
        assert_eq!((s.status, s.status_since), (Status::Waiting, 30));
        let s = write(transition(&input(perm), Some(&s), 40));
        assert_eq!(s.status_since, 30);
    }

    #[test]
    fn reasons_and_turn_start_follow_events() {
        let s = write(transition(&input(ev("UserPromptSubmit")), None, 100));
        assert_eq!(
            (s.turn_started_at, s.status_reason.as_deref()),
            (Some(100), None)
        );
        let perm = json!({"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm"}});
        let s = write(transition(&input(perm), Some(&s), 200));
        assert_eq!(s.status_reason.as_deref(), Some("permission"));
        let note = json!({"session_id":"s1","hook_event_name":"Notification","notification_type":"permission_prompt"});
        let s = write(transition(&input(note), Some(&s), 250));
        assert_eq!(s.status_reason.as_deref(), Some("permission"));
        let s = write(transition(&input(ev("PostToolUse")), Some(&s), 300));
        assert_eq!(s.status_reason, None);
        for tool in ["AskUserQuestion", "request_user_input"] {
            let ask = json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":tool,"tool_input":{}});
            assert_eq!(
                write(transition(&input(ask), Some(&s), 310))
                    .status_reason
                    .as_deref(),
                Some("question"),
                "{tool}"
            );
        }
        let plan = json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"ExitPlanMode","tool_input":{}});
        assert_eq!(
            write(transition(&input(plan), Some(&s), 320))
                .status_reason
                .as_deref(),
            Some("plan")
        );
        let perm = json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"request_permissions","tool_input":{}});
        assert_eq!(
            write(transition(&input(perm), Some(&s), 330))
                .status_reason
                .as_deref(),
            Some("permission")
        );
        let stop =
            json!({"session_id":"s1","hook_event_name":"Stop","last_assistant_message":"ok"});
        let s = write(transition(&input(stop), Some(&s), 900));
        assert_eq!((s.turn_started_at, s.status_since), (Some(100), 900));
        let fail = json!({"session_id":"s1","hook_event_name":"StopFailure","error":"rate_limit"});
        assert_eq!(
            write(transition(&input(fail), Some(&s), 950))
                .status_reason
                .as_deref(),
            Some("rate_limit")
        );
        let start = json!({"session_id":"s1","hook_event_name":"SessionStart","source":"compact"});
        assert_eq!(
            write(transition(&input(start), Some(&s), 960)).turn_started_at,
            Some(100)
        );
        let start = json!({"session_id":"s1","hook_event_name":"SessionStart","source":"startup"});
        assert_eq!(
            write(transition(&input(start), Some(&s), 970)).turn_started_at,
            None
        );
    }

    #[test]
    fn message_display_shows_last_non_empty_line() {
        let cur = SessionState {
            status: Status::Waiting,
            ..SessionState::new("s1")
        };
        let md = input(json!({
            "session_id": "s1", "hook_event_name": "MessageDisplay",
            "turn_id": "t", "message_id": "m", "index": 0, "final": false,
            "delta": "Here is the plan:\n\n1. 手順を   確認する\n\n"
        }));
        let s = write(transition(&md, Some(&cur), 2));
        assert_eq!(s.status, Status::Waiting);
        assert_eq!(s.activity.unwrap().summary, "1. 手順を 確認する");
        let long = input(json!({
            "session_id": "s1", "hook_event_name": "MessageDisplay", "delta": "x".repeat(900)
        }));
        let s = write(transition(&long, Some(&cur), 3));
        assert_eq!(s.activity.unwrap().summary.chars().count(), TEXT_MAX_CHARS);
    }

    #[test]
    fn older_state_files_are_read_and_carried_forward() {
        let legacy = json!({
            "session_id": "s1", "cwd": "/w/p", "status": "working",
            "line": "Bash: cargo test", "last_event": "PreToolUse",
            "updated_at": 5, "context": null
        });
        let s: SessionState = serde_json::from_value(legacy).unwrap();
        let a = s.activity.unwrap();
        assert_eq!(
            (a.kind, a.summary.as_str()),
            (ActivityKind::Text, "Bash: cargo test")
        );
        assert_eq!(
            (s.status_since, s.origin, s.provider),
            (0, None, Provider::Claude)
        );

        for line in [json!(null), json!(""), json!(42)] {
            let v = json!({"session_id": "s1", "status": "done", "line": line});
            let s: SessionState = serde_json::from_value(v).unwrap();
            assert_eq!(s.activity, None);
        }
        let minimal: SessionState = serde_json::from_value(json!({"session_id": "s1"})).unwrap();
        assert_eq!(minimal, SessionState::new("s1"));

        // own_status を持たない古いファイルは、表示用の値を親の会話の状態として引き継ぐ。
        let legacy: SessionState = serde_json::from_value(json!({
            "session_id": "s1", "cwd": "/w/p", "status": "done", "status_since": 7,
            "activity": {"kind": "message", "summary": "前の応答"}, "updated_at": 7
        }))
        .unwrap();
        assert_eq!((legacy.own_status, legacy.agents.len()), (None, 0));
        let s = write(transition(&sub("SubagentStart", "a1"), Some(&legacy), 20));
        assert_eq!(
            (
                s.status,
                s.own_status,
                s.own_activity.as_ref().unwrap().summary.as_str()
            ),
            (Status::Working, Some(Status::Done), "前の応答")
        );
        let s = write(transition(&sub("SubagentStop", "a1"), Some(&s), 30));
        assert_eq!((s.status, s.status_since), (Status::Done, 30));
        assert_eq!(s.activity.unwrap().summary, "前の応答");

        // 親の会話のイベントでも、古いファイルの状態を引き継いで判断する。
        let idle = json!({"session_id":"s1","hook_event_name":"Notification","notification_type":"idle_prompt"});
        let working: SessionState =
            serde_json::from_value(json!({"session_id": "s1", "status": "working"})).unwrap();
        assert_eq!(
            write(transition(&input(idle), Some(&working), 5)).status,
            Status::Idle
        );
    }

    #[test]
    fn state_files_round_trip() {
        let mut s = write(transition(
            &input(
                json!({"session_id":"s1","hook_event_name":"PreToolUse","cwd":"/w","tool_name":"Grep","tool_input":{"pattern":"x"}}),
            ),
            None,
            1,
        ));
        s.origin = Some(Origin {
            bundle_id: Some("com.apple.Terminal".into()),
            term_program: Some("Apple_Terminal".into()),
            tty: Some("/dev/ttys002".into()),
            ..Origin::default()
        });
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"activity\""));
        assert!(!text.contains("\"line\""));
        assert!(!text.contains("window") && !text.contains("ancestors"));
        assert!(!text.contains("\"agents\""));
        assert_eq!(serde_json::from_str::<SessionState>(&text).unwrap(), s);

        let origin = windows_terminal();
        let text = serde_json::to_string(&origin).unwrap();
        assert!(text.contains("\"hwnd\"") && text.contains("\"ancestors\""));
        assert_eq!(serde_json::from_str::<Origin>(&text).unwrap(), origin);
        let older: Origin = serde_json::from_value(
            json!({"term_program": "Apple_Terminal", "tty": "/dev/ttys002"}),
        )
        .unwrap();
        assert_eq!((older.window, older.ancestors), (None, Vec::new()));

        let s = write(transition(
            &sub("PermissionRequest", "a1"),
            Some(&working_parent()),
            20,
        ));
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"agents\"") && text.contains("\"own_status\""));
        assert_eq!(serde_json::from_str::<SessionState>(&text).unwrap(), s);
        let plain = serde_json::to_string(&working_parent()).unwrap();
        assert!(!plain.contains("\"agents\""));
    }

    #[test]
    fn origin_merge_cases() {
        let terminal = Origin {
            bundle_id: Some("com.apple.Terminal".into()),
            term_program: Some("Apple_Terminal".into()),
            tty: Some("/dev/ttys002".into()),
            ..Origin::default()
        };
        let merged = Origin::merge(None, Some(&terminal), Some("cli")).unwrap();
        assert_eq!(merged.entrypoint.as_deref(), Some("cli"));
        let desktop = Origin {
            bundle_id: Some("com.anthropic.claudefordesktop".into()),
            ..Origin::default()
        };
        let merged = Origin::merge(Some(&merged), Some(&desktop), None).unwrap();
        assert_eq!(
            merged.bundle_id.as_deref(),
            Some("com.anthropic.claudefordesktop")
        );
        assert_eq!(
            (merged.tty, merged.entrypoint.as_deref()),
            (None, Some("cli"))
        );
        assert_eq!(Origin::merge(None, Some(&Origin::default()), None), None);
        let kept = Origin::merge(Some(&terminal), Some(&Origin::default()), None).unwrap();
        assert_eq!(kept, terminal);

        let first = windows_terminal();
        let merged = Origin::merge(None, Some(&first), Some("cli")).unwrap();
        // SessionStart と UserPromptSubmit 以外のイベントでは環境変数しか取らないので、
        // それで Windows の手がかりが消えてはいけない。
        let env_only = Origin {
            term_program: Some("vscode".into()),
            ..Origin::default()
        };
        let kept = Origin::merge(Some(&merged), Some(&env_only), None).unwrap();
        assert_eq!(kept.term_program.as_deref(), Some("vscode"));
        assert_eq!(
            (&kept.window, &kept.ancestors, kept.entrypoint.as_deref()),
            (&first.window, &first.ancestors, Some("cli"))
        );
        assert_eq!(
            Origin::merge(Some(&kept), Some(&Origin::default()), None).unwrap(),
            kept
        );

        // 新しい Windows の検出は、環境変数も含めて全体を置き換える。
        let conhost = Origin {
            window: Some(WindowRef {
                hwnd: 0x2_0b3d,
                pid: 7300,
                created: 133_800_000_000_000_000,
            }),
            ..Origin::default()
        };
        let replaced = Origin::merge(Some(&kept), Some(&conhost), None).unwrap();
        assert_eq!(replaced.window, conhost.window);
        assert_eq!(replaced.ancestors, Vec::new());
        assert_eq!(
            (replaced.term_program, replaced.entrypoint.as_deref()),
            (None, Some("cli"))
        );
    }

    fn windows_terminal() -> Origin {
        Origin {
            window: Some(WindowRef {
                hwnd: 0x1_0a2c,
                pid: 4120,
                created: 133_700_000_000_000_000,
            }),
            ancestors: vec![ProcessRef {
                pid: 5008,
                created: 133_700_000_100_000_000,
                exe: r"C:\Users\user\.local\bin\claude.exe".into(),
            }],
            ..Origin::default()
        }
    }

    #[test]
    fn transcript_context_respects_statusline() {
        let fresh = ContextUsage {
            used_percentage: Some(40.0),
            total_input_tokens: Some(80_000),
            context_window_size: Some(200_000),
            updated_at: 1_000_000,
            source: Some("statusline".into()),
        };
        assert_eq!(
            context_from_transcript(Some(&fresh), 90_000, 1_000_000 + 60_000),
            None
        );
        let later = context_from_transcript(Some(&fresh), 90_000, 1_000_000 + 10 * 60_000).unwrap();
        assert_eq!(later.used_percentage, Some(45.0));
        assert_eq!(later.context_window_size, Some(200_000));
        assert_eq!(later.source.as_deref(), Some("transcript"));
        // 上限が分からなければトークン数だけにする。
        let bare = context_from_transcript(None, 142_000, 5).unwrap();
        assert_eq!(
            (bare.used_percentage, bare.total_input_tokens),
            (None, Some(142_000))
        );
        let again =
            context_from_transcript(Some(&later), 100_000, 1_000_000 + 11 * 60_000).unwrap();
        assert_eq!(again.used_percentage, Some(50.0));
    }

    const WORKTREE: &str = "/w/proj/.claude/worktrees/agent-a1";

    fn sub(name: &str, agent: &str) -> HookInput {
        input(json!({
            "session_id": "s1", "hook_event_name": name, "agent_id": agent,
            "agent_type": "Explore", "cwd": WORKTREE,
            "tool_name": "Bash", "tool_input": {"command": "rm x"}
        }))
    }

    fn working_parent() -> SessionState {
        let s = write(transition(&input(ev("UserPromptSubmit")), None, 5));
        let pre = json!({
            "session_id": "s1", "hook_event_name": "PreToolUse", "cwd": "/w/proj",
            "tool_name": "Agent", "tool_input": {"description": "調べる", "run_in_background": true}
        });
        write(transition(&input(pre), Some(&s), 10))
    }

    fn stop() -> HookInput {
        input(
            json!({"session_id":"s1","hook_event_name":"Stop","last_assistant_message":"終わりました。"}),
        )
    }

    #[test]
    fn background_agent_keeps_a_stopped_parent_working() {
        let s = write(transition(
            &sub("SubagentStart", "a1"),
            Some(&working_parent()),
            20,
        ));
        let s = write(transition(&stop(), Some(&s), 30));
        assert_eq!(
            (s.status, s.own_status, s.status_since),
            (Status::Working, Some(Status::Done), 5)
        );
        assert_eq!(
            s.activity.as_ref().unwrap().summary,
            "サブエージェント: Explore"
        );
        assert_eq!(s.own_activity.as_ref().unwrap().summary, "終わりました。");

        let s = write(transition(&sub("PostToolUse", "a1"), Some(&s), 40));
        assert_eq!((s.status, s.status_since), (Status::Working, 5));
        let s = write(transition(&sub("SubagentStop", "a1"), Some(&s), 50));
        assert_eq!((s.status, s.status_since), (Status::Done, 50));
        assert_eq!(s.activity.unwrap().summary, "終わりました。");
        assert!(s.agents.is_empty());
        assert_eq!(s.turn_started_at, Some(5));
    }

    #[test]
    fn subagent_events_leave_parent_state_and_cwd() {
        let parent = working_parent();
        let mut s = parent.clone();
        for (i, name) in [
            "SubagentStart",
            "PreToolUse",
            "PostToolUse",
            "MessageDisplay",
        ]
        .into_iter()
        .enumerate()
        {
            s = write(transition(&sub(name, "a1"), Some(&s), 20 + i as u64));
            assert_eq!(
                (s.own_status, &s.own_activity, s.cwd.as_deref()),
                (Some(Status::Working), &parent.activity, Some("/w/proj")),
                "{name}"
            );
            assert_eq!((s.status, &s.activity), (Status::Working, &parent.activity));
        }
        assert_eq!(s.agents["a1"].last_seen, 23);
        assert_eq!(s.agents["a1"].started_at, 20);

        // 承認待ちと終了のイベントも、worktree の cwd で行の名前を変えない。
        for name in ["PermissionRequest", "PostToolUse", "SubagentStop"] {
            s = write(transition(&sub(name, "a1"), Some(&s), 30));
            assert_eq!(s.cwd.as_deref(), Some("/w/proj"), "{name}");
        }
    }

    #[test]
    fn subagent_approval_waits_until_that_agent_answers() {
        let s = write(transition(
            &sub("SubagentStart", "a1"),
            Some(&working_parent()),
            20,
        ));
        let done = write(transition(&stop(), Some(&s), 30));
        let s = write(transition(&sub("PermissionRequest", "a1"), Some(&done), 40));
        assert_eq!(
            (s.status, s.status_since, s.status_reason.as_deref()),
            (Status::Waiting, 40, Some("permission"))
        );
        assert_eq!(s.activity.as_ref().unwrap().summary, "rm x");
        assert_eq!(s.own_status, Some(Status::Done));

        // 別のサブエージェントのツールの結果では、a1 の承認待ちは消えない。
        let other = write(transition(&sub("PostToolUse", "a2"), Some(&s), 45));
        assert_eq!(other.status, Status::Waiting);
        assert!(other.agents["a1"].pending.is_some());

        for end in ["PostToolUse", "PostToolUseFailure", "PermissionDenied"] {
            let after = write(transition(&sub(end, "a1"), Some(&s), 50));
            assert_eq!(
                (after.status, after.status_since, after.status_reason),
                (Status::Working, 50, None),
                "{end}"
            );
        }
        // Codex のサブエージェントが request_permissions で承認を求めた場合も同じく待つ。
        let mut ask = sub("PreToolUse", "a3");
        ask.tool_name = Some("request_permissions".to_owned());
        let asked = write(transition(&ask, Some(&done), 55));
        assert_eq!(
            (asked.status, asked.status_reason.as_deref()),
            (Status::Waiting, Some("permission"))
        );
        let s = write(transition(&sub("PostToolUse", "a1"), Some(&s), 50));
        let s = write(transition(&sub("SubagentStop", "a1"), Some(&s), 60));
        assert_eq!((s.status, s.status_since), (Status::Done, 60));
        assert_eq!(s.activity.unwrap().summary, "終わりました。");
    }

    #[test]
    fn main_thread_waiting_takes_precedence() {
        let ask = input(json!({
            "session_id": "s1", "hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion",
            "tool_input": {"questions": [{"question": "どちら？"}]}
        }));
        let s = write(transition(
            &sub("PermissionRequest", "a1"),
            Some(&working_parent()),
            20,
        ));
        let s = write(transition(&ask, Some(&s), 30));
        assert_eq!(
            (s.status, s.status_since, s.status_reason.as_deref()),
            (Status::Waiting, 20, Some("question"))
        );
        assert_eq!(s.activity.as_ref().unwrap().summary, "質問: どちら？");

        // 親の会話の回答が済めば、まだ残っているサブエージェントの承認待ちを見せる。
        let s = write(transition(&input(ev("PostToolUse")), Some(&s), 40));
        assert_eq!(
            (s.status, s.status_reason.as_deref(), s.own_status),
            (Status::Waiting, Some("permission"), Some(Status::Working))
        );
        assert_eq!(s.activity.unwrap().summary, "rm x");
    }

    #[test]
    fn permission_notification_for_a_subagent_does_not_stick() {
        let s = write(transition(
            &sub("SubagentStart", "a1"),
            Some(&working_parent()),
            20,
        ));
        let done = write(transition(&stop(), Some(&s), 30));
        let s = write(transition(&sub("PermissionRequest", "a1"), Some(&done), 40));
        let note = input(
            json!({"session_id":"s1","hook_event_name":"Notification","notification_type":"permission_prompt"}),
        );
        assert_eq!(transition(&note, Some(&s), 45), Transition::Nothing);
        let s = write(transition(&sub("PostToolUse", "a1"), Some(&s), 50));
        let s = write(transition(&sub("SubagentStop", "a1"), Some(&s), 60));
        assert_eq!(s.status, Status::Done);

        // サブエージェントが承認を待っていなければ、今までどおり親の会話の承認待ちにする。
        let s = write(transition(&note, Some(&done), 70));
        assert_eq!(s.own_status, Some(Status::Waiting));
    }

    #[test]
    fn stale_agents_expire() {
        let s = write(transition(
            &sub("SubagentStart", "a1"),
            Some(&working_parent()),
            20,
        ));
        let s = write(transition(&sub("SubagentStart", "a2"), Some(&s), 1_000));
        let done = write(transition(&stop(), Some(&s), 2_000));
        assert_eq!(done.agents.len(), 2);

        // a1 は 30 分イベントがないので、次に書くときに外す。
        let t = 20 + AGENT_STALE_MS;
        let s = write(transition(&sub("PostToolUse", "a2"), Some(&done), t));
        assert_eq!(s.agents.keys().collect::<Vec<_>>(), ["a2"]);
        assert_eq!(s.status, Status::Working);

        // 見回りからも外す。表示が変わる時刻は最後に動いた時刻にし、updated_at は変えない。
        assert_eq!(expire_agents(&s, t + 1), None);
        let later = t + AGENT_STALE_MS + 60_000;
        let e = expire_agents(&s, later).unwrap();
        assert!(e.agents.is_empty());
        assert_eq!(
            (e.status, e.status_since, e.updated_at),
            (Status::Done, t, s.updated_at)
        );
        assert_eq!(e.activity.unwrap().summary, "終わりました。");
    }

    #[test]
    fn subagent_events_without_a_session_file() {
        assert_eq!(
            transition(&sub("SubagentStop", "a1"), None, 10),
            Transition::Nothing
        );
        assert_eq!(
            transition(&sub("SubagentStop", "a1"), Some(&working_parent()), 10),
            Transition::Nothing
        );
        let s = write(transition(&sub("SubagentStart", "a1"), None, 10));
        assert_eq!(
            (s.status, s.own_status, s.cwd.as_deref(), s.started_at),
            (Status::Working, Some(Status::Idle), None, 10)
        );
        let s = write(transition(&sub("SubagentStop", "a1"), Some(&s), 20));
        assert_eq!(s.status, Status::Idle);
    }

    #[test]
    fn session_start_clears_agents_but_stop_does_not() {
        let s = write(transition(
            &sub("SubagentStart", "a1"),
            Some(&working_parent()),
            20,
        ));
        let s = write(transition(&stop(), Some(&s), 30));
        assert_eq!(s.agents.len(), 1);
        let compact =
            input(json!({"session_id":"s1","hook_event_name":"SessionStart","source":"compact"}));
        assert_eq!(write(transition(&compact, Some(&s), 40)).agents.len(), 1);
        let resume =
            input(json!({"session_id":"s1","hook_event_name":"SessionStart","source":"resume"}));
        let s = write(transition(&resume, Some(&s), 50));
        assert_eq!((s.status, s.agents.len()), (Status::Idle, 0));
    }

    #[test]
    fn only_agent_id_marks_a_subagent() {
        // `--agent` で起動したセッションでは、親の会話のフックにも agent_type が入る。
        let pre = json!({
            "session_id": "s1", "hook_event_name": "PreToolUse", "agent_type": "reviewer",
            "cwd": "/w/other", "tool_name": "Bash", "tool_input": {"command": "ls"}
        });
        let mut plain = pre.clone();
        plain.as_object_mut().unwrap().remove("agent_type");
        let parent = working_parent();
        let typed = write(transition(&input(pre), Some(&parent), 20));
        assert_eq!(typed, write(transition(&input(plain), Some(&parent), 20)));
        assert_eq!(typed.cwd.as_deref(), Some("/w/other"));
        assert!(typed.agents.is_empty());
        let empty_id = json!({"session_id":"s1","hook_event_name":"Stop","agent_id":""});
        assert_eq!(
            write(transition(&input(empty_id), Some(&parent), 30)).own_status,
            Some(Status::Done)
        );

        // サブエージェントのツールの実行は、親のコンテキストの使用量を数え直さない。
        let main = input(
            json!({"session_id":"s1","hook_event_name":"PostToolUse","transcript_path":"/t.jsonl"}),
        );
        assert!(main.wants_transcript_usage());
        let sub = input(
            json!({"session_id":"s1","hook_event_name":"PostToolUse","transcript_path":"/t.jsonl","agent_id":"a1"}),
        );
        assert!(!sub.wants_transcript_usage());
        let typed = input(
            json!({"session_id":"s1","hook_event_name":"PostToolUse","transcript_path":"/t.jsonl","agent_type":"reviewer"}),
        );
        assert!(typed.wants_transcript_usage());
    }

    #[test]
    fn repo_is_looked_up_only_when_needed() {
        let lookup_count = std::cell::Cell::new(0);
        let lookup = |cwd: &str| {
            lookup_count.set(lookup_count.get() + 1);
            Some(format!("repo-of-{cwd}"))
        };
        let ev =
            input(json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","cwd":"/w/a"}));
        let mut next = write(transition(&ev, None, 1));
        refresh_repo(&ev, None, &mut next, lookup);
        assert_eq!(next.repo.as_deref(), Some("repo-of-/w/a"));

        let cur = next.clone();
        let mut again = write(transition(&ev, Some(&cur), 2));
        refresh_repo(&ev, Some(&cur), &mut again, lookup);
        assert_eq!(lookup_count.get(), 1);

        let moved = input(
            json!({"session_id":"s1","hook_event_name":"PreToolUse","cwd":"/w/b","tool_name":"Bash"}),
        );
        let mut next = write(transition(&moved, Some(&cur), 3));
        refresh_repo(&moved, Some(&cur), &mut next, |_| None);
        assert_eq!(next.repo, None);

        let sub = input(
            json!({"session_id":"s1","hook_event_name":"PreToolUse","agent_id":"a1","cwd":"/w/b"}),
        );
        let mut next = write(transition(&sub, Some(&cur), 4));
        refresh_repo(&sub, Some(&cur), &mut next, lookup);
        assert_eq!(next.repo.as_deref(), Some("repo-of-/w/a"));
        assert_eq!(lookup_count.get(), 1);
    }

    #[test]
    fn provider_follows_the_input() {
        let mut codex = input(ev("UserPromptSubmit"));
        codex.provider = Provider::Codex;
        let s = write(transition(&codex, None, 1));
        assert_eq!(s.provider, Provider::Codex);
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"provider\":\"codex\""));
        assert_eq!(serde_json::from_str::<SessionState>(&text).unwrap(), s);

        let mut sub_event = sub("SubagentStart", "a1");
        sub_event.provider = Provider::Codex;
        assert_eq!(
            write(transition(&sub_event, None, 2)).provider,
            Provider::Codex
        );
        assert_eq!(
            write(transition(&input(ev("UserPromptSubmit")), None, 3)).provider,
            Provider::Claude
        );
    }

    #[test]
    fn logs_are_read_only_on_their_events() {
        // (provider, event, agent_id, rollout か会話ログを読むか, Codex の題名を読むか)
        let cases = [
            (Provider::Claude, "PostToolUse", None, true, false),
            (Provider::Claude, "SessionStart", None, false, false),
            (Provider::Claude, "UserPromptSubmit", None, false, false),
            (Provider::Codex, "PostToolUse", None, true, false),
            (Provider::Codex, "Stop", None, true, true),
            (Provider::Codex, "SessionStart", None, true, true),
            (Provider::Codex, "UserPromptSubmit", None, false, true),
            (Provider::Codex, "PreToolUse", None, false, false),
            (
                Provider::Codex,
                "UserPromptSubmit",
                Some("a1"),
                false,
                false,
            ),
            (Provider::Codex, "PostToolUse", Some("a1"), false, false),
        ];
        for (provider, event, agent, usage, title) in cases {
            let mut i = input(json!({
                "session_id": "s1", "hook_event_name": event,
                "transcript_path": "/t.jsonl", "agent_id": agent
            }));
            i.provider = provider;
            assert_eq!(
                (i.wants_transcript_usage(), i.wants_codex_title()),
                (usage, title),
                "{provider:?} {event} {agent:?}"
            );
        }
    }

    #[test]
    fn codex_context_matches_the_tui_indicator() {
        // (last total_tokens, model_context_window, 使用中の %)
        let cases = [
            (12_000, Some(272_000), Some(0.0)),
            (5_000, Some(272_000), Some(0.0)),
            (142_000, Some(272_000), Some(50.0)),
            (70_000, Some(272_000), Some(22.0)),
            (400_000, Some(272_000), Some(100.0)),
            (100, Some(12_000), Some(100.0)),
            (70_000, None, None),
        ];
        for (tokens, window, expected) in cases {
            let usage = CodexTokenUsage {
                last_total_tokens: tokens,
                model_context_window: window,
            };
            let c = context_from_codex(&usage, 7);
            assert_eq!(c.used_percentage, expected, "{tokens} / {window:?}");
            assert_eq!(c.total_input_tokens, Some(tokens as u64));
            assert_eq!(c.context_window_size, window.map(|w| w as u64));
            assert_eq!(
                (c.updated_at, c.source.as_deref()),
                (7, Some("codex-rollout"))
            );
        }
    }

    #[test]
    fn aggregate_follows_priority() {
        use Status::*;
        assert_eq!(aggregate([]), Idle);
        assert_eq!(aggregate([Idle, Working]), Working);
        assert_eq!(aggregate([Working, Done]), Working);
        assert_eq!(aggregate([Idle, Done]), Done);
        assert_eq!(aggregate([Done, Error, Working]), Error);
        assert_eq!(aggregate([Error, Waiting, Done]), Waiting);
        assert_eq!(aggregate([Idle, Idle]), Idle);
    }
}
