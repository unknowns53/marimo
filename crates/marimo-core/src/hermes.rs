//! Hermes Agent のシェルフックの入力を、Claude Code のフックの入力の形に読み替える。
//!
//! 項目名とイベント名は Hermes の agent/shell_hooks.py（_serialize_payload）と、各イベントを発火する
//! 箇所の引数に従う。Hermes のフックはどのイベントでも hook_event_name、tool_name、tool_input、
//! session_id、cwd を持ち、それ以外の引数は extra にまとめて入る。状態の決め方は Claude Code と
//! 同じ規則を使うので、ここではイベント名とツールの名前と引数を Claude Code のものに置き換えるだけにする。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::paths::user_home;
use crate::state::{ContextUsage, HookInput, Provider, SessionState, clean_title};

/// Hermes の設定とデータを置くフォルダ。Hermes と同じく HERMES_HOME があればそれを使い、なければ
/// Windows では %LOCALAPPDATA%\hermes、ほかの OS では ~/.hermes を使う。Hermes の起動用のスクリプトは
/// HERMES_HOME を設定してから本体を動かすので、フックからはふつう HERMES_HOME が見える。
pub fn hermes_home() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("HERMES_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if cfg!(windows) {
        return env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(|d| PathBuf::from(d).join("hermes"));
    }
    user_home().map(|h| h.join(".hermes"))
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HermesInput {
    pub hook_event_name: String,
    /// gateway を止めたときの agent_loop_stopped のように、空で届くイベントもある。
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Claude Code のフックの入力に読み替えたもの。状態の遷移はこれで決める。
    Hook(Box<HookInput>),
    /// 状態は変えず、コンテキストの使用量だけを書き換える。
    Context(ContextUsage),
    Ignore,
}

// 承認の返事のうち、コマンドが実行されずに終わるもの（tools/approval*.py の choice）。
const REFUSED: [&str; 5] = [
    "deny",
    "timeout",
    "cancelled",
    "notify_failed",
    "smart_deny",
];

pub fn translate(
    input: &HermesInput,
    current: Option<&SessionState>,
    hermes_home: Option<&Path>,
    now_ms: u64,
) -> Event {
    if input.session_id.is_empty() {
        return Event::Ignore;
    }
    let extra = &input.extra;
    let text = |key: &str| extra.get(key).and_then(Value::as_str).map(str::to_owned);
    let flag = |key: &str| extra.get(key).and_then(Value::as_bool).unwrap_or(false);
    // Hermes は応答の後に、同じ session_id のまま記憶やスキルの見直しを裏で走らせる
    // （agent/background_review.py）。その呼び出しは pre_llm_call を出さないので、今のターンで知らされた
    // turn_id と違うイベントは、完了の行を作業中へ戻さないよう捨てる。どちらかが分からなければ受ける。
    let in_turn = match (
        extra.get("turn_id").and_then(Value::as_str),
        current.and_then(|c| c.turn_id.as_deref()),
    ) {
        (Some(event), Some(turn)) => event == turn,
        _ => true,
    };
    let hook = |name: &str| HookInput {
        provider: Provider::Hermes,
        session_id: input.session_id.clone(),
        hook_event_name: name.to_owned(),
        cwd: work_dir(input.cwd.as_deref(), hermes_home),
        ..HookInput::default()
    };
    let event = match input.hook_event_name.as_str() {
        // on_session_start は新しい会話の最初のターンで、on_session_reset は /new で作られた新しい
        // session_id で届く。
        "on_session_start" | "on_session_reset" => hook("SessionStart"),
        "pre_llm_call" => HookInput {
            turn_id: text("turn_id"),
            ..hook("UserPromptSubmit")
        },
        // /new で捨てた会話と、gateway や CLI を終えたときに届く。Hermes の on_session_end は
        // ターンの終わりごとに届くもので、会話の終わりではない。
        "on_session_finalize" => hook("SessionEnd"),
        _ if !in_turn => return Event::Ignore,
        name @ ("pre_tool_call" | "post_tool_call") => {
            let (tool_name, tool_input) = tool(
                input.tool_name.as_deref().unwrap_or(""),
                input.tool_input.as_ref(),
            );
            let event = if name == "pre_tool_call" {
                "PreToolUse"
            } else {
                "PostToolUse"
            };
            HookInput {
                tool_name: Some(tool_name),
                tool_input,
                ..hook(event)
            }
        }
        // 危険なコマンドの承認を利用者に求めている間。承認を求めるのはシェルのコマンドなので、
        // Bash の実行許可として出す。
        "pre_approval_request" => HookInput {
            tool_name: Some("Bash".to_owned()),
            tool_input: Some(json!({ "command": text("command") })),
            ..hook("PermissionRequest")
        },
        "post_approval_response" => {
            let refused = text("choice").is_some_and(|c| REFUSED.contains(&c.as_str()));
            hook(if refused {
                "PermissionDenied"
            } else {
                "PostToolUse"
            })
        }
        // 最終の応答があり、中断されなかったターンの終わりにだけ届く。
        "post_llm_call" => HookInput {
            last_assistant_message: text("assistant_response"),
            ..hook("Stop")
        },
        "on_session_end" if flag("interrupted") => hook("Interrupt"),
        "on_session_end" if flag("failed") => HookInput {
            error: text("turn_exit_reason"),
            ..hook("StopFailure")
        },
        "on_session_end" if flag("completed") => hook("Stop"),
        // やり直しの効かないエラーでは、ターンの終わりのフックが届かないまま応答が終わることがある
        // （agent/conversation_loop.py の早期の return）。reason は rate_limit などの分類の名前で、
        // Claude Code の StopFailure の error と同じくセリフの選び分けに使える。
        "api_request_error" if extra.get("retryable").and_then(Value::as_bool) == Some(false) => {
            HookInput {
                error: text("reason"),
                last_assistant_message: extra
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                ..hook("StopFailure")
            }
        }
        "post_api_request" => {
            return context(extra, now_ms).map_or(Event::Ignore, Event::Context);
        }
        _ => return Event::Ignore,
    };
    Event::Hook(Box::new(event))
}

// gateway は HERMES_HOME を作業フォルダにして動くので、その場所は会話の居場所ではない。行の名前が
// どれも同じフォルダ名にならないよう、HERMES_HOME と同じ cwd は捨てて、会話の題名を名前に使う。
fn work_dir(cwd: Option<&str>, hermes_home: Option<&Path>) -> Option<String> {
    let cwd = cwd.filter(|c| !c.is_empty())?;
    let norm = |p: &str| {
        let p = p.trim_end_matches(['/', '\\']).replace('\\', "/");
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    let is_home = hermes_home.is_some_and(|h| norm(cwd) == norm(&h.to_string_lossy()));
    (!is_home).then(|| cwd.to_owned())
}

// Hermes のツールを、要約の作り方が同じ Claude Code のツールに置き換える。引数の名前は Hermes の
// tools/file_tools.py、terminal_tool.py、web_tools.py、delegate_tool.py、clarify_tool.py のスキーマに従う。
fn tool(name: &str, args: Option<&Value>) -> (String, Option<Value>) {
    let arg = |key: &str| args.and_then(|a| a.get(key)).cloned().unwrap_or_default();
    let (claude, input) = match name {
        "terminal" => ("Bash", json!({ "command": arg("command") })),
        "read_file" => ("Read", json!({ "file_path": arg("path") })),
        "write_file" => ("Write", json!({ "file_path": arg("path") })),
        "patch" => ("Edit", json!({ "file_path": arg("path") })),
        "search_files" => (
            "Grep",
            json!({ "pattern": arg("pattern"), "path": arg("path") }),
        ),
        "web_search" => ("WebSearch", json!({ "query": arg("query") })),
        "web_extract" => ("WebFetch", json!({ "url": first_url(args) })),
        "delegate_task" => ("Agent", json!({ "description": arg("goal") })),
        // 選択肢は文字列の配列で届くので、Codex の質問のツールと同じ label を持つ形にする。
        "clarify" => (
            "request_user_input",
            json!({ "questions": questions(args) }),
        ),
        _ => return (name.to_owned(), args.cloned()),
    };
    (claude.to_owned(), Some(input))
}

fn first_url(args: Option<&Value>) -> Value {
    let first = args.and_then(|a| a.pointer("/urls/0"));
    let url = match first {
        Some(Value::String(s)) => Some(s.as_str()),
        Some(item) => item
            .get("url")
            .or_else(|| item.get("href"))
            .and_then(Value::as_str),
        None => None,
    };
    url.map_or(Value::Null, |u| Value::String(u.to_owned()))
}

fn questions(args: Option<&Value>) -> Value {
    let items = args
        .and_then(|a| a.get("questions"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    items
        .iter()
        .map(|q| {
            let options: Vec<Value> = q
                .get("choices")
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .filter_map(Value::as_str)
                        .map(|c| json!({ "label": c }))
                        .collect()
                })
                .unwrap_or_default();
            json!({ "question": q.get("question").cloned().unwrap_or_default(), "options": options })
        })
        .collect()
}

// post_api_request の usage.prompt_tokens は、キャッシュから読んだ分も含めてモデルに渡した入力の
// トークン数で、次の呼び出しでもコンテキストに残る量である。context_length はそのモデルの上限。
fn context(extra: &Value, now_ms: u64) -> Option<ContextUsage> {
    let tokens = extra
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_u64)?;
    let window = extra
        .get("context_length")
        .and_then(Value::as_u64)
        .filter(|w| *w > 0);
    Some(ContextUsage {
        used_percentage: window.map(|w| (tokens as f64 / w as f64 * 100.0).min(100.0)),
        total_input_tokens: Some(tokens),
        context_window_size: window,
        updated_at: now_ms,
        source: Some("hermes-api".to_owned()),
    })
}

/// 会話がどのチャットのものか。フックの入力には入っていないので、Hermes のセッションの一覧から読む。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatMeta {
    pub title: Option<String>,
    pub link: Option<String>,
}

/// gateway が `sessions/sessions.json` に書く、チャットの鍵から今の session_id への対応表を読む。
/// Hermes はこのファイルを state.db の写し（legacy mirror）と書いており、無くなっても題名とリンクが
/// 出なくなるだけで状態は変わらない。CLI の会話はこの表に載らない。
pub fn chat_meta(hermes_home: &Path, session_id: &str) -> ChatMeta {
    let path = hermes_home.join("sessions").join("sessions.json");
    let map = match fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        Some(Value::Object(map)) => map,
        _ => return ChatMeta::default(),
    };
    map.values()
        .find(|v| v.get("session_id").and_then(Value::as_str) == Some(session_id))
        .map(meta_from_entry)
        .unwrap_or_default()
}

fn meta_from_entry(entry: &Value) -> ChatMeta {
    let text = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let origin = entry.get("origin").unwrap_or(&Value::Null);
    let chat_type = text(origin, "chat_type").or_else(|| text(entry, "chat_type"));
    let platform = text(origin, "platform").or_else(|| text(entry, "platform"));
    // DM の display_name は相手の利用者名なので、題名には使わない。
    let title = if chat_type.as_deref() == Some("dm") {
        Some("DM".to_owned())
    } else {
        // チャンネルの chat_name は「サーバー名 / #チャンネル名」の形で、行にはチャンネル名だけを出す。
        text(origin, "chat_name")
            .or_else(|| text(entry, "display_name"))
            .and_then(|n| clean_title(n.rsplit(" / ").next().unwrap_or(&n)))
    };
    let link = match platform.as_deref() {
        Some("discord") => discord_link(
            chat_type.as_deref(),
            origin,
            entry
                .get("session_key")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        _ => None,
    };
    ChatMeta { title, link }
}

const DISCORD_CHANNELS: &str = "discord://-/channels/";

// Discord の ID（snowflake）は 10 進の数字だけでできている。URL に埋め込む前に確かめる。
fn is_snowflake(v: &str) -> bool {
    !v.is_empty() && v.len() <= 20 && v.bytes().all(|b| b.is_ascii_digit())
}

// DM の origin を持たない古い項目もあるので、DM の ID は会話の鍵
// （agent:main:discord:dm:<チャンネル ID>、gateway/session.py の build_session_key）からも取る。
fn discord_link(chat_type: Option<&str>, origin: &Value, session_key: &str) -> Option<String> {
    let id = |key: &str| {
        origin
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| is_snowflake(v))
    };
    if chat_type == Some("dm") {
        let chat = id("chat_id").or_else(|| {
            session_key
                .split(":dm:")
                .nth(1)
                .and_then(|rest| rest.split(':').next())
                .filter(|v| is_snowflake(v))
        })?;
        return Some(format!("{DISCORD_CHANNELS}@me/{chat}"));
    }
    let guild = id("guild_id")?;
    let chat = id("thread_id").or_else(|| id("chat_id"))?;
    Some(format!("{DISCORD_CHANNELS}{guild}/{chat}"))
}

/// 行を押したときに開いてよい URL か。状態ファイルの値をそのまま OS に渡すので、marimo が組み立てる
/// Discord のチャンネルの URL の形だけを受け付ける。
pub fn is_discord_link(url: &str) -> bool {
    let Some(rest) = url.strip_prefix(DISCORD_CHANNELS) else {
        return false;
    };
    let parts: Vec<&str> = rest.split('/').collect();
    matches!(parts.as_slice(), [scope, chat] if (*scope == "@me" || is_snowflake(scope)) && is_snowflake(chat))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Status, Transition, transition};

    fn input(event: &str, extra: Value) -> HermesInput {
        HermesInput {
            hook_event_name: event.to_owned(),
            session_id: "20260101_000000_abcd1234".to_owned(),
            cwd: Some("/home/u/.hermes".to_owned()),
            tool_name: None,
            tool_input: None,
            extra,
        }
    }

    fn hook_of(e: Event) -> HookInput {
        match e {
            Event::Hook(h) => *h,
            other => panic!("not a hook: {other:?}"),
        }
    }

    fn home() -> PathBuf {
        PathBuf::from("/home/u/.hermes")
    }

    fn run(events: &[HermesInput]) -> Option<SessionState> {
        let mut state: Option<SessionState> = None;
        for (i, e) in events.iter().enumerate() {
            if let Event::Hook(h) = translate(e, state.as_ref(), Some(&home()), 1000 + i as u64) {
                match transition(&h, state.as_ref(), 1000 + i as u64) {
                    Transition::Write(next) => state = Some(*next),
                    Transition::Delete => state = None,
                    Transition::Nothing => {}
                }
            }
        }
        state
    }

    #[test]
    fn events_map_to_claude_code_events() {
        let t = |event: &str, extra: Value| {
            hook_of(translate(&input(event, extra), None, Some(&home()), 0)).hook_event_name
        };
        assert_eq!(t("on_session_start", json!({})), "SessionStart");
        assert_eq!(t("on_session_reset", json!({})), "SessionStart");
        assert_eq!(t("pre_llm_call", json!({})), "UserPromptSubmit");
        assert_eq!(t("on_session_finalize", json!({})), "SessionEnd");
        assert_eq!(t("pre_tool_call", json!({})), "PreToolUse");
        assert_eq!(t("post_tool_call", json!({})), "PostToolUse");
        assert_eq!(t("pre_approval_request", json!({})), "PermissionRequest");
        assert_eq!(
            t("post_approval_response", json!({ "choice": "once" })),
            "PostToolUse"
        );
        assert_eq!(
            t("post_approval_response", json!({ "choice": "deny" })),
            "PermissionDenied"
        );
        assert_eq!(t("post_llm_call", json!({})), "Stop");
        assert_eq!(t("on_session_end", json!({ "completed": true })), "Stop");
        assert_eq!(
            t(
                "on_session_end",
                json!({ "interrupted": true, "completed": false })
            ),
            "Interrupt"
        );
        assert_eq!(
            t("on_session_end", json!({ "failed": true })),
            "StopFailure"
        );
        assert_eq!(
            t(
                "api_request_error",
                json!({ "retryable": false, "reason": "rate_limit" })
            ),
            "StopFailure"
        );
        for (event, extra) in [
            ("api_request_error", json!({ "retryable": true })),
            ("on_session_end", json!({})),
            ("on_stream_delta", json!({})),
            ("subagent_start", json!({})),
        ] {
            assert_eq!(
                translate(&input(event, extra), None, Some(&home()), 0),
                Event::Ignore,
                "{event}"
            );
        }
        let empty = HermesInput {
            session_id: String::new(),
            ..input("on_session_finalize", json!({}))
        };
        assert_eq!(translate(&empty, None, Some(&home()), 0), Event::Ignore);
    }

    #[test]
    fn a_turn_runs_from_prompt_to_done_and_ignores_the_background_review() {
        let turn = json!({ "turn_id": "s:t:1" });
        let tool = |event: &str, turn_id: &str| HermesInput {
            tool_name: Some("terminal".to_owned()),
            tool_input: Some(json!({ "command": "git status" })),
            ..input(event, json!({ "turn_id": turn_id }))
        };
        let working = run(&[
            input("on_session_start", json!({})),
            input("pre_llm_call", turn.clone()),
            tool("pre_tool_call", "s:t:1"),
        ])
        .unwrap();
        assert_eq!(working.status, Status::Working);
        assert_eq!(working.provider, Provider::Hermes);
        assert_eq!(working.turn_id.as_deref(), Some("s:t:1"));
        let activity = working.activity.unwrap();
        assert_eq!(activity.tool.as_deref(), Some("Bash"));
        assert_eq!(activity.summary, "git status");

        let done = run(&[
            input("pre_llm_call", turn.clone()),
            input(
                "post_llm_call",
                json!({ "turn_id": "s:t:1", "assistant_response": "終わったよ" }),
            ),
            input(
                "on_session_end",
                json!({ "turn_id": "s:t:1", "completed": true }),
            ),
            tool("pre_tool_call", "s:review:2"),
            tool("post_tool_call", "s:review:2"),
        ])
        .unwrap();
        assert_eq!(done.status, Status::Done);
        assert_eq!(done.activity.unwrap().summary, "終わったよ");
    }

    #[test]
    fn approval_and_clarify_wait_for_the_user() {
        let turn = || input("pre_llm_call", json!({ "turn_id": "t" }));
        let approval = run(&[
            turn(),
            input(
                "pre_approval_request",
                json!({ "turn_id": "t", "command": "rm -rf build" }),
            ),
        ])
        .unwrap();
        assert_eq!(approval.status, Status::Waiting);
        assert_eq!(approval.status_reason.as_deref(), Some("permission"));
        assert_eq!(approval.activity.unwrap().summary, "rm -rf build");

        let resolved = run(&[
            turn(),
            input("pre_approval_request", json!({ "turn_id": "t" })),
            input(
                "post_approval_response",
                json!({ "turn_id": "t", "choice": "timeout" }),
            ),
        ])
        .unwrap();
        assert_eq!(resolved.status, Status::Working);

        let clarify = run(&[
            turn(),
            HermesInput {
                tool_name: Some("clarify".to_owned()),
                tool_input: Some(json!({
                    "questions": [{ "question": "どちらにする？", "choices": ["main", "dev"] }]
                })),
                ..input("pre_tool_call", json!({ "turn_id": "t" }))
            },
        ])
        .unwrap();
        assert_eq!(clarify.status, Status::Waiting);
        assert_eq!(clarify.status_reason.as_deref(), Some("question"));
        let a = clarify.activity.unwrap();
        assert_eq!(a.summary, "質問: どちらにする？");
        assert_eq!(a.detail.as_deref(), Some("main / dev"));
    }

    #[test]
    fn failures_and_interrupts_end_the_turn() {
        let turn = || input("pre_llm_call", json!({ "turn_id": "t" }));
        let limited = run(&[
            turn(),
            input(
                "api_request_error",
                json!({
                    "turn_id": "t", "retryable": false, "reason": "rate_limit",
                    "error": { "type": "RateLimitError", "message": "slow down" }
                }),
            ),
        ])
        .unwrap();
        assert_eq!(limited.status, Status::Error);
        assert_eq!(limited.status_reason.as_deref(), Some("rate_limit"));

        let stopped = run(&[
            turn(),
            input(
                "on_session_end",
                json!({ "turn_id": "t", "interrupted": true }),
            ),
        ])
        .unwrap();
        assert_eq!(stopped.status, Status::Idle);

        assert!(
            run(&[turn(), input("on_session_finalize", json!({}))]).is_none(),
            "finalize removes the row"
        );
    }

    #[test]
    fn tools_become_their_claude_code_counterparts() {
        let t = |name: &str, args: Value| tool(name, Some(&args));
        assert_eq!(
            t("read_file", json!({ "path": "/a/b.txt", "offset": 3 })),
            ("Read".to_owned(), Some(json!({ "file_path": "/a/b.txt" })))
        );
        assert_eq!(
            t("web_extract", json!({ "urls": ["https://example.com/"] })).1,
            Some(json!({ "url": "https://example.com/" }))
        );
        assert_eq!(
            t(
                "web_extract",
                json!({ "urls": [{ "url": "https://example.org/" }] })
            )
            .1,
            Some(json!({ "url": "https://example.org/" }))
        );
        assert_eq!(
            t("mcp_context7_query_docs", json!({ "q": 1 })),
            (
                "mcp_context7_query_docs".to_owned(),
                Some(json!({ "q": 1 }))
            )
        );
    }

    #[test]
    fn context_comes_from_the_prompt_tokens_of_each_call() {
        let e = translate(
            &input(
                "post_api_request",
                json!({ "usage": { "prompt_tokens": 68000 }, "context_length": 272000 }),
            ),
            None,
            Some(&home()),
            5,
        );
        let Event::Context(c) = e else {
            panic!("{e:?}")
        };
        assert_eq!(c.used_percentage, Some(25.0));
        assert_eq!(c.total_input_tokens, Some(68000));
        assert_eq!(c.source.as_deref(), Some("hermes-api"));
        assert_eq!(
            translate(
                &input("post_api_request", json!({})),
                None,
                Some(&home()),
                5
            ),
            Event::Ignore
        );
    }

    #[test]
    fn the_gateway_folder_is_not_a_place() {
        assert_eq!(work_dir(Some("/home/u/.hermes/"), Some(&home())), None);
        assert_eq!(
            work_dir(Some("/home/u/src/app"), Some(&home())).as_deref(),
            Some("/home/u/src/app")
        );
        assert_eq!(work_dir(Some(""), Some(&home())), None);
        assert_eq!(
            work_dir(Some("/home/u/src/app"), None).as_deref(),
            Some("/home/u/src/app")
        );
    }

    #[test]
    fn chat_meta_names_the_channel_and_links_to_it() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sessions")).unwrap();
        fs::write(
            dir.path().join("sessions").join("sessions.json"),
            json!({
                "_README": "mirror",
                "agent:main:discord:dm:111111111111111111": {
                    "session_key": "agent:main:discord:dm:111111111111111111",
                    "session_id": "dm-session",
                    "display_name": "someone",
                    "platform": "discord",
                    "chat_type": "dm"
                },
                "agent:main:discord:group:222222222222222222:333333333333333333": {
                    "session_key": "agent:main:discord:group:222222222222222222:333333333333333333",
                    "session_id": "channel-session",
                    "display_name": "Server / #general",
                    "platform": "discord",
                    "chat_type": "group",
                    "origin": {
                        "platform": "discord", "chat_type": "group",
                        "chat_id": "222222222222222222", "chat_name": "Server / #general",
                        "guild_id": "444444444444444444", "thread_id": null
                    }
                },
                "agent:main:telegram:dm:5": {
                    "session_id": "telegram-session", "platform": "telegram", "chat_type": "group",
                    "display_name": "Family"
                }
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            chat_meta(dir.path(), "dm-session"),
            ChatMeta {
                title: Some("DM".to_owned()),
                link: Some("discord://-/channels/@me/111111111111111111".to_owned()),
            }
        );
        assert_eq!(
            chat_meta(dir.path(), "channel-session"),
            ChatMeta {
                title: Some("#general".to_owned()),
                link: Some("discord://-/channels/444444444444444444/222222222222222222".to_owned()),
            }
        );
        assert_eq!(
            chat_meta(dir.path(), "telegram-session"),
            ChatMeta {
                title: Some("Family".to_owned()),
                link: None,
            }
        );
        assert_eq!(chat_meta(dir.path(), "unknown"), ChatMeta::default());
        assert_eq!(
            chat_meta(&dir.path().join("missing"), "dm-session"),
            ChatMeta::default()
        );
    }

    #[test]
    fn only_discord_channel_links_may_be_opened() {
        assert!(is_discord_link("discord://-/channels/@me/123"));
        assert!(is_discord_link("discord://-/channels/1/2"));
        for bad in [
            "https://discord.com/channels/1/2",
            "discord://-/channels/1/2/3",
            "discord://-/channels/1/x",
            "discord://-/channels/@me/",
            "discord://-/channels/1/2 & calc",
            "file:///C:/Windows/System32/calc.exe",
        ] {
            assert!(!is_discord_link(bad), "{bad}");
        }
    }
}
