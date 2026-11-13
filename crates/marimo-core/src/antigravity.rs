//! Antigravity（agy の CLI と、Gemini のデスクトップアプリの中の Antigravity）のフックの入力を、Claude Code の
//! フックの入力の形に読み替える。利用制限は `agy -p /usage` の出力から読む。
//!
//! フックの項目名とイベントは、agy に同梱された agy-customizations の skill の docs/hooks.md に従う。
//! 入力の項目名は camelCase で、どのイベントも conversationId と transcriptPath を持つ。Gemini のデスクトップアプリが
//! 動かす agy は、CLI の 1.5.8 と違って hookEventName と cwd を入れてこないので、イベント名は登録したコマンドの
//! 引数で受け取り、作業フォルダは workspacePaths で補う。
//! 状態の決め方は Claude Code と同じ規則を使うので、ここではイベント名とツールの名前と引数を Claude Code の
//! ものに置き換えるだけにする。

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{
    ContextUsage, HookInput, Provider, RateLimits, RateWindow, SessionState, Status, clean_title,
};
use crate::time::parse_rfc3339_utc_ms;
use crate::transcript::{complete_lines, search_tail};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgyInput {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub workspace_paths: Vec<String>,
    /// `<agy のデータのフォルダ>/brain/<会話の ID>`。タイトルを探すのに使う。
    #[serde(default)]
    pub artifact_directory_path: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub tool_call: Option<ToolCall>,
    /// PreInvocation と PostInvocation にだけ入る。ターンの中でモデルを呼んだ回数で、ターンごとに 0 から数える。
    #[serde(default)]
    pub invocation_num: Option<u64>,
    #[serde(default)]
    pub termination_reason: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Stop にだけ入る。サブエージェントやバックグラウンドの作業が残っていれば false になる。
    #[serde(default)]
    pub fully_idle: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// 会話の transcript（transcript_full.jsonl）から読んだもの。この形式は文書に無く、agy 1.5.8 が書く形に従う。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    /// サブエージェントの会話なら、それを起動した親の会話の ID。
    pub parent: Option<String>,
    /// 最後の応答の文章。
    pub last_response: Option<String>,
    /// 最後のモデルの呼び出しに渡した入力のトークン数で、キャッシュから読んだ分を含む。
    pub context_tokens: Option<u64>,
}

// 利用者が始めた会話の先頭の行は USER_INPUT である。サブエージェントの会話は、親からの指示を載せた
// SYSTEM_MESSAGE の行で始まり、その本文が「sender=<親の会話の ID>」を含む。一つ目のプロンプトが長くても
// 読む量が増えないよう、先頭の一部だけを文字列として調べ、はっきり当てはまるときだけサブエージェントとみなす。
const HEAD_BYTES: u64 = 2048;

pub fn read_transcript(path: &Path) -> io::Result<Transcript> {
    let mut head = Vec::new();
    File::open(path)?.take(HEAD_BYTES).read_to_end(&mut head)?;
    let parent = parent_of(&String::from_utf8_lossy(&head));
    let (last_response, context_tokens) = search_tail(path, |buf, at_head| {
        let found = last_planner_response(&complete_lines(buf, at_head));
        let done = found.1.is_some();
        (found, done)
    })?;
    Ok(Transcript {
        parent,
        last_response,
        context_tokens,
    })
}

fn parent_of(head: &str) -> Option<String> {
    let first = head.lines().next()?;
    if !first.contains(r#""source":"SYSTEM""#) {
        return None;
    }
    let rest = &first[first.find("sender=")? + "sender=".len()..];
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
        .collect();
    (id.len() == 36).then_some(id)
}

// モデルの応答は type が PLANNER_RESPONSE の行で、入力のトークン数と、ツールを呼ばなかった応答では文章を持つ。
fn last_planner_response(lines: &[&[u8]]) -> (Option<String>, Option<u64>) {
    #[derive(Deserialize)]
    struct Step {
        #[serde(rename = "type")]
        kind: Option<String>,
        input_tokens: Option<u64>,
        #[serde(default)]
        cache_read_tokens: u64,
        content: Option<String>,
    }
    let mut text = None;
    let mut tokens = None;
    for line in lines.iter().rev() {
        let Ok(step) = serde_json::from_slice::<Step>(line) else {
            continue;
        };
        if step.kind.as_deref() != Some("PLANNER_RESPONSE") {
            continue;
        }
        if text.is_none() {
            text = step.content.filter(|c| !c.trim().is_empty());
        }
        if tokens.is_none() {
            tokens = step.input_tokens.map(|t| t + step.cache_read_tokens);
        }
        if text.is_some() && tokens.is_some() {
            break;
        }
    }
    (text, tokens)
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_ascii_lowercase().to_string() + chars.as_str()
    })
}

/// デスクトップアプリが会話に付けたタイトルを、`<agy のデータのフォルダ>/annotations/<会話の ID>.pbtxt` から読む。
/// このファイルは protobuf のテキスト形式で `title:"..."` を持つ。agy の CLI はこのファイルを作らない。
/// この形式は文書に無く、Gemini のデスクトップアプリの中の agy が書く形に従う。
pub fn title(artifact_dir: &Path, conversation_id: &str) -> Option<String> {
    let plain = !conversation_id.is_empty()
        && conversation_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-');
    if !plain {
        return None;
    }
    let data_dir = artifact_dir.parent()?.parent()?;
    let path = data_dir
        .join("annotations")
        .join(format!("{conversation_id}.pbtxt"));
    let mut text = Vec::new();
    File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut text)
        .ok()?;
    let text = String::from_utf8_lossy(&text);
    let start = text.find("title:\"")? + "title:\"".len();
    clean_title(&unescape_text_proto(&text[start..])?)
}

// protobuf のテキスト形式の文字列を、閉じる引用符まで読んで戻す。UTF-8 の文字は \343 のような 8 進数の
// バイトで書かれることがあるので、バイト列に戻してから文字列にする。
fn unescape_text_proto(s: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut bytes = s.bytes().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'"' => return String::from_utf8(out).ok(),
            b'\\' => match bytes.next()? {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                d @ b'0'..=b'7' => {
                    let mut v = u32::from(d - b'0');
                    for _ in 0..2 {
                        match bytes.peek() {
                            Some(&n @ b'0'..=b'7') => {
                                v = v * 8 + u32::from(n - b'0');
                                bytes.next();
                            }
                            _ => break,
                        }
                    }
                    out.push(u8::try_from(v).ok()?);
                }
                other => out.push(other),
            },
            _ => out.push(b),
        }
    }
    None
}

/// モデルの一覧にコンテキストの上限が無いので、トークン数だけを持つ。
pub fn context(transcript: &Transcript, now_ms: u64) -> Option<ContextUsage> {
    Some(ContextUsage {
        used_percentage: None,
        total_input_tokens: Some(transcript.context_tokens?),
        context_window_size: None,
        updated_at: now_ms,
        source: Some("antigravity-transcript".to_owned()),
    })
}

/// フックの入力を Claude Code のフックの入力に読み替える。状態を変えないイベントには None を返す。
/// `current` はこの会話の行、`transcript` はこの会話の transcript を読めたときの内容。
pub fn translate(
    input: &AgyInput,
    current: Option<&SessionState>,
    transcript: Option<&Transcript>,
) -> Option<HookInput> {
    let id = input.conversation_id.as_str();
    if id.is_empty() {
        return None;
    }
    // サブエージェントの会話は、最初のツールの呼び出しまで transcript ができない。読めないうちは親の会話か
    // サブエージェントかを決められないので、まだ行の無い会話のイベントは捨てる。利用者が始めた会話の
    // transcript は SessionStart の前にできている。
    if transcript.is_none() && current.is_none() {
        return None;
    }
    let hook = |session_id: &str, name: &str| HookInput {
        provider: Provider::Antigravity,
        session_id: session_id.to_owned(),
        hook_event_name: name.to_owned(),
        cwd: input
            .cwd
            .iter()
            .chain(input.workspace_paths.first())
            .find(|c| !c.is_empty())
            .cloned(),
        transcript_path: input.transcript_path.clone(),
        ..HookInput::default()
    };
    let with_tool = |mut h: HookInput| {
        let call = input.tool_call.clone().unwrap_or_default();
        let (name, args) = tool(&call.name, &call.args);
        h.tool_name = Some(name);
        h.tool_input = Some(args);
        h
    };
    // サブエージェントのイベントは、親の会話の行に、動いているサブエージェントとして書く。
    if let Some(parent) = transcript
        .and_then(|t| t.parent.as_deref())
        .filter(|p| *p != id)
    {
        let child = |name: &str| HookInput {
            agent_id: Some(id.to_owned()),
            cwd: None,
            transcript_path: None,
            ..hook(parent, name)
        };
        return match input.hook_event_name.as_str() {
            "PreToolUse" => Some(with_tool(child("PreToolUse"))),
            "PostToolUse" => Some(child("PostToolUse")),
            "Stop" => Some(child("SubagentStop")),
            _ => None,
        };
    }
    let own = current.map(|c| c.own_status.unwrap_or(c.status));
    let event = match input.hook_event_name.as_str() {
        // ターンの最初のモデルの呼び出しの前に、invocationNum が 0 で届く。サブエージェントの返事を受けて
        // 親の会話が動き直すときも 0 から数え直すが、そのときは前の Stop で作業中のままにしてあるので、
        // ターンの始まりの時刻を書き換えない。
        "PreInvocation" if input.invocation_num == Some(0) && own != Some(Status::Working) => {
            hook(id, "UserPromptSubmit")
        }
        "PreToolUse" => with_tool(hook(id, "PreToolUse")),
        "PostToolUse" => hook(id, "PostToolUse"),
        "Stop" => stop(input, transcript, hook(id, "Stop")),
        _ => return None,
    };
    Some(event)
}

// terminationReason の値は docs/hooks.md の Stop の節にある。
fn stop(input: &AgyInput, transcript: Option<&Transcript>, base: HookInput) -> HookInput {
    let reason = input.termination_reason.as_deref().unwrap_or("");
    let named = |name: &str| HookInput {
        hook_event_name: name.to_owned(),
        ..base.clone()
    };
    match reason {
        "USER_CANCELED" => named("Interrupt"),
        "ERROR" | "MAX_INVOCATIONS" | "MAX_FORCED_INVOCATIONS" | "MAX_TOKEN_BUDGET_EXCEEDED" => {
            HookInput {
                error: Some(reason.to_ascii_lowercase()),
                last_assistant_message: input.error.clone().filter(|e| !e.is_empty()),
                ..named("StopFailure")
            }
        }
        // サブエージェントの返事を待つ親の会話は、いったん Stop を出してから、返事を受けて動き直す。
        _ if input.fully_idle == Some(false) => named("PostToolUse"),
        _ => HookInput {
            last_assistant_message: transcript.and_then(|t| t.last_response.clone()),
            ..base
        },
    }
}

// agy のツールを、要約の作り方が同じ Claude Code のツールに置き換える。引数の名前は、agy 1.5.8 の
// transcript に残ったツールの呼び出しで確かめたものと、agy の実行ファイルの中の文字列から拾ったものである。
fn tool(name: &str, args: &Value) -> (String, Value) {
    // 引数の名前は、CLI では Query のように大文字で始まり、デスクトップアプリでは query のように小文字で始まる。
    let arg = |keys: &[&str]| {
        keys.iter()
            .flat_map(|k| [k.to_string(), lower_first(k)])
            .find_map(|k| args.get(&k).filter(|v| v.is_string()))
            .cloned()
            .unwrap_or_default()
    };
    let path = || arg(&["Path", "TargetFile", "AbsolutePath"]);
    let (claude, input) = match name {
        "run_command" => (
            "Bash",
            json!({ "command": arg(&["CommandLine"]), "description": arg(&["toolSummary"]) }),
        ),
        "read_file" | "view_file" => ("Read", json!({ "file_path": path() })),
        "write_file" | "write_to_file" | "create_file" => ("Write", json!({ "file_path": path() })),
        "edit_file" | "replace_file_content" | "multi_replace_file_content" => {
            ("Edit", json!({ "file_path": path() }))
        }
        "grep_search" => (
            "Grep",
            json!({ "pattern": arg(&["Query"]), "path": arg(&["SearchPath"]) }),
        ),
        "search_web" => ("WebSearch", json!({ "query": arg(&["Query"]) })),
        "read_url_content" => ("WebFetch", json!({ "url": arg(&["Url"]) })),
        "invoke_subagent" => {
            let prompt = arg(&["Prompt"]);
            let prompt = prompt.as_str().unwrap_or("");
            let first = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            (
                "Agent",
                json!({ "description": first, "subagent_type": arg(&["TypeName"]) }),
            )
        }
        // 利用者に質問するツール。呼ばれた時点で答えを待つ。
        "ask_question" => ("AskUserQuestion", json!({})),
        _ => return (name.to_owned(), args.clone()),
    };
    (claude.to_owned(), input)
}

// Gemini Models のグループの 5 時間と週のウィンドウ。Gemini 4 Argon の週の枠（gemini-argon-wtu-weekly）や
// Claude と GPT のモデルの枠は、Claude Code のモデル別の枠を出さないのと同じく出さない。
const FIVE_HOUR_BUCKET: &str = "gemini-5h";
const WEEKLY_BUCKET: &str = "gemini-weekly";

/// `agy -p /usage --output-format json` の出力から、Gemini のモデルの利用制限を読む。この出力の形は
/// 文書に無く、agy 1.5.8 が書く形に従う。バケットは残りの割合（remaining_fraction）を持つので、
/// ほかのツールと同じ使った割合に直す。値はその場で問い合わせたものなので、読んだ時刻を更新の時刻にする。
pub fn rate_limits_from_usage(output: &Value, now_ms: u64) -> Option<RateLimits> {
    if output.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        return None;
    }
    let buckets: Vec<&Value> = output
        .pointer("/command/data/groups")?
        .as_array()?
        .iter()
        .filter_map(|g| g.get("buckets").and_then(Value::as_array))
        .flatten()
        .collect();
    let window = |id: &str| {
        let b = buckets
            .iter()
            .find(|b| b.get("id").and_then(Value::as_str) == Some(id))?;
        let remaining = b.get("remaining_fraction")?.as_f64()?;
        Some(RateWindow {
            used_percentage: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
            resets_at: b
                .get("reset_time")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_utc_ms)
                .map(|ms| (ms / 1000) as i64),
        })
    };
    let limits = RateLimits {
        five_hour: window(FIVE_HOUR_BUCKET),
        seven_day: window(WEEKLY_BUCKET),
        updated_at: now_ms,
    };
    (limits.five_hour.is_some() || limits.seven_day.is_some()).then_some(limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PARENT: &str = "6d04017a-9441-42ac-b2ea-8341471932fd";

    #[test]
    fn translates_hooks_into_claude_events() {
        let input = |v: Value| serde_json::from_value::<AgyInput>(v).unwrap();
        let ev = |name: &str, extra: Value| {
            let mut v = json!({"conversationId": "c1", "hookEventName": name, "cwd": "/w"});
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            input(v)
        };
        let top = Transcript {
            last_response: Some("finished".to_owned()),
            ..Transcript::default()
        };
        let child = Transcript {
            parent: Some(PARENT.to_owned()),
            ..Transcript::default()
        };
        let mut working = SessionState::new("c1");
        working.status = Status::Working;
        let run = json!({"toolCall": {"name": "run_command",
            "args": {"CommandLine": "sleep 1", "toolSummary": "Wait"}}});
        let stop = |reason: &str, idle: bool| json!({"terminationReason": reason, "fullyIdle": idle, "error": ""});
        // 読み替え先のイベント名、agent_id、ツール名、最後の応答の文章。
        type Row<'a> = (&'a str, Option<&'a str>, Option<&'a str>, Option<&'a str>);
        type Case<'a> = (
            AgyInput,
            Option<&'a SessionState>,
            Option<&'a Transcript>,
            Option<Row<'a>>,
        );
        let cases: [Case; 11] = [
            (
                ev("PreInvocation", json!({"invocationNum": 0})),
                None,
                Some(&top),
                Some(("UserPromptSubmit", None, None, None)),
            ),
            (
                ev("PreInvocation", json!({"invocationNum": 0})),
                Some(&working),
                Some(&top),
                None,
            ),
            (
                ev("PreInvocation", json!({"invocationNum": 1})),
                None,
                Some(&top),
                None,
            ),
            (
                ev("PreToolUse", run.clone()),
                None,
                Some(&top),
                Some(("PreToolUse", None, Some("Bash"), None)),
            ),
            (
                ev("Stop", stop("NO_TOOL_CALL", true)),
                None,
                Some(&top),
                Some(("Stop", None, None, Some("finished"))),
            ),
            (
                ev("Stop", stop("NO_TOOL_CALL", false)),
                None,
                Some(&top),
                Some(("PostToolUse", None, None, None)),
            ),
            (
                ev("Stop", stop("USER_CANCELED", true)),
                None,
                Some(&top),
                Some(("Interrupt", None, None, None)),
            ),
            (
                ev("Stop", stop("ERROR", true)),
                None,
                Some(&top),
                Some(("StopFailure", None, None, None)),
            ),
            (ev("PostToolUse", json!({})), None, None, None),
            (
                ev("PreToolUse", run),
                None,
                Some(&child),
                Some(("PreToolUse", Some("c1"), Some("Bash"), None)),
            ),
            (
                ev("Stop", stop("NO_TOOL_CALL", true)),
                None,
                Some(&child),
                Some(("SubagentStop", Some("c1"), None, None)),
            ),
        ];
        for (raw, current, transcript, expected) in cases {
            let got = translate(&raw, current, transcript);
            let row = got.as_ref().map(|h| {
                (
                    h.hook_event_name.as_str(),
                    h.agent_id.as_deref(),
                    h.tool_name.as_deref(),
                    h.last_assistant_message.as_deref(),
                )
            });
            assert_eq!(
                row, expected,
                "{} {:?}",
                raw.hook_event_name, raw.termination_reason
            );
            if let Some(h) = got {
                let owner = if h.agent_id.is_some() { PARENT } else { "c1" };
                assert_eq!(
                    (h.provider, h.session_id.as_str()),
                    (Provider::Antigravity, owner)
                );
            }
        }
    }

    #[test]
    fn reads_the_desktop_title_with_octal_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let id = "beb340ee-37cf-4877-9e9d-9726d34dd0ca";
        let brain = dir.path().join("brain").join(id);
        std::fs::create_dir_all(&brain).unwrap();
        std::fs::create_dir_all(dir.path().join("annotations")).unwrap();
        let write = |text: &str| {
            std::fs::write(
                dir.path().join("annotations").join(format!("{id}.pbtxt")),
                text,
            )
            .unwrap()
        };
        write(r#"title:"\346\230\216\346\227\245 \"plan\"" last_user_view_time:{seconds:1}"#);
        assert_eq!(title(&brain, id).as_deref(), Some("明日 \"plan\""));
        write("last_user_view_time:{seconds:1}");
        assert_eq!(title(&brain, id), None);
        assert_eq!(title(&brain, "../x"), None);
    }

    #[test]
    fn reads_the_parent_and_the_last_response_from_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript_full.jsonl");
        let lines = [
            json!({"step_index": 0, "source": "SYSTEM", "type": "SYSTEM_MESSAGE",
                   "content": format!("[Message] timestamp=2026-11-13T13:48:35Z sender={PARENT} priority=HIGH")}),
            json!({"step_index": 1, "type": "PLANNER_RESPONSE", "input_tokens": 100,
                   "cache_read_tokens": 900, "content": "done"}),
            json!({"step_index": 2, "type": "PLANNER_RESPONSE", "input_tokens": 50,
                   "cache_read_tokens": 1000, "tool_calls": []}),
            json!({"step_index": 3, "type": "GENERIC", "content": "tool output"}),
        ];
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, &text).unwrap();
        assert_eq!(
            read_transcript(&path).unwrap(),
            Transcript {
                parent: Some(PARENT.to_owned()),
                last_response: Some("done".to_owned()),
                context_tokens: Some(1050),
            }
        );
        let user = json!({"step_index": 0, "source": "USER_EXPLICIT", "type": "USER_INPUT",
                          "content": format!("sender={PARENT}")});
        std::fs::write(&path, format!("{user}\n")).unwrap();
        assert_eq!(read_transcript(&path).unwrap().parent, None);
    }

    #[test]
    fn reads_only_the_gemini_buckets() {
        let bucket = |id: &str, remaining: f64| {
            json!({"id": id, "window": "weekly", "remaining_fraction": remaining,
                   "reset_time": "2026-11-20T12:55:29Z"})
        };
        let output = |status: &str, groups: Value| {
            json!({"conversation_id": "", "status": status,
                   "command": {"name": "usage", "data": {"groups": groups}}})
        };
        let groups = json!([
            {"name": "Gemini Models", "buckets": [bucket("gemini-weekly", 0.75), bucket("gemini-5h", 0.9)]},
            {"name": "Claude and GPT models", "buckets": [bucket("3p-5h", 0.0)]},
            {"name": "Gemini 4 Argon", "buckets": [bucket("gemini-argon-wtu-weekly", 0.0)]},
        ]);
        let reset = Some(1_795_179_329);
        let cases = [
            (
                output("SUCCESS", groups.clone()),
                Some(RateLimits {
                    five_hour: Some(RateWindow {
                        used_percentage: (1.0 - 0.9) * 100.0,
                        resets_at: reset,
                    }),
                    seven_day: Some(RateWindow {
                        used_percentage: 25.0,
                        resets_at: reset,
                    }),
                    updated_at: 7,
                }),
            ),
            (output("ERROR", groups), None),
            (
                output(
                    "SUCCESS",
                    json!([{"name": "Gemini 4 Argon",
                    "buckets": [bucket("gemini-argon-wtu-weekly", 0.5)]}]),
                ),
                None,
            ),
            (
                json!({"status": "SUCCESS", "response": "no quota summary"}),
                None,
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(rate_limits_from_usage(&input, 7), expected, "{input}");
        }
    }
}
