use serde::{Deserialize, Serialize};
use serde_json::Value;

/// バイトでなく文字で数え、日本語でも見た目の長さが揃うようにする。
pub const LINE_MAX_CHARS: usize = 80;

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub last_event: Option<String>,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub context: Option<ContextUsage>,
}

impl SessionState {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            cwd: None,
            status: Status::Idle,
            line: None,
            last_event: None,
            updated_at: 0,
            context: None,
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    Write(SessionState),
    Delete,
    Nothing,
}

enum Line {
    Keep,
    Clear,
    Set(String),
}

pub fn transition(input: &HookInput, current: Option<&SessionState>, now_ms: u64) -> Transition {
    let cur_status = current.map(|s| s.status);
    let (status, line) = match input.hook_event_name.as_str() {
        "SessionEnd" => return Transition::Delete,
        // compaction が作業の途中で起きても状態を戻さないよう、compact だけは現状を保つ。
        "SessionStart" if input.source.as_deref() == Some("compact") => {
            (cur_status.unwrap_or_default(), Line::Keep)
        }
        "SessionStart" => (Status::Idle, Line::Clear),
        "UserPromptSubmit" => (Status::Working, Line::Clear),
        "PreToolUse" => {
            let name = input.tool_name.as_deref().unwrap_or("");
            let summary = tool_summary(name, input.tool_input.as_ref());
            // この二つのツールは、呼ばれた時点でユーザーの回答や承認を待つ。
            if matches!(name, "AskUserQuestion" | "ExitPlanMode") {
                (Status::Waiting, Line::Set(summary))
            } else {
                (Status::Working, Line::Set(summary))
            }
        }
        "PermissionRequest" => {
            let name = input.tool_name.as_deref().unwrap_or("");
            (
                Status::Waiting,
                Line::Set(tool_summary(name, input.tool_input.as_ref())),
            )
        }
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" | "ElicitationResult" => {
            (Status::Working, Line::Keep)
        }
        "Elicitation" => (Status::Waiting, Line::Keep),
        "Notification" => match input.notification_type.as_deref() {
            Some("permission_prompt" | "elicitation_dialog" | "elicitation_url_dialog") => {
                (Status::Waiting, Line::Keep)
            }
            Some("quota_auto_resume_fired") => (Status::Working, Line::Keep),
            // idle_prompt は応答を終えて 60 秒ほど経ったときに届く。ユーザーの割り込みで
            // Stop が発火せず作業中のまま残ったセッションを、ここで待機へ戻す。
            Some("idle_prompt") if cur_status == Some(Status::Working) => {
                (Status::Idle, Line::Keep)
            }
            _ => return Transition::Nothing,
        },
        "Stop" => {
            let line = match input.last_assistant_message.as_deref().map(one_line) {
                Some(text) if !text.is_empty() => Line::Set(text),
                _ => Line::Keep,
            };
            (Status::Done, line)
        }
        "StopFailure" => {
            let text = input
                .last_assistant_message
                .as_deref()
                .or(input.error.as_deref())
                .map(one_line)
                .unwrap_or_default();
            (Status::Error, Line::Set(text))
        }
        _ => return Transition::Nothing,
    };

    let mut next = current
        .cloned()
        .unwrap_or_else(|| SessionState::new(&input.session_id));
    next.status = status;
    match line {
        Line::Keep => {}
        Line::Clear => next.line = None,
        Line::Set(text) if text.is_empty() => next.line = None,
        Line::Set(text) => next.line = Some(text),
    }
    if let Some(cwd) = input.cwd.as_ref().filter(|c| !c.is_empty()) {
        next.cwd = Some(cwd.clone());
    }
    next.last_event = Some(input.hook_event_name.clone());
    next.updated_at = now_ms;
    Transition::Write(next)
}

/// `tool_input` の項目名は hooks のドキュメントの PreToolUse input にある、ツールごとの表に従う。
pub fn tool_summary(tool_name: &str, tool_input: Option<&Value>) -> String {
    let get = |key: &str| {
        tool_input
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    };
    let detail: Option<String> = match tool_name {
        "Bash" | "PowerShell" => get("command").map(str::to_owned),
        // パスは長く、行の幅をすぐ使い切るので、ファイル名だけを出す。
        "Write" | "Edit" | "Read" => get("file_path").map(|p| file_name(p).to_owned()),
        "Glob" | "Grep" => get("pattern").map(str::to_owned),
        "WebFetch" => get("url").map(str::to_owned),
        "WebSearch" => get("query").map(str::to_owned),
        "Agent" => get("description")
            .or(get("subagent_type"))
            .map(str::to_owned),
        "AskUserQuestion" => tool_input
            .and_then(|v| v.pointer("/questions/0/question"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        "ExitPlanMode" => get("planFilePath").map(|p| file_name(p).to_owned()),
        _ => first_string_field(tool_input),
    };
    let label = if tool_name.is_empty() {
        "Tool"
    } else {
        tool_name
    };
    match detail {
        Some(d) => one_line(&format!("{label}: {d}")),
        None => one_line(label),
    }
}

fn first_string_field(tool_input: Option<&Value>) -> Option<String> {
    tool_input?
        .as_object()?
        .values()
        .find_map(|v| v.as_str().filter(|s| !s.trim().is_empty()))
        .map(str::to_owned)
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|seg| !seg.is_empty())
        .unwrap_or(path)
}

pub fn one_line(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= LINE_MAX_CHARS {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(LINE_MAX_CHARS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn lines_follow_events() {
        let pre = input(json!({
            "session_id": "s1", "hook_event_name": "PreToolUse", "cwd": "/w/proj",
            "tool_name": "Edit",
            "tool_input": {"file_path": "/w/proj/src/main.rs", "old_string": "a", "new_string": "b"}
        }));
        let Transition::Write(s) = transition(&pre, None, 5) else {
            panic!()
        };
        assert_eq!(s.line.as_deref(), Some("Edit: main.rs"));
        assert_eq!(s.cwd.as_deref(), Some("/w/proj"));
        assert_eq!(s.updated_at, 5);

        let post = input(ev("PostToolUse"));
        let Transition::Write(s) = transition(&post, Some(&s), 6) else {
            panic!()
        };
        assert_eq!(s.line.as_deref(), Some("Edit: main.rs"));

        let stop = input(json!({
            "session_id": "s1", "hook_event_name": "Stop",
            "last_assistant_message": "リファクタリングが終わりました。\n\n## 変更点\n- a"
        }));
        let Transition::Write(s) = transition(&stop, Some(&s), 7) else {
            panic!()
        };
        assert_eq!(
            s.line.as_deref(),
            Some("リファクタリングが終わりました。 ## 変更点 - a")
        );

        let prompt = input(ev("UserPromptSubmit"));
        let Transition::Write(s) = transition(&prompt, Some(&s), 8) else {
            panic!()
        };
        assert_eq!(s.line, None);
    }

    #[test]
    fn tool_summaries() {
        assert_eq!(
            tool_summary("Bash", Some(&json!({"command": "npm   test\n--watch"}))),
            "Bash: npm test --watch"
        );
        assert_eq!(
            tool_summary("Read", Some(&json!({"file_path": "C:\\p\\src\\index.ts"}))),
            "Read: index.ts"
        );
        assert_eq!(
            tool_summary("mcp__srv__do", Some(&json!({"n": 1, "q": "hello"}))),
            "mcp__srv__do: hello"
        );
        assert_eq!(tool_summary("Glob", None), "Glob");
    }

    #[test]
    fn one_line_truncates_by_chars() {
        let long = "あ".repeat(200);
        let out = one_line(&long);
        assert_eq!(out.chars().count(), LINE_MAX_CHARS);
        assert!(out.ends_with('…'));
        assert_eq!(one_line("  short\ttext \n"), "short text");
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
