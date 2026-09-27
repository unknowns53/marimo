use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// 並び順は要件定義が決めている（承認待ち、エラー、完了、作業中、待機の順に高い）。
    pub fn priority(self) -> u8 {
        match self {
            Status::Idle => 0,
            Status::Working => 1,
            Status::Done => 2,
            Status::Error => 3,
            Status::Waiting => 4,
        }
    }
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
}

impl Origin {
    pub fn is_empty(&self) -> bool {
        self.bundle_id.is_none()
            && self.term_program.is_none()
            && self.tty.is_none()
            && self.entrypoint.is_none()
    }

    fn has_process_clues(&self) -> bool {
        self.bundle_id.is_some() || self.term_program.is_some() || self.tty.is_some()
    }

    // CLI で --resume すると、デスクトップアプリの会話と同じ session_id のまま別のアプリへ移る。
    // 環境変数と端末の手がかりは毎回の実行から取り直し、得られたときだけ置き換える。
    // 会話ログの entrypoint は読める契機が限られるので、新しい値がなければ前の値を残す。
    pub fn merge(
        existing: Option<&Origin>,
        process: Option<&Origin>,
        entrypoint: Option<&str>,
    ) -> Option<Origin> {
        let mut next = match process.filter(|p| p.has_process_clues()) {
            Some(p) => Origin {
                entrypoint: existing.and_then(|e| e.entrypoint.clone()),
                ..p.clone()
            },
            None => existing.cloned().unwrap_or_default(),
        };
        if let Some(e) = entrypoint {
            next.entrypoint = Some(e.to_owned());
        }
        (!next.is_empty()).then_some(next)
    }
}

// statusLine は応答ごとに走るので、この時間より新しい値があれば会話ログからの計算より優先する。
const STATUSLINE_FRESH_MS: u64 = 5 * 60 * 1000;

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
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub status: Status,
    /// 今の status になった時刻。吹き出しが「同じ状態が続いている間」を見分けるのに使う。
    #[serde(default)]
    pub status_since: u64,
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
}

impl SessionState {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            cwd: None,
            status: Status::Idle,
            status_since: 0,
            activity: None,
            last_event: None,
            updated_at: 0,
            context: None,
            origin: None,
        }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub aggregate: Status,
    /// 優先度の高い順に並べ、同じ優先度では更新の新しい順に並べる。
    pub sessions: Vec<SessionState>,
    pub rate_limits: Option<RateLimits>,
}

/// 項目名は https://code.claude.com/docs/en/hooks の Common input fields と、
/// 各イベントの input の節に従う。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookInput {
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
}

impl HookInput {
    // ツールの実行ごとに API の呼び出しが一回挟まるので、PostToolUse と Stop で数え直せば
    // 使用量の変化に追いつける。
    pub fn wants_transcript_usage(&self) -> bool {
        matches!(self.hook_event_name.as_str(), "PostToolUse" | "Stop")
            && self
                .transcript_path
                .as_deref()
                .is_some_and(|p| !p.is_empty())
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
    let cur_status = current.map(|s| s.status);
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
    let (status, act) = match input.hook_event_name.as_str() {
        "SessionEnd" => return Transition::Delete,
        // compaction が作業の途中で起きても状態を戻さないよう、compact だけは現状を保つ。
        "SessionStart" if input.source.as_deref() == Some("compact") => {
            (cur_status.unwrap_or_default(), Act::Keep)
        }
        "SessionStart" => (Status::Idle, Act::Clear),
        "UserPromptSubmit" => (Status::Working, Act::Clear),
        "PreToolUse" => {
            let (name, a) = tool();
            // この二つのツールは、呼ばれた時点でユーザーの回答や承認を待つ。
            if matches!(name, "AskUserQuestion" | "ExitPlanMode") {
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
            Some("permission_prompt" | "elicitation_dialog" | "elicitation_url_dialog") => {
                (Status::Waiting, Act::Keep)
            }
            Some("quota_auto_resume_fired") => (Status::Working, Act::Keep),
            // idle_prompt は応答を終えて 60 秒ほど経ったときに届く。ユーザーの割り込みで
            // Stop が発火せず作業中のまま残ったセッションを、ここで待機へ戻す。
            Some("idle_prompt") if cur_status == Some(Status::Working) => (Status::Idle, Act::Keep),
            _ => return Transition::Nothing,
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
                None => return Transition::Nothing,
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
        "StopFailure" => {
            let text = input
                .last_assistant_message
                .as_deref()
                .or(input.error.as_deref())
                .unwrap_or_default();
            (Status::Error, Act::Set(Activity::error(text)))
        }
        _ => return Transition::Nothing,
    };

    let mut next = current
        .cloned()
        .unwrap_or_else(|| SessionState::new(&input.session_id));
    if current.is_none_or(|c| c.status != status) {
        next.status_since = now_ms;
    }
    next.status = status;
    match act {
        Act::Keep => {}
        Act::Clear => next.activity = None,
        Act::Set(a) if a.summary.is_empty() => next.activity = None,
        Act::Set(a) => next.activity = Some(a),
    }
    if let Some(cwd) = input.cwd.as_ref().filter(|c| !c.is_empty()) {
        next.cwd = Some(cwd.clone());
    }
    next.last_event = Some(input.hook_event_name.clone());
    next.updated_at = now_ms;
    Transition::Write(Box::new(next))
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
    fn session_end_deletes() {
        let cur = SessionState::new("s1");
        assert_eq!(
            transition(&input(ev("SessionEnd")), Some(&cur), 1),
            Transition::Delete
        );
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
    fn reads_legacy_state_files() {
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
        assert_eq!((s.status_since, s.origin), (0, None));

        for line in [json!(null), json!(""), json!(42)] {
            let v = json!({"session_id": "s1", "status": "done", "line": line});
            let s: SessionState = serde_json::from_value(v).unwrap();
            assert_eq!(s.activity, None);
        }
        let minimal: SessionState = serde_json::from_value(json!({"session_id": "s1"})).unwrap();
        assert_eq!(minimal, SessionState::new("s1"));
    }

    #[test]
    fn new_format_round_trips() {
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
            entrypoint: None,
        });
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"activity\""));
        assert!(!text.contains("\"line\""));
        assert_eq!(serde_json::from_str::<SessionState>(&text).unwrap(), s);
    }

    #[test]
    fn origin_merge_keeps_entrypoint_and_refreshes_process_clues() {
        let terminal = Origin {
            bundle_id: Some("com.apple.Terminal".into()),
            term_program: Some("Apple_Terminal".into()),
            tty: Some("/dev/ttys002".into()),
            entrypoint: None,
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

    #[test]
    fn aggregate_follows_priority() {
        use Status::*;
        assert_eq!(aggregate([]), Idle);
        assert_eq!(aggregate([Idle, Working]), Working);
        assert_eq!(aggregate([Working, Done]), Done);
        assert_eq!(aggregate([Done, Error, Working]), Error);
        assert_eq!(aggregate([Error, Waiting, Done]), Waiting);
        assert_eq!(aggregate([Idle, Idle]), Idle);
    }
}
