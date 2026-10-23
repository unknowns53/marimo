use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::state::clean_title;
use crate::time::parse_rfc3339_utc_ms;

// 会話ログは数十 MB になりうるので、末尾から少しずつ広げて探す。最後の assistant の
// 応答はふつう末尾近くにあり、最初の範囲で見つかる。
const FIRST_WINDOW: u64 = 512 * 1024;
const MAX_WINDOW: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptUsage {
    /// statusLine の used_percentage と同じ式で数えた、コンテキストを占めるトークン数。
    pub context_tokens: u64,
    pub entrypoint: Option<String>,
}

/// 会話ログの末尾から読み取れたもの。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptTail {
    pub usage: Option<TranscriptUsage>,
    /// 読んだ範囲で最後の custom-title の行のタイトル。範囲を広げてまでは探さない。
    pub title: Option<String>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    entrypoint: Option<String>,
    message: Option<Message>,
}

// content などの大きな値は、構造体に無い項目として serde が読み飛ばす。
#[derive(Deserialize)]
struct Message {
    usage: Option<Usage>,
}

#[derive(Deserialize, Clone, Copy, Default)]
struct Counts {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    iterations: Option<Vec<Counts>>,
}

impl Usage {
    // 使用中のトークン数は https://code.claude.com/docs/en/statusline の Context window fields の
    // 節にある式（入力、キャッシュ作成、キャッシュ読み出しの合計で、出力は含めない）に合わせる。
    fn context_tokens(&self) -> u64 {
        let c = self
            .iterations
            .as_ref()
            .and_then(|it| it.last().copied())
            .unwrap_or(Counts {
                input_tokens: self.input_tokens,
                cache_creation_input_tokens: self.cache_creation_input_tokens,
                cache_read_input_tokens: self.cache_read_input_tokens,
            });
        c.input_tokens + c.cache_creation_input_tokens + c.cache_read_input_tokens
    }
}

pub fn read_tail(path: &Path) -> io::Result<TranscriptTail> {
    search_tail(path, |buf, head| {
        let tail = scan(buf, head);
        let found = tail.usage.is_some();
        (tail, found)
    })
}

/// 親の会話の最後の行が何だったか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastTurn {
    /// 会話がバックグラウンドへ移り、このログの続きが別のセッションで書かれている。
    Moved,
    /// 利用者が応答を止めたか、承認や質問を断った。`at_ms` はその行の時刻。
    Interrupted { at_ms: u64 },
    /// それ以外か、会話の行が見つからない。
    Other,
}

// 応答を止めたときと、承認や質問を断ったときに Claude Code が書く user の行の文章。
const INTERRUPT_MARKERS: [&str; 2] = [
    "[Request interrupted by user]",
    "[Request interrupted by user for tool use]",
];

/// 末尾から見て最初の親の会話の行（user、assistant、continued-in の行）で、会話がどう終わっているかを返す。
/// 会話をバックグラウンドへ移すと、元のログの末尾に continued-in の行が書かれる。その後で元のセッションが
/// 応答を続けることもあるので、それより後に user か assistant の行があれば移っていないとみなす。利用者が
/// 応答を止めたり承認や質問を断ったりすると、Stop が発火しないまま中断の文章だけの user の行が書かれる。
/// これらの行は Claude Code のドキュメントに載っていない形式なので、形が変わったら見分けられずに
/// 24 時間で消えるのを待つだけで済むよう、読めない行は捨てる。
pub fn last_turn(path: &Path) -> io::Result<LastTurn> {
    #[derive(Deserialize)]
    struct KindLine {
        #[serde(rename = "type")]
        kind: Option<String>,
        #[serde(rename = "isSidechain", default)]
        is_sidechain: bool,
    }
    search_tail(path, |buf, head| {
        let last = complete_lines(buf, head).into_iter().rev().find_map(|l| {
            let line = serde_json::from_slice::<KindLine>(l).ok()?;
            let kind = line.kind.filter(|_| !line.is_sidechain)?;
            match kind.as_str() {
                "continued-in" => Some(LastTurn::Moved),
                "user" => Some(
                    interrupted_at(l)
                        .map_or(LastTurn::Other, |at_ms| LastTurn::Interrupted { at_ms }),
                ),
                "assistant" => Some(LastTurn::Other),
                _ => None,
            }
        });
        (last.unwrap_or(LastTurn::Other), last.is_some())
    })
}

// 文章は content に文字列で入る形と、text の要素の配列で入る形の両方を受け付ける。
fn interrupted_at(bytes: &[u8]) -> Option<u64> {
    #[derive(Deserialize)]
    struct UserLine {
        timestamp: Option<String>,
        message: Option<UserMessage>,
    }
    #[derive(Deserialize)]
    struct UserMessage {
        content: Option<Value>,
    }
    if !contains(bytes, b"[Request interrupted by user") {
        return None;
    }
    let line: UserLine = serde_json::from_slice(bytes).ok()?;
    let is_marker = |v: &Value| v.as_str().is_some_and(|t| INTERRUPT_MARKERS.contains(&t));
    let marked = match line.message?.content? {
        Value::Array(items) => items
            .iter()
            .any(|i| i["type"] == "text" && is_marker(&i["text"])),
        text => is_marker(&text),
    };
    if !marked {
        return None;
    }
    parse_rfc3339_utc_ms(line.timestamp.as_deref()?)
}

/// JSON Lines のファイルを末尾から読み、`scan` が見つけたと答えるか、ファイルの先頭か読む量の
/// 上限に達するまで範囲を広げる。`scan` には読んだバイト列と、それがファイルの先頭から始まるかを渡す。
pub(crate) fn search_tail<T>(
    path: &Path,
    mut scan: impl FnMut(&[u8], bool) -> (T, bool),
) -> io::Result<T> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut window = FIRST_WINDOW.min(len);
    loop {
        let start = len - window;
        file.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::with_capacity(window as usize);
        (&mut file).take(window).read_to_end(&mut buf)?;
        let (result, found) = scan(&buf, start == 0);
        if found || window >= len || window >= MAX_WINDOW {
            return Ok(result);
        }
        window = (window * 8).min(MAX_WINDOW).min(len);
    }
}

// buf の先頭がファイルの途中なら、最初の行は切れているので読まない。末尾の改行で
// 終わっていない行も書き込みの途中なので読まない。
pub(crate) fn complete_lines(buf: &[u8], starts_at_file_head: bool) -> Vec<&[u8]> {
    let Some(complete_end) = buf.iter().rposition(|&b| b == b'\n') else {
        return Vec::new();
    };
    let mut lines: Vec<&[u8]> = buf[..complete_end].split(|&b| b == b'\n').collect();
    if !starts_at_file_head && !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

fn scan(buf: &[u8], starts_at_file_head: bool) -> TranscriptTail {
    let lines = complete_lines(buf, starts_at_file_head);
    let usage = lines
        .iter()
        .rev()
        .filter(|l| contains(l, b"\"usage\"") && contains(l, b"\"assistant\""))
        .find_map(|l| parse_line(l));
    let title = lines
        .iter()
        .rev()
        .filter(|l| contains(l, b"\"custom-title\""))
        .find_map(|l| parse_title(l));
    TranscriptTail { usage, title }
}

// custom-title の行は Claude Code のドキュメントに載っていない形式で、デスクトップアプリの会話ログに
// 同じタイトルが何度も書き足される。形が変わったらタイトルが出なくなるだけで済むよう、読めない行は捨てる。
fn parse_title(bytes: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct TitleLine {
        #[serde(rename = "type")]
        kind: Option<String>,
        #[serde(rename = "customTitle")]
        custom_title: Option<String>,
    }
    let line: TitleLine = serde_json::from_slice(bytes).ok()?;
    if line.kind.as_deref() != Some("custom-title") {
        return None;
    }
    clean_title(line.custom_title.as_deref()?)
}

fn parse_line(bytes: &[u8]) -> Option<TranscriptUsage> {
    let line: Line = serde_json::from_slice(bytes).ok()?;
    if line.kind.as_deref() != Some("assistant") || line.is_sidechain {
        return None;
    }
    let usage = line.message?.usage?;
    Some(TranscriptUsage {
        context_tokens: usage.context_tokens(),
        entrypoint: line.entrypoint.filter(|e| !e.is_empty()),
    })
}

pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn last_usage(path: &Path) -> io::Result<Option<TranscriptUsage>> {
        read_tail(path).map(|t| t.usage)
    }

    fn title(t: &str) -> String {
        json!({"type": "custom-title", "customTitle": t, "sessionId": "s1"}).to_string()
    }

    fn assistant(usage: serde_json::Value, sidechain: bool) -> String {
        json!({
            "type": "assistant", "isSidechain": sidechain, "entrypoint": "cli",
            "sessionId": "s1", "uuid": "u",
            "message": {"id": "m", "role": "assistant", "model": "claude-sonnet-5",
                        "content": [{"type": "text", "text": "hi"}], "usage": usage}
        })
        .to_string()
    }

    fn write(lines: &[String], trailing: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        f.write_all(trailing.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn usage(i: u64, cc: u64, cr: u64) -> serde_json::Value {
        json!({"input_tokens": i, "cache_creation_input_tokens": cc, "cache_read_input_tokens": cr, "output_tokens": 999})
    }

    #[test]
    fn takes_the_last_main_thread_assistant_usage() {
        let lines = vec![
            assistant(usage(1, 2, 3), false),
            assistant(usage(10, 20, 30), false),
            json!({"type": "user", "message": {"role": "user", "content": "x"}}).to_string(),
            assistant(usage(500, 500, 500), true),
            json!({"type": "assistant", "message": {"content": []}}).to_string(),
        ];
        let f = write(
            &lines,
            "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":7",
        );
        let got = last_usage(f.path()).unwrap().unwrap();
        assert_eq!(got.context_tokens, 60);
        assert_eq!(got.entrypoint.as_deref(), Some("cli"));

        // iterations があれば、その最後の要素で数える。
        let mut u = usage(2, 0, 27038);
        u["iterations"] = json!([
            {"input_tokens": 1, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000, "type": "message"},
            {"input_tokens": 2, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 26000, "type": "message"}
        ]);
        let f = write(&[assistant(u, false)], "");
        assert_eq!(last_usage(f.path()).unwrap().unwrap().context_tokens, 26002);
    }

    #[test]
    fn finds_usage_behind_megabytes_of_long_lines() {
        let huge =
            json!({"type": "user", "message": {"content": "y".repeat(1_500_000)}}).to_string();
        let lines = vec![assistant(usage(4, 5, 6), false), huge.clone(), huge];
        let f = write(&lines, "");
        assert!(f.as_file().metadata().unwrap().len() > 3_000_000);
        assert_eq!(last_usage(f.path()).unwrap().unwrap().context_tokens, 15);
    }

    #[test]
    fn gives_up_beyond_the_limit_or_on_garbage() {
        let far = vec![
            assistant(usage(1, 1, 1), false),
            json!({"type": "user", "message": {"content": "z".repeat(5_000_000)}}).to_string(),
        ];
        assert_eq!(last_usage(write(&far, "").path()).unwrap(), None);
        assert_eq!(
            last_usage(write(&["not json \"usage\" \"assistant\"".into()], "").path()).unwrap(),
            None
        );
        assert_eq!(last_usage(write(&[], "").path()).unwrap(), None);
        assert!(last_usage(Path::new("/nonexistent/transcript.jsonl")).is_err());
    }

    #[test]
    fn line_cut_at_the_window_head_is_skipped() {
        // 最初の読み取り範囲の先頭で切れる位置に、usage を含む長い行を置く。
        let cut = assistant(usage(9, 9, 9), false);
        let filler = json!({"type": "user", "message": {"content": "f".repeat(FIRST_WINDOW as usize - 200)}}).to_string();
        let lines = vec![assistant(usage(1, 2, 3), false), cut, filler];
        let f = write(&lines, "");
        assert_eq!(last_usage(f.path()).unwrap().unwrap().context_tokens, 27);
    }

    #[test]
    fn last_turn_follows_the_last_main_thread_line() {
        let user = json!({"type": "user", "message": {"role": "user", "content": "x"}}).to_string();
        let moved =
            json!({"type": "continued-in", "sessionId": "s1", "continuedInSessionId": "s2"})
                .to_string();
        let queued = json!({"type": "queue-operation", "operation": "enqueue"}).to_string();
        let at = "2026-10-16T12:13:57.473Z";
        let at_ms = parse_rfc3339_utc_ms(at).unwrap();
        let rejected = json!({"type": "user", "isSidechain": false, "timestamp": at, "message": {
            "role": "user",
            "content": [{"type": "text", "text": "[Request interrupted by user for tool use]"}]
        }})
        .to_string();
        let stopped = json!({"type": "user", "timestamp": at, "message": {
            "role": "user", "content": "[Request interrupted by user]"
        }})
        .to_string();
        let quoted = json!({"type": "user", "timestamp": at, "message": {
            "role": "user", "content": "なぜ [Request interrupted by user] と出るの"
        }})
        .to_string();
        let untimed =
            json!({"type": "user", "message": {"content": "[Request interrupted by user]"}})
                .to_string();
        let side = json!({"type": "assistant", "isSidechain": true, "message": {"content": []}})
            .to_string();
        let interrupted = LastTurn::Interrupted { at_ms };
        let cases = [
            (vec![user.clone(), moved.clone()], LastTurn::Moved),
            (
                vec![user.clone(), moved.clone(), queued.clone()],
                LastTurn::Moved,
            ),
            (vec![moved.clone(), queued, user.clone()], LastTurn::Other),
            (
                vec![moved, assistant(usage(1, 2, 3), false)],
                LastTurn::Other,
            ),
            (vec![user.clone(), rejected.clone()], interrupted),
            (vec![stopped.clone(), side], interrupted),
            (vec![rejected, user.clone()], LastTurn::Other),
            (
                vec![stopped, assistant(usage(1, 2, 3), false)],
                LastTurn::Other,
            ),
            (vec![quoted], LastTurn::Other),
            (vec![untimed], LastTurn::Other),
            (vec![user], LastTurn::Other),
            (vec![], LastTurn::Other),
        ];
        for (lines, want) in cases {
            let f = write(&lines, "");
            assert_eq!(last_turn(f.path()).unwrap(), want, "{lines:?}");
        }
    }

    #[test]
    fn takes_the_last_title_in_the_window() {
        let lines = vec![
            title("最初のタイトル"),
            json!({"type": "agent-name", "agentName": "x"}).to_string(),
            assistant(usage(1, 2, 3), false),
            title(&format!("  {}  ", "長".repeat(300))),
            title("   "),
            json!({"type": "custom-title", "customTitle": 5}).to_string(),
            json!({"type": "user", "message": {"content": "\"custom-title\""}}).to_string(),
        ];
        let got = read_tail(write(&lines, "").path()).unwrap();
        assert_eq!(got.usage.unwrap().context_tokens, 6);
        let t = got.title.unwrap();
        assert_eq!(t.chars().count(), crate::state::TITLE_MAX_CHARS);
        assert!(t.starts_with('長'));

        let got = read_tail(write(&[title("タイトルだけ")], "").path()).unwrap();
        assert_eq!(
            (got.usage, got.title.as_deref()),
            (None, Some("タイトルだけ"))
        );
    }
}
