//! OpenCode v2 のイベントを、Claude Code のフックの入力の形に読み替える。
//!
//! イベントは `marimo-hook install` が OpenCode の設定のフォルダの `plugins/` に置くプラグインが、
//! `ctx.event.subscribe` で受けたものから選んで送ってくる。形は OpenCode の本体の定義
//! （Session.Event、Permission、Form の各イベント）に従い、どのイベントも
//! `{id, created, type, location?, data}` を持つ。プラグインは、イベントだけでは分からない値
//! （ツールの名前、最後の応答の文章、モデルのコンテキストの上限、途中から見たセッションのタイトルと場所、
//! サブエージェントのイベントを書く親の行）を同じ JSON の最上位に足して送る。状態の決め方は Claude Code と
//! 同じ規則を使うので、ここではイベント名とツールの名前と引数を Claude Code のものに置き換えるだけにする。

use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{ContextUsage, HookInput, Provider, clean_title};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpencodeInput {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub data: Value,
    /// プラグインが足す値。ツールの実行のイベントはツールの名前を持たないので、入力の始まりの
    /// イベントで覚えた名前を入れてくる。
    #[serde(default)]
    pub tool: Option<String>,
    /// プラグインが足す値。ターンの終わりに、そのターンで最後に書かれた応答の文章を入れてくる。
    #[serde(default)]
    pub last_text: Option<String>,
    /// プラグインが足す値。モデルの一覧から引いたコンテキストの上限で、引けなければ入らない。
    #[serde(default)]
    pub context_window: Option<u64>,
    /// プラグインが足す値。作成のイベントを見ていないセッションを初めて見たときに、OpenCode から
    /// 引いたタイトルと作業フォルダを入れてくる。
    #[serde(default)]
    pub session: Option<SessionMeta>,
    /// プラグインが足す値。サブエージェントのセッションのイベントに、親をたどった先の親を持たない
    /// セッションの ID を入れてくる。
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Location {
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionMeta {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Claude Code のフックの入力に読み替えたもの。状態の遷移はこれで決める。
    Hook(Box<HookInput>),
    /// 状態は変えず、タイトルだけを書き換える。
    Title(String),
    /// 状態は変えず、コンテキストの使用量だけを書き換える。
    Context(ContextUsage),
    Ignore,
}

impl OpencodeInput {
    /// 状態を書く行のセッションの ID。サブエージェントのイベントは親の行に書く。
    pub fn session_id(&self) -> &str {
        self.parent
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| self.event_session_id())
    }

    /// サブエージェントのイベントなら、そのサブエージェントのセッションの ID を返す。
    fn subagent(&self) -> Option<&str> {
        self.parent
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|_| self.event_session_id())
            .filter(|s| !s.is_empty())
    }

    /// フォームの作成のイベントだけは、セッションの ID をフォームの中に持つ。
    fn event_session_id(&self) -> &str {
        let data = &self.data;
        data.get("sessionID")
            .or_else(|| data.pointer("/form/sessionID"))
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// 行のタイトル。作成と改名のイベントが持つタイトルか、プラグインが引いたタイトルを使う。
    pub fn title(&self) -> Option<String> {
        let from_event = matches!(self.kind.as_str(), "session.created" | "session.renamed")
            .then(|| self.data.get("title").and_then(Value::as_str))
            .flatten();
        from_event
            .or_else(|| self.session.as_ref().and_then(|s| s.title.as_deref()))
            .and_then(clean_title)
    }

    fn cwd(&self) -> Option<String> {
        self.data
            .pointer("/location/directory")
            .and_then(Value::as_str)
            .or_else(|| self.location.as_ref().and_then(|l| l.directory.as_deref()))
            .or_else(|| self.session.as_ref().and_then(|s| s.directory.as_deref()))
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
    }
}

pub fn translate(input: &OpencodeInput, now_ms: u64) -> Event {
    let session_id = input.session_id();
    if session_id.is_empty() {
        return Event::Ignore;
    }
    let data = &input.data;
    let text = |pointer: &str| data.pointer(pointer).and_then(Value::as_str);
    let agent = input.subagent();
    let hook = |name: &str| HookInput {
        provider: Provider::Opencode,
        session_id: session_id.to_owned(),
        hook_event_name: name.to_owned(),
        cwd: input.cwd(),
        agent_id: agent.map(str::to_owned),
        ..HookInput::default()
    };
    if agent.is_some() {
        // プラグインがサブエージェントから渡すのは、ターンの始まりと終わり、利用者の答えを待つイベントとその答えだけ。
        // ターンが始まったら親の行の作業に加え、答えを待つ間は親の行の承認待ちとして出し、ターンが終わったら外す。
        match input.kind.as_str() {
            "permission.asked" | "permission.replied" | "form.created" | "form.replied"
            | "form.cancelled" => {}
            "session.execution.started" => return Event::Hook(Box::new(hook("SubagentStart"))),
            "session.execution.interrupted" if text("/reason") == Some("superseded") => {
                return Event::Ignore;
            }
            "session.execution.succeeded"
            | "session.execution.failed"
            | "session.execution.interrupted" => {
                return Event::Hook(Box::new(hook("SubagentStop")));
            }
            _ => return Event::Ignore,
        }
    }
    let event = match input.kind.as_str() {
        // サブエージェントは親の行の作業として見えるので、自分の行を持たせない。プラグインも捨てるが、
        // 作成のイベントだけは親を持つかをここでも確かめられる。
        "session.created" if text("/parentID").is_some_and(|p| !p.is_empty()) => {
            return Event::Ignore;
        }
        "session.created" => hook("SessionStart"),
        "session.renamed" => return input.title().map_or(Event::Ignore, Event::Title),
        // 消したセッションは開き直せないので、まだ読んでいない完了やエラーの行も残さない。
        // Claude Code で利用者が会話を捨てたときの reason を渡して、そう扱わせる。
        "session.deleted" => HookInput {
            reason: Some("clear".to_owned()),
            ..hook("SessionEnd")
        },
        "session.execution.started" => hook("UserPromptSubmit"),
        "session.execution.succeeded" => HookInput {
            last_assistant_message: input.last_text.clone(),
            ..hook("Stop")
        },
        // step.failed と retry.scheduled はその後に再試行されるので、ターンの失敗だけをエラーにする。
        "session.execution.failed" => HookInput {
            error: text("/error/type").map(str::to_owned),
            last_assistant_message: text("/error/message").map(str::to_owned),
            ..hook("StopFailure")
        },
        // superseded は次のターンがすぐ始まるので、待機へ戻さない。
        "session.execution.interrupted" => match text("/reason") {
            Some("superseded") => return Event::Ignore,
            _ => hook("Interrupt"),
        },
        "session.tool.input.started" | "session.tool.called" => {
            let name = text("/name").or(input.tool.as_deref()).unwrap_or("");
            let (tool_name, tool_input) = tool(name, data.get("input"));
            HookInput {
                tool_name: Some(tool_name),
                tool_input,
                ..hook("PreToolUse")
            }
        }
        "session.tool.success" => hook("PostToolUse"),
        "session.tool.failed" => hook("PostToolUseFailure"),
        "permission.asked" => {
            let (tool_name, tool_input) = permission_tool(data);
            HookInput {
                tool_name: Some(tool_name),
                tool_input: Some(tool_input),
                ..hook("PermissionRequest")
            }
        }
        "permission.replied" if text("/reply") == Some("reject") => hook("PermissionDenied"),
        "permission.replied" => hook("PostToolUse"),
        // 質問のツールはフォームを作って利用者の答えを待つ。Claude Code の質問のツールとして出す。
        "form.created" => HookInput {
            tool_name: Some("AskUserQuestion".to_owned()),
            tool_input: Some(json!({ "questions": questions(data.get("form")) })),
            ..hook("PreToolUse")
        },
        "form.replied" | "form.cancelled" => hook("PostToolUse"),
        "session.step.ended" => {
            return context(data, input.context_window, now_ms)
                .map_or(Event::Ignore, Event::Context);
        }
        _ => return Event::Ignore,
    };
    Event::Hook(Box::new(event))
}

// OpenCode のツールを、要約の作り方が同じ Claude Code のツールに置き換える。引数の名前は OpenCode の
// 組み込みのツール（opencode.tool.shell、read、write、edit、patch、glob、grep、webfetch、websearch、
// subagent、skill）の入力の定義に従う。v1 の名前（bash、filePath）も同じ扱いにする。
fn tool(name: &str, args: Option<&Value>) -> (String, Option<Value>) {
    let arg = |key: &str| args.and_then(|a| a.get(key)).cloned().unwrap_or_default();
    let path = || {
        args.and_then(|a| a.get("path").or_else(|| a.get("filePath")))
            .cloned()
            .unwrap_or_default()
    };
    let (claude, input) = match name {
        "shell" | "bash" => (
            "Bash",
            json!({ "command": arg("command"), "description": arg("description") }),
        ),
        "read" => ("Read", json!({ "file_path": path() })),
        "write" => ("Write", json!({ "file_path": path() })),
        "edit" => ("Edit", json!({ "file_path": path() })),
        // パッチの書式は Codex の apply_patch と同じなので、対象のファイルの読み方も同じにする。
        "patch" | "apply_patch" => ("apply_patch", json!({ "command": arg("patchText") })),
        "glob" => (
            "Glob",
            json!({ "pattern": arg("pattern"), "path": arg("path") }),
        ),
        "grep" => (
            "Grep",
            json!({ "pattern": arg("pattern"), "path": arg("path") }),
        ),
        "webfetch" => ("WebFetch", json!({ "url": arg("url") })),
        "websearch" => ("WebSearch", json!({ "query": arg("query") })),
        "subagent" | "task" => ("Agent", json!({ "description": arg("description") })),
        _ => return (name.to_owned(), args.cloned()),
    };
    (claude.to_owned(), args.map(|_| input))
}

// 許可の要求は、ツールの名前ではなく操作の種類（action）と対象（resources）で届く。シェルの対象は
// 実行するコマンドで、ファイルの操作の対象はパスである。v1 の名前（permission、patterns）も読む。
fn permission_tool(data: &Value) -> (String, Value) {
    let action = data
        .get("action")
        .or_else(|| data.get("permission"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let resources: Vec<&str> = data
        .get("resources")
        .or_else(|| data.get("patterns"))
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let first = resources.first().copied().unwrap_or("");
    let (claude, input) = match action {
        "shell" | "bash" => {
            let command = data
                .pointer("/metadata/command")
                .and_then(Value::as_str)
                .map_or_else(|| resources.join(" "), str::to_owned);
            ("Bash", json!({ "command": command }))
        }
        "read" => ("Read", json!({ "file_path": first })),
        "edit" => ("Edit", json!({ "file_path": first })),
        "glob" => ("Glob", json!({ "pattern": first })),
        "grep" => ("Grep", json!({ "pattern": first })),
        "webfetch" => ("WebFetch", json!({ "url": first })),
        "websearch" => ("WebSearch", json!({ "query": first })),
        _ => return (action.to_owned(), json!({ "resources": resources })),
    };
    (claude.to_owned(), input)
}

// フォームの各フィールドを質問として並べる。質問のツールは、問いの文をフィールドの説明（description）に、短い見出しを
// 題（title）に入れ、選択肢を value と label のペアにする。ほかのフォームは説明を持たないこともある。
fn questions(form: Option<&Value>) -> Value {
    let fields = form
        .and_then(|f| f.get("fields"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    fields
        .iter()
        .filter(|f| f.get("hidden").and_then(Value::as_bool) != Some(true))
        .map(|f| {
            let question = f
                .get("description")
                .or_else(|| f.get("title"))
                .or_else(|| form.and_then(|f| f.get("title")))
                .cloned()
                .unwrap_or_default();
            let options: Vec<Value> = f
                .get("options")
                .and_then(Value::as_array)
                .map(|os| {
                    os.iter()
                        .filter_map(|o| o.get("label").and_then(Value::as_str))
                        .map(|l| json!({ "label": l }))
                        .collect()
                })
                .unwrap_or_default();
            json!({ "question": question, "options": options })
        })
        .collect()
}

// ステップの終わりのトークン数は、そのステップでモデルに渡した入力と、次のステップでも会話に残る
// 出力の合計で、OpenCode の端末の画面がコンテキストの使用量として出す値と同じ数え方にする。
// session.usage.updated はセッションの累計なので使わない。
fn context(data: &Value, window: Option<u64>, now_ms: u64) -> Option<ContextUsage> {
    let tokens = data.get("tokens")?;
    let count = |pointer: &str| tokens.pointer(pointer).and_then(Value::as_u64);
    let parts = [
        count("/input")?,
        count("/output").unwrap_or(0),
        count("/reasoning").unwrap_or(0),
        count("/cache/read").unwrap_or(0),
        count("/cache/write").unwrap_or(0),
    ];
    let total: u64 = parts.iter().sum();
    if total == 0 {
        return None;
    }
    let window = window.filter(|w| *w > 0);
    Some(ContextUsage {
        used_percentage: window.map(|w| (total as f64 / w as f64 * 100.0).min(100.0)),
        total_input_tokens: Some(total),
        context_window_size: window,
        updated_at: now_ms,
        source: Some("opencode-step".to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{SessionState, Status, Transition, transition};

    const SID: &str = "ses_0123456789abcdefghijklmn";

    fn ev(kind: &str, data: Value) -> OpencodeInput {
        let mut data = data;
        if data.get("sessionID").is_none() && data.get("form").is_none() {
            data["sessionID"] = json!(SID);
        }
        OpencodeInput {
            kind: kind.to_owned(),
            location: Some(Location {
                directory: Some("/w/proj".to_owned()),
            }),
            data,
            ..OpencodeInput::default()
        }
    }

    fn run(events: &[OpencodeInput]) -> Option<SessionState> {
        let mut state: Option<SessionState> = None;
        for (i, e) in events.iter().enumerate() {
            let now = 1000 + i as u64;
            match translate(e, now) {
                Event::Hook(h) => match transition(&h, state.as_ref(), now) {
                    Transition::Write(next) => state = Some(*next),
                    Transition::Delete => state = None,
                    Transition::Nothing => {}
                },
                Event::Title(t) => {
                    if let Some(s) = state.as_mut() {
                        s.title = Some(t);
                    }
                }
                Event::Context(c) => {
                    if let Some(s) = state.as_mut() {
                        s.context = Some(c);
                    }
                }
                Event::Ignore => {}
            }
        }
        state
    }

    fn started() -> OpencodeInput {
        ev("session.execution.started", json!({}))
    }

    fn shell(kind: &str) -> OpencodeInput {
        OpencodeInput {
            tool: Some("shell".to_owned()),
            ..ev(
                kind,
                json!({ "id": "call_1", "assistantMessageID": "msg_1", "input": { "command": "ls -la" } }),
            )
        }
    }

    #[test]
    fn event_sequences_end_in_the_expected_state() {
        type Expected = Option<(Status, Option<&'static str>)>;
        let cases: Vec<(&str, Vec<OpencodeInput>, Expected)> = vec![
            (
                "created",
                vec![ev(
                    "session.created",
                    json!({ "title": "題", "location": { "directory": "/w/a" } }),
                )],
                Some((Status::Idle, None)),
            ),
            ("started", vec![started()], Some((Status::Working, None))),
            (
                "succeeded",
                vec![started(), ev("session.execution.succeeded", json!({}))],
                Some((Status::Done, None)),
            ),
            (
                "failed",
                vec![
                    started(),
                    ev(
                        "session.execution.failed",
                        json!({ "error": { "type": "provider.rate_limit", "message": "slow down" } }),
                    ),
                ],
                Some((Status::Error, Some("provider.rate_limit"))),
            ),
            (
                "retries are not failures",
                vec![
                    started(),
                    ev(
                        "session.step.failed",
                        json!({ "error": { "type": "x", "message": "y" } }),
                    ),
                    ev("session.retry.scheduled", json!({ "attempt": 1 })),
                ],
                Some((Status::Working, None)),
            ),
            (
                "interrupted by the user",
                vec![
                    started(),
                    ev("session.execution.interrupted", json!({ "reason": "user" })),
                ],
                Some((Status::Idle, None)),
            ),
            (
                "interrupted by shutdown",
                vec![
                    started(),
                    ev(
                        "session.execution.interrupted",
                        json!({ "reason": "shutdown" }),
                    ),
                ],
                Some((Status::Idle, None)),
            ),
            (
                "superseded keeps working",
                vec![
                    started(),
                    ev(
                        "session.execution.interrupted",
                        json!({ "reason": "superseded" }),
                    ),
                ],
                Some((Status::Working, None)),
            ),
            (
                "permission asked",
                vec![
                    started(),
                    ev(
                        "permission.asked",
                        json!({ "id": "per_1", "action": "shell", "resources": ["ls"] }),
                    ),
                ],
                Some((Status::Waiting, Some("permission"))),
            ),
            (
                "permission granted",
                vec![
                    started(),
                    ev("permission.asked", json!({ "action": "shell" })),
                    ev(
                        "permission.replied",
                        json!({ "requestID": "per_1", "reply": "once" }),
                    ),
                ],
                Some((Status::Working, None)),
            ),
            (
                "permission rejected",
                vec![
                    started(),
                    ev("permission.asked", json!({ "action": "shell" })),
                    ev(
                        "permission.replied",
                        json!({ "requestID": "per_1", "reply": "reject" }),
                    ),
                ],
                Some((Status::Working, None)),
            ),
            (
                "form waits for an answer",
                vec![
                    started(),
                    ev(
                        "form.created",
                        json!({ "form": { "id": "frm_1", "sessionID": SID, "title": "Questions", "fields": [] } }),
                    ),
                ],
                Some((Status::Waiting, Some("question"))),
            ),
            (
                "form answered",
                vec![
                    started(),
                    ev(
                        "form.created",
                        json!({ "form": { "id": "frm_1", "sessionID": SID, "fields": [] } }),
                    ),
                    ev("form.replied", json!({ "id": "frm_1", "answer": {} })),
                ],
                Some((Status::Working, None)),
            ),
            (
                "form cancelled",
                vec![
                    started(),
                    ev(
                        "form.created",
                        json!({ "form": { "id": "frm_1", "sessionID": SID, "fields": [] } }),
                    ),
                    ev("form.cancelled", json!({ "id": "frm_1" })),
                ],
                Some((Status::Working, None)),
            ),
            (
                "tool runs",
                vec![
                    started(),
                    shell("session.tool.called"),
                    ev("session.tool.success", json!({ "id": "call_1" })),
                ],
                Some((Status::Working, None)),
            ),
            (
                "deleted",
                vec![started(), ev("session.deleted", json!({}))],
                None,
            ),
            (
                "deleted after done",
                vec![
                    started(),
                    ev("session.execution.succeeded", json!({})),
                    ev("session.deleted", json!({})),
                ],
                None,
            ),
            (
                "subagent",
                vec![ev(
                    "session.created",
                    json!({ "parentID": "ses_parent", "title": "child" }),
                )],
                None,
            ),
            (
                "no session id",
                vec![OpencodeInput {
                    kind: "session.execution.started".to_owned(),
                    ..OpencodeInput::default()
                }],
                None,
            ),
            (
                "unknown events",
                vec![
                    ev("session.text.delta", json!({ "delta": "x" })),
                    ev("session.usage.updated", json!({ "tokens": { "input": 1 } })),
                ],
                None,
            ),
        ];
        for (name, events, expected) in cases {
            let state = run(&events);
            assert_eq!(
                state
                    .as_ref()
                    .map(|s| (s.status, s.status_reason.as_deref())),
                expected,
                "{name}"
            );
            if let Some(s) = state {
                assert_eq!(s.provider, Provider::Opencode, "{name}");
            }
        }
    }

    #[test]
    fn subagent_waits_show_on_the_parent_row() {
        let child = |kind: &str, data: Value| {
            let mut data = data;
            if data.get("form").is_none() {
                data["sessionID"] = json!("ses_child");
            }
            OpencodeInput {
                parent: Some(SID.to_owned()),
                ..ev(kind, data)
            }
        };
        let asked = || {
            child(
                "permission.asked",
                json!({ "action": "shell", "resources": ["rm x"] }),
            )
        };
        let replied = |reply: &str| {
            child(
                "permission.replied",
                json!({ "requestID": "per_1", "reply": reply }),
            )
        };
        let done = || vec![started(), ev("session.execution.succeeded", json!({}))];
        let with = |mut events: Vec<OpencodeInput>, more: Vec<OpencodeInput>| {
            events.extend(more);
            events
        };
        let cases: Vec<(&str, Vec<OpencodeInput>, Status)> = vec![
            (
                "child running",
                with(done(), vec![child("session.execution.started", json!({}))]),
                Status::Working,
            ),
            ("asked", with(done(), vec![asked()]), Status::Waiting),
            (
                "granted while still running",
                with(done(), vec![asked(), replied("once")]),
                Status::Working,
            ),
            (
                "rejected",
                with(done(), vec![asked(), replied("reject")]),
                Status::Working,
            ),
            (
                "child finished",
                with(
                    done(),
                    vec![
                        asked(),
                        replied("once"),
                        child("session.execution.succeeded", json!({})),
                    ],
                ),
                Status::Done,
            ),
            (
                "child superseded",
                with(
                    done(),
                    vec![
                        asked(),
                        child(
                            "session.execution.interrupted",
                            json!({ "reason": "superseded" }),
                        ),
                    ],
                ),
                Status::Waiting,
            ),
            (
                "question",
                with(
                    done(),
                    vec![child(
                        "form.created",
                        json!({ "form": { "id": "frm_1", "sessionID": "ses_child", "fields": [] } }),
                    )],
                ),
                Status::Waiting,
            ),
            (
                "answered",
                with(
                    done(),
                    vec![
                        child(
                            "form.created",
                            json!({ "form": { "id": "frm_1", "sessionID": "ses_child", "fields": [] } }),
                        ),
                        child("form.cancelled", json!({ "id": "frm_1" })),
                        child("session.execution.failed", json!({})),
                    ],
                ),
                Status::Done,
            ),
            (
                "other child events",
                with(
                    done(),
                    vec![child("session.tool.success", json!({ "id": "c" }))],
                ),
                Status::Done,
            ),
        ];
        for (name, events, expected) in cases {
            let s = run(&events).unwrap();
            assert_eq!(
                (s.session_id.as_str(), s.status, s.own_status),
                (SID, expected, Some(Status::Done)),
                "{name}"
            );
        }
    }

    #[test]
    fn rows_carry_the_title_folder_and_last_reply() {
        let created = || {
            ev(
                "session.created",
                json!({ "title": "最初の題", "location": { "directory": "/w/a" } }),
            )
        };
        assert_eq!(run(&[created()]).unwrap().cwd.as_deref(), Some("/w/a"));
        let s = run(&[
            created(),
            ev("session.renamed", json!({ "title": "  付け直した題 " })),
            started(),
            OpencodeInput {
                last_text: Some("終わったよ".to_owned()),
                ..ev("session.execution.succeeded", json!({}))
            },
        ])
        .unwrap();
        assert_eq!(s.title.as_deref(), Some("付け直した題"));
        assert_eq!(s.activity.unwrap().summary, "終わったよ");

        let looked_up = OpencodeInput {
            location: None,
            session: Some(SessionMeta {
                title: Some("引いた題".to_owned()),
                directory: Some("/w/b".to_owned()),
            }),
            ..started()
        };
        assert_eq!(looked_up.title().as_deref(), Some("引いた題"));
        assert_eq!(run(&[looked_up]).unwrap().cwd.as_deref(), Some("/w/b"));
        // タイトルは作成と改名のイベントか、プラグインが引いた値からだけ取る。
        assert_eq!(
            ev("session.execution.started", json!({ "title": "x" })).title(),
            None
        );
    }

    #[test]
    fn tools_become_their_claude_code_counterparts() {
        let called = run(&[started(), shell("session.tool.called")]).unwrap();
        let a = called.activity.unwrap();
        assert_eq!(a.tool.as_deref(), Some("Bash"));
        assert_eq!(a.summary, "ls -la");

        // 入力の始まりのイベントは名前だけを持つ。
        let input_started = run(&[
            started(),
            ev(
                "session.tool.input.started",
                json!({ "id": "call_2", "name": "read" }),
            ),
        ])
        .unwrap();
        assert_eq!(
            input_started.activity.unwrap().tool.as_deref(),
            Some("Read")
        );

        let t = |name: &str, args: Value| tool(name, Some(&args));
        assert_eq!(
            t("read", json!({ "path": "/a/b.txt", "offset": 3 })),
            ("Read".to_owned(), Some(json!({ "file_path": "/a/b.txt" })))
        );
        assert_eq!(
            t("edit", json!({ "filePath": "/a/c.rs" })).1,
            Some(json!({ "file_path": "/a/c.rs" }))
        );
        assert_eq!(
            t("patch", json!({ "patchText": "*** Begin Patch" })),
            (
                "apply_patch".to_owned(),
                Some(json!({ "command": "*** Begin Patch" }))
            )
        );
        assert_eq!(t("subagent", json!({ "description": "調べる" })).0, "Agent");
        assert_eq!(
            t("mcp_tool", json!({ "q": 1 })),
            ("mcp_tool".to_owned(), Some(json!({ "q": 1 })))
        );
        assert_eq!(tool("shell", None), ("Bash".to_owned(), None));
    }

    #[test]
    fn permissions_and_forms_describe_what_waits() {
        let shell = run(&[
            started(),
            ev(
                "permission.asked",
                json!({ "action": "shell", "resources": ["rm -rf build"], "metadata": {} }),
            ),
        ])
        .unwrap();
        let a = shell.activity.unwrap();
        assert_eq!(a.tool.as_deref(), Some("Bash"));
        assert_eq!(a.summary, "rm -rf build");

        let edit = run(&[
            started(),
            ev(
                "permission.asked",
                json!({ "action": "edit", "resources": ["/w/proj/src/main.rs"] }),
            ),
        ])
        .unwrap();
        assert_eq!(edit.activity.unwrap().summary, "編集: src/main.rs");

        let form = run(&[
            started(),
            ev(
                "form.created",
                json!({ "form": { "id": "frm_1", "sessionID": SID, "title": "Questions", "fields": [
                    { "key": "q0", "type": "string", "title": "ブランチ", "description": "どちらにする？",
                      "options": [{ "value": "a", "label": "main" }, { "value": "b", "label": "dev" }] }
                ] } }),
            ),
        ])
        .unwrap();
        let a = form.activity.unwrap();
        assert_eq!(a.summary, "質問: どちらにする？");
        assert_eq!(a.detail.as_deref(), Some("main / dev"));
    }

    #[test]
    fn context_comes_from_the_tokens_of_each_step() {
        let step = |window: Option<u64>| OpencodeInput {
            context_window: window,
            ..ev(
                "session.step.ended",
                json!({ "tokens": { "input": 1000, "output": 500, "reasoning": 500,
                    "cache": { "read": 48000, "write": 0 } } }),
            )
        };
        let Event::Context(c) = translate(&step(Some(200_000)), 5) else {
            panic!()
        };
        assert_eq!(c.used_percentage, Some(25.0));
        assert_eq!(c.total_input_tokens, Some(50_000));
        assert_eq!(c.context_window_size, Some(200_000));
        assert_eq!(c.source.as_deref(), Some("opencode-step"));
        let Event::Context(c) = translate(&step(None), 5) else {
            panic!()
        };
        assert_eq!(
            (c.used_percentage, c.total_input_tokens),
            (None, Some(50_000))
        );
        assert_eq!(
            translate(&ev("session.step.ended", json!({})), 5),
            Event::Ignore
        );
    }
}
