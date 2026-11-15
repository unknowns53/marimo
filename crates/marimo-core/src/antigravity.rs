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
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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
        // 作業中のモデルの呼び出しは、状態を変えずに最後のフックとして残す。応答を待つ間に止めたターンは、最後の
        // フックがこれか UserPromptSubmit のまま止まる。バックグラウンドの作業を待つ会話は、最後が Stop になる。
        "PreInvocation" if own == Some(Status::Working) => hook(id, "PreInvocation"),
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

// agy の step の状態（CortexStepStatus）の値。名前と値は agy の実行ファイルに埋め込まれた protobuf の定義から読んだ。
const STEP_RUNNING: i64 = 2;
const STEP_DONE: i64 = 3;
const STEP_CANCELED: i64 = 6;
const STEP_ERROR: i64 = 7;
const STEP_WAITING: i64 = 9;
const STEP_INTERRUPTED: i64 = 12;

// 承認されたツールが失敗したときも step は ERROR になり、そのあとすぐ PostToolUse が届く。断ったときは
// どのフックも届かないので、ERROR のまま PostToolUse を待つ時間を置いてから断ったとみなす。
const DECLINE_GRACE_MS: u64 = 2_000;

// 承認したあとは、ツールが終わるまで PostToolUse が届かない。コマンドは 5 秒で非同期に切り替わるまで届かないので、
// RUNNING に戻ってこの時間がたったら承認されたとみなす。断ったときも RUNNING を通るが、数ミリ秒で終わる。
const APPROVE_GRACE_MS: u64 = 1_000;

// モデルの応答を待っている間に利用者が止めると、agy は Stop も PostInvocation も出さず、DB にも止めた step を
// 残さない。応答を待つ間も step は増えないので、止めた会話と考えている会話は、フックが来ない時間の長さでしか
// 見分けられない。agy 1.5.9 の会話の DB では、step が増えないまま応答を待った時間は最長 46 秒、考え始めてから
// 応答の step が終わるまでは最長 89 秒だった。DB を読めないときは後者が次のフックまでの時間になる。
const SILENT_TURN_MS: u64 = 3 * 60 * 1000;

/// 会話の DB の最後の step。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub status: i64,
    pub tool: Option<ToolCall>,
    /// 今の状態になった時刻。
    pub since_ms: Option<u64>,
    /// 今のターン（最後の利用者の入力から後）に、PENDING（1）、RUNNING（2）、GENERATING（8）、WAITING（9）の
    /// どれかのままの step がある。バックグラウンドで動くコマンドの step は、最後の step より前で RUNNING のまま残る。
    pub turn_busy: bool,
}

/// 会話の DB（`<agy のデータのフォルダ>/conversations/<会話の ID>.db`）の場所を、transcript の場所
/// （`<agy のデータのフォルダ>/brain/<会話の ID>/.system_generated/logs/transcript_full.jsonl`）から求める。
/// サブエージェントの会話も同じフォルダに DB を持つので、その会話の ID を渡す。
pub fn conversation_db(transcript_path: &Path, conversation_id: &str) -> Option<PathBuf> {
    let plain = !conversation_id.is_empty()
        && conversation_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-');
    let data_dir = transcript_path.ancestors().nth(5).filter(|_| plain)?;
    Some(
        data_dir
            .join("conversations")
            .join(format!("{conversation_id}.db")),
    )
}

/// steps の表の status の列と metadata の列から、step を読む。metadata は protobuf で、ツールの呼び出し
/// （フィールド 4。その中のフィールド 2 が名前、3 が引数の JSON）と、状態の移り変わり（繰り返しのフィールド 26。
/// その中のフィールド 2 が時刻）を持つ。この形は文書に無く、agy 1.5.9 が書く形に従う。
pub fn step(status: i64, metadata: &[u8]) -> Step {
    let fields = proto_fields(metadata);
    let tool = fields.iter().rev().find_map(|&(n, v)| match (n, v) {
        (4, Proto::Bytes(call)) => {
            let call = proto_fields(call);
            let text = |want| {
                call.iter().find_map(|&(n, v)| match v {
                    Proto::Bytes(b) if n == want => std::str::from_utf8(b).ok(),
                    _ => None,
                })
            };
            Some(ToolCall {
                name: text(2)?.to_owned(),
                args: text(3)
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or(Value::Null),
            })
        }
        _ => None,
    });
    let since_ms = fields.iter().rev().find_map(|&(n, v)| match (n, v) {
        (26, Proto::Bytes(history)) => {
            let last = proto_fields(history)
                .into_iter()
                .rev()
                .find_map(|(n, v)| match v {
                    Proto::Bytes(change) if n == 1 => Some(change),
                    _ => None,
                })?;
            proto_fields(last).into_iter().find_map(|(n, v)| match v {
                Proto::Bytes(ts) if n == 2 => timestamp_ms(ts),
                _ => None,
            })
        }
        _ => None,
    });
    Step {
        status,
        tool,
        since_ms,
        turn_busy: false,
    }
}

#[derive(Clone, Copy)]
enum Proto<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

// protobuf の一段だけを読む。読めない形に当たったら、そこまでに読めたフィールドを返す。
fn proto_fields(mut b: &[u8]) -> Vec<(u64, Proto<'_>)> {
    fn varint(b: &mut &[u8]) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let (&c, rest) = b.split_first()?;
            *b = rest;
            v |= u64::from(c & 0x7f) << shift;
            if c < 0x80 {
                return Some(v);
            }
        }
        None
    }
    let mut out = Vec::new();
    while !b.is_empty() {
        let Some(key) = varint(&mut b) else { break };
        let value = match key & 7 {
            0 => match varint(&mut b) {
                Some(v) => Proto::Varint(v),
                None => break,
            },
            1 | 5 => {
                let n = if key & 7 == 1 { 8 } else { 4 };
                let Some(rest) = b.get(n..) else { break };
                b = rest;
                continue;
            }
            2 => {
                let Some(len) = varint(&mut b).and_then(|l| usize::try_from(l).ok()) else {
                    break;
                };
                let Some((bytes, rest)) = b.split_at_checked(len) else {
                    break;
                };
                b = rest;
                Proto::Bytes(bytes)
            }
            _ => break,
        };
        out.push((key >> 3, value));
    }
    out
}

// google.protobuf.Timestamp（1 が秒、2 がナノ秒）をミリ秒にする。
fn timestamp_ms(b: &[u8]) -> Option<u64> {
    let fields = proto_fields(b);
    let get = |want| {
        fields.iter().find_map(|&(n, v)| match v {
            Proto::Varint(x) if n == want => Some(x),
            _ => None,
        })
    };
    Some(get(1)? * 1000 + get(2).unwrap_or(0) / 1_000_000)
}

/// agy は承認を求めている間に呼ぶフックを持たず、利用者が承認を断ったときや、モデルの応答を待っている間に止めた
/// ときも、PostToolUse や Stop を出さずにターンを止める。そこで、会話の DB の最後の step の状態と、最後の
/// フックからの時間を、Claude Code のフックの入力に読み替える。`agent` が Some なら、`step` はその ID の
/// サブエージェントの会話のものである。`step` が None なのは DB を読めなかったときで、agy がその会話の DB を
/// 開いていないときもこれにあたる。状態を変えないときは None を返す。
pub fn step_event(
    session: &SessionState,
    agent: Option<&str>,
    step: Option<&Step>,
    now_ms: u64,
) -> Option<HookInput> {
    let base = |name: &str| {
        let mut h = HookInput {
            provider: Provider::Antigravity,
            session_id: session.session_id.clone(),
            hook_event_name: name.to_owned(),
            agent_id: agent.map(str::to_owned),
            ..HookInput::default()
        };
        if let Some(call) = step.and_then(|s| s.tool.as_ref()) {
            let (name, args) = tool(&call.name, &call.args);
            h.tool_name = Some(name);
            h.tool_input = Some(args);
        }
        h
    };
    let own = session.own_status.unwrap_or(session.status);
    // 数えるのは、最後のフックがモデルの呼び出しの始まりの行だけにする。PreToolUse の後はツールが動いており、
    // fullyIdle が false の Stop（PostToolUse に読み替えたもの）の後はバックグラウンドの作業を待っている。
    // サブエージェントが動いている間は、その会話が応答を待っていても親の行のフックは来ない。
    let silent = agent.is_none()
        && own == Status::Working
        && session.agents.is_empty()
        && matches!(
            session.last_event.as_deref(),
            Some("UserPromptSubmit" | "PreInvocation")
        )
        && step.is_none_or(|s| !s.turn_busy)
        && session.updated_at.saturating_add(SILENT_TURN_MS) <= now_ms;
    if silent {
        return Some(base("Interrupt"));
    }
    let step = step?;
    let ended = matches!(step.status, STEP_ERROR | STEP_CANCELED | STEP_INTERRUPTED);
    if let Some(agent) = agent {
        let pending = session.agents.get(agent)?.pending.is_some();
        return match step.status {
            STEP_WAITING if !pending => Some(base("PermissionRequest")),
            _ if pending && (ended || step.status == STEP_DONE) => Some(base("PostToolUse")),
            _ => None,
        };
    }
    let lasted = |ms: u64| {
        step.since_ms
            .is_some_and(|t| t.saturating_add(ms) <= now_ms)
    };
    match step.status {
        // ツールの名前が読めなければ、PreToolUse で書いた作業の要約をそのまま使う。
        STEP_WAITING if own == Status::Working => Some(match step.tool {
            Some(_) => base("PermissionRequest"),
            None => HookInput {
                notification_type: Some("permission_prompt".to_owned()),
                ..base("Notification")
            },
        }),
        _ if own == Status::Waiting && ended && lasted(DECLINE_GRACE_MS) => Some(base("Interrupt")),
        // 承認待ちにしたのがこの読み替えのときだけ戻す。質問のツールは、PreToolUse で承認待ちにしたあと、
        // WAITING になる前に RUNNING のまま数秒たつことがある。
        STEP_RUNNING
            if own == Status::Waiting
                && matches!(
                    session.last_event.as_deref(),
                    Some("PermissionRequest" | "Notification")
                )
                && lasted(APPROVE_GRACE_MS) =>
        {
            Some(base("PostToolUse"))
        }
        _ => None,
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
        let cases: [Case; 12] = [
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
                Some(("PreInvocation", None, None, None)),
            ),
            (
                ev("PreInvocation", json!({"invocationNum": 1})),
                Some(&working),
                Some(&top),
                Some(("PreInvocation", None, None, None)),
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

    #[test]
    fn reads_the_last_step_from_the_conversation_db_row() {
        fn varint(mut x: u64, out: &mut Vec<u8>) {
            while x >= 0x80 {
                out.push(x as u8 | 0x80);
                x >>= 7;
            }
            out.push(x as u8);
        }
        let num = |n: u64, x: u64| {
            let mut v = Vec::new();
            varint(n << 3, &mut v);
            varint(x, &mut v);
            v
        };
        let field = |n: u64, bytes: &[u8]| {
            let mut v = Vec::new();
            varint(n << 3 | 2, &mut v);
            varint(bytes.len() as u64, &mut v);
            v.extend_from_slice(bytes);
            v
        };
        let ts = |secs, nanos| [num(1, secs), num(2, nanos)].concat();
        let change =
            |status, secs, nanos| field(1, &[num(1, status), field(2, &ts(secs, nanos))].concat());
        let args = r#"{"CommandLine":"echo hi","toolSummary":"Say hi"}"#;
        let call = [
            field(1, b"call_1"),
            field(2, b"run_command"),
            field(3, args.as_bytes()),
        ]
        .concat();
        let metadata = [
            field(1, &ts(100, 0)),
            num(3, 2),
            field(4, &call),
            field(26, &[change(1, 100, 0), change(9, 102, 5_000_000)].concat()),
        ]
        .concat();
        let tool = Some(ToolCall {
            name: "run_command".to_owned(),
            args: serde_json::from_str(args).unwrap(),
        });
        assert_eq!(
            step(9, &metadata),
            Step {
                status: 9,
                tool: tool.clone(),
                since_ms: Some(102_005),
                turn_busy: false,
            }
        );
        let cut = step(9, &metadata[..metadata.len() - 3]);
        assert_eq!((cut.tool, cut.since_ms), (tool, None));

        let id = "a4476239-3e70-42fc-accd-2a5220efcdd2";
        let transcript = Path::new("/d/brain")
            .join(id)
            .join(".system_generated/logs/transcript_full.jsonl");
        assert_eq!(
            conversation_db(&transcript, id),
            Some(Path::new("/d/conversations").join(format!("{id}.db")))
        );
        assert_eq!(conversation_db(&transcript, "../x"), None);
    }

    #[test]
    fn reads_steps_hidden_from_hooks_into_claude_events() {
        const NOW: u64 = 1_000_000;
        let with = |own: Status, pending: Option<bool>| {
            let mut s = SessionState::new("c1");
            s.status = own;
            s.own_status = Some(own);
            if let Some(p) = pending {
                s.agents.insert(
                    "c2".to_owned(),
                    crate::state::AgentRun {
                        agent_type: None,
                        started_at: 0,
                        last_seen: 0,
                        pending: p.then(|| crate::activity::Activity::message("x").unwrap()),
                    },
                );
            }
            s
        };
        let working = with(Status::Working, Some(false));
        let waiting = with(Status::Waiting, Some(true));
        let prompted = SessionState {
            last_event: Some("PermissionRequest".to_owned()),
            ..waiting.clone()
        };
        let call = ToolCall {
            name: "run_command".to_owned(),
            args: json!({"CommandLine": "echo hi"}),
        };
        let at = |status, tool: Option<&ToolCall>, since| {
            Some(Step {
                status,
                tool: tool.cloned(),
                since_ms: Some(since),
                turn_busy: false,
            })
        };
        // 最後のフックから SILENT_TURN_MS たった、サブエージェントの無い作業中の行。
        let silent = |last: &str, quiet: u64| SessionState {
            last_event: Some(last.to_owned()),
            updated_at: NOW - quiet,
            ..with(Status::Working, None)
        };
        let thinking = silent("UserPromptSubmit", SILENT_TURN_MS);
        let invoking = silent("PreInvocation", SILENT_TURN_MS);
        let backgrounded = silent("PostToolUse", SILENT_TURN_MS);
        let recent = silent("UserPromptSubmit", SILENT_TURN_MS - 1);
        let tooling = silent("PreToolUse", SILENT_TURN_MS);
        let busy = Some(Step {
            turn_busy: true,
            ..at(3, None, NOW).unwrap()
        });
        // (行, サブエージェント, 最後の step, 読み替え先のイベント名とツール名)
        type Case<'a> = (
            &'a SessionState,
            Option<&'a str>,
            Option<Step>,
            Option<(&'a str, Option<&'a str>)>,
        );
        let cases: [Case; 20] = [
            (
                &working,
                None,
                at(9, Some(&call), NOW),
                Some(("PermissionRequest", Some("Bash"))),
            ),
            (
                &working,
                None,
                at(9, None, NOW),
                Some(("Notification", None)),
            ),
            (
                &waiting,
                None,
                at(7, None, NOW - 2_000),
                Some(("Interrupt", None)),
            ),
            (&waiting, None, at(7, None, NOW - 500), None),
            (&working, None, at(7, None, NOW - 9_000), None),
            (&waiting, None, at(2, None, NOW - 9_000), None),
            (
                &prompted,
                None,
                at(2, None, NOW - 1_000),
                Some(("PostToolUse", None)),
            ),
            (&prompted, None, at(2, None, NOW - 10), None),
            (
                &working,
                Some("c2"),
                at(9, Some(&call), NOW),
                Some(("PermissionRequest", Some("Bash"))),
            ),
            (
                &waiting,
                Some("c2"),
                at(3, None, NOW),
                Some(("PostToolUse", None)),
            ),
            (&waiting, Some("c2"), at(9, Some(&call), NOW), None),
            (&working, Some("zz"), at(9, Some(&call), NOW), None),
            (&working, Some("c2"), None, None),
            (&thinking, None, at(3, None, NOW), Some(("Interrupt", None))),
            (&thinking, None, None, Some(("Interrupt", None))),
            (&invoking, None, None, Some(("Interrupt", None))),
            (&backgrounded, None, None, None),
            (&recent, None, None, None),
            (&tooling, None, None, None),
            (&thinking, None, busy, None),
        ];
        for (session, agent, step, expected) in cases {
            let got = step_event(session, agent, step.as_ref(), NOW);
            let row = got
                .as_ref()
                .map(|h| (h.hook_event_name.as_str(), h.tool_name.as_deref()));
            assert_eq!(row, expected, "{agent:?} {step:?}");
            if let Some(h) = got {
                assert_eq!(
                    (h.session_id.as_str(), h.agent_id.as_deref()),
                    ("c1", agent)
                );
            }
        }
    }
}
