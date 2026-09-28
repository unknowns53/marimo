use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::paths::user_home;
use crate::state::{CodexRateLimits, CodexRateWindow, CodexTokenUsage, clean_title};
use crate::time::parse_rfc3339_utc_ms;
use crate::transcript::{complete_lines, contains, search_tail};

/// Codex の設定とデータを置くフォルダ。Codex と同じく CODEX_HOME があればそれを、なければ ~/.codex を使う。
pub fn codex_home() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    user_home().map(|h| h.join(".codex"))
}

/// rollout の末尾から読み取れたもの。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RolloutTail {
    pub usage: Option<CodexTokenUsage>,
    pub rate_limits: Option<CodexRateLimits>,
}

// 項目名は codex-rs/protocol/src/protocol.rs の TokenCountEvent、TokenUsageInfo、
// RateLimitSnapshot、RateLimitWindow に従う。rollout の各行はこれを payload に入れ、
// type を event_msg、payload.type を token_count として書く。
#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    info: Option<Info>,
    rate_limits: Option<Limits>,
}

#[derive(Deserialize)]
struct Info {
    last_token_usage: Option<Usage>,
    model_context_window: Option<i64>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    total_tokens: i64,
}

#[derive(Deserialize)]
struct Limits {
    primary: Option<Window>,
    secondary: Option<Window>,
    plan_type: Option<Value>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: f64,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

/// rollout の末尾から、最後の token_count の使用量と利用制限を探す。token_count は info か
/// rate_limits の片方だけを持つことがあるので、それぞれ最後に値を持つ行から取る。
pub fn read_rollout_tail(path: &Path, now_ms: u64) -> io::Result<RolloutTail> {
    search_tail(path, |buf, head| {
        let tail = scan(buf, head, now_ms);
        let found = tail.usage.is_some();
        (tail, found)
    })
}

fn scan(buf: &[u8], starts_at_file_head: bool, now_ms: u64) -> RolloutTail {
    let mut tail = RolloutTail::default();
    let token_counts = complete_lines(buf, starts_at_file_head)
        .into_iter()
        .rev()
        .filter(|l| contains(l, b"\"token_count\""))
        .filter_map(parse_token_count);
    for (timestamp, payload) in token_counts {
        if tail.usage.is_none() {
            tail.usage = payload.info.as_ref().and_then(|info| {
                Some(CodexTokenUsage {
                    last_total_tokens: info.last_token_usage.as_ref()?.total_tokens,
                    model_context_window: info.model_context_window.filter(|w| *w > 0),
                })
            });
        }
        if tail.rate_limits.is_none() {
            tail.rate_limits = payload
                .rate_limits
                .and_then(|l| rate_limits(l, timestamp?, now_ms));
        }
        if tail.usage.is_some() && tail.rate_limits.is_some() {
            break;
        }
    }
    tail
}

fn parse_token_count(bytes: &[u8]) -> Option<(Option<u64>, Payload)> {
    let line: Line = serde_json::from_slice(bytes).ok()?;
    let payload = line.payload?;
    if line.kind.as_deref() != Some("event_msg") || payload.kind.as_deref() != Some("token_count") {
        return None;
    }
    let timestamp = line.timestamp.as_deref().and_then(parse_rfc3339_utc_ms);
    Some((timestamp, payload))
}

// primary が 5 時間の窓とは限らない（プランによっては primary が 7 日で secondary が null になる）ので、
// primary と secondary の区別は捨てて、値のある窓だけを並べる。
fn rate_limits(limits: Limits, observed_at: u64, now_ms: u64) -> Option<CodexRateLimits> {
    let windows: Vec<CodexRateWindow> = [limits.primary, limits.secondary]
        .into_iter()
        .flatten()
        .map(|w| CodexRateWindow {
            window_minutes: w.window_minutes.and_then(|m| u64::try_from(m).ok()),
            used_percentage: w.used_percent,
            resets_at: w.resets_at,
        })
        .collect();
    (!windows.is_empty()).then(|| CodexRateLimits {
        windows,
        plan_type: limits
            .plan_type
            .as_ref()
            .and_then(Value::as_str)
            .map(str::to_owned),
        observed_at,
        updated_at: now_ms,
    })
}

/// rollout の先頭の session_meta から、Codex のデスクトップアプリが始めた会話かを判断する。
/// originator の値は Codex のドキュメントに無く、手元の rollout で、デスクトップアプリが "Codex Desktop" を、
/// 端末の Codex CLI が "codex-tui" を書くことを確かめた。先頭の行は基本の指示を含んで 20 KB ほどになるので、
/// 64 KB まで読んで行が終わらなければ諦める。
pub fn started_by_desktop_app(path: &Path) -> bool {
    #[derive(Deserialize)]
    struct Meta {
        #[serde(rename = "type")]
        kind: String,
        payload: MetaPayload,
    }
    #[derive(Deserialize)]
    struct MetaPayload {
        originator: Option<String>,
    }
    const HEAD_LIMIT: u64 = 64 * 1024;
    let mut head = Vec::new();
    let read = File::open(path).and_then(|f| f.take(HEAD_LIMIT).read_to_end(&mut head));
    let Some(end) = read.ok().and(head.iter().position(|b| *b == b'\n')) else {
        return false;
    };
    serde_json::from_slice::<Meta>(&head[..end]).is_ok_and(|m| {
        m.kind == "session_meta" && m.payload.originator.as_deref() == Some("Codex Desktop")
    })
}

/// `$CODEX_HOME/session_index.jsonl` の末尾から、この会話の最後の thread_name を探す。
/// 名前を付け直すたびに行が書き足され、最後の行が有効になる（codex-rs/rollout/src/session_index.rs）。
/// 題名のために大きく読むことはせず、末尾の一回分の範囲に無ければ諦める。
pub fn thread_name(codex_home: &Path, session_id: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        thread_name: String,
    }
    let path = codex_home.join("session_index.jsonl");
    search_tail(&path, |buf, head| {
        let name = complete_lines(buf, head)
            .into_iter()
            .rev()
            .filter(|l| contains(l, session_id.as_bytes()))
            .filter_map(|l| serde_json::from_slice::<Entry>(l).ok())
            .find(|e| e.id == session_id)
            .and_then(|e| clean_title(&e.thread_name));
        (name, true)
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn write(lines: &[String]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        f.flush().unwrap();
        f
    }

    fn token_count(ts: &str, info: Value, rate_limits: Value) -> String {
        json!({"timestamp": ts, "type": "event_msg",
               "payload": {"type": "token_count", "info": info, "rate_limits": rate_limits}})
        .to_string()
    }

    fn info(total: i64, window: i64) -> Value {
        let usage = json!({"input_tokens": total - 10, "cached_input_tokens": 0,
                           "output_tokens": 10, "reasoning_output_tokens": 0, "total_tokens": total});
        json!({"total_token_usage": usage, "last_token_usage": usage, "model_context_window": window})
    }

    fn limits(primary: Value, secondary: Value) -> Value {
        json!({"limit_id": "codex", "primary": primary, "secondary": secondary,
               "credits": null, "plan_type": "plus"})
    }

    #[test]
    fn takes_usage_and_limits_from_the_last_token_counts() {
        let week =
            json!({"used_percent": 42.5, "window_minutes": 10080, "resets_at": 1_800_000_000});
        let lines = vec![
            token_count(
                "2026-09-27T00:00:00.000Z",
                info(1000, 200_000),
                limits(week.clone(), json!(null)),
            ),
            token_count(
                "2026-09-27T00:00:05.000Z",
                info(50_000, 272_000),
                limits(
                    json!({"used_percent": 7.0, "window_minutes": 300, "resets_at": null}),
                    week,
                ),
            ),
            json!({"timestamp": "2026-09-27T00:00:06.000Z", "type": "response_item",
                   "payload": {"type": "message", "content": "\"token_count\""}})
            .to_string(),
            token_count("2026-09-27T00:00:07.000Z", json!(null), json!(null)),
        ];
        let tail = read_rollout_tail(write(&lines).path(), 99).unwrap();
        assert_eq!(
            tail.usage,
            Some(CodexTokenUsage {
                last_total_tokens: 50_000,
                model_context_window: Some(272_000)
            })
        );
        let rl = tail.rate_limits.unwrap();
        assert_eq!(rl.observed_at, 1_790_467_205_000);
        assert_eq!((rl.updated_at, rl.plan_type.as_deref()), (99, Some("plus")));
        assert_eq!(rl.windows.len(), 2);
        assert_eq!(
            (rl.windows[0].window_minutes, rl.windows[0].resets_at),
            (Some(300), None)
        );
        assert_eq!(rl.windows[1].used_percentage, 42.5);

        // 窓がどれも null の利用制限と、timestamp の読めない行の利用制限は使わない。
        let lines = vec![
            token_count(
                "bad",
                info(1, 2),
                limits(json!({"used_percent": 1.0}), json!(null)),
            ),
            token_count(
                "2026-09-27T00:00:00Z",
                info(3000, 128_000),
                limits(json!(null), json!(null)),
            ),
        ];
        let tail = read_rollout_tail(write(&lines).path(), 1).unwrap();
        assert_eq!(tail.usage.unwrap().last_total_tokens, 3000);
        assert_eq!(tail.rate_limits, None);
    }

    #[test]
    fn thread_name_takes_the_last_row_for_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let row = |id: &str, name: &str| {
            json!({"id": id, "thread_name": name, "updated_at": "2026-09-27T00:00:00Z"}).to_string()
        };
        let rows = [
            row("t1", "最初の名前"),
            row("t2", "別の会話"),
            row("t1", "  付け直した名前  "),
            row("t10", "似た id の会話"),
            "{broken t1".to_owned(),
        ];
        std::fs::write(
            dir.path().join("session_index.jsonl"),
            rows.join("\n") + "\n",
        )
        .unwrap();
        assert_eq!(
            thread_name(dir.path(), "t1").as_deref(),
            Some("付け直した名前")
        );
        assert_eq!(thread_name(dir.path(), "t3"), None);
        assert_eq!(thread_name(&dir.path().join("missing"), "t1"), None);
    }

    #[test]
    fn started_by_desktop_app_reads_the_session_meta_originator() {
        let meta = |originator: &str, instructions: usize| {
            json!({"timestamp": "2026-09-20T01:00:00.000Z", "type": "session_meta",
                   "payload": {"id": "t1", "originator": originator, "cli_version": "0.130.0",
                               "base_instructions": {"text": "x".repeat(instructions)}}})
            .to_string()
        };
        let event = json!({"timestamp": "2026-09-20T01:00:00.000Z", "type": "event_msg",
                           "payload": {"type": "user_message", "originator": "Codex Desktop"}})
        .to_string();
        let cases = [
            (vec![meta("Codex Desktop", 20_000)], true),
            (vec![meta("codex-tui", 20_000)], false),
            (vec![meta("Codex Desktop", 70_000)], false),
            (vec![event, meta("Codex Desktop", 10)], false),
        ];
        for (i, (lines, want)) in cases.iter().enumerate() {
            assert_eq!(
                started_by_desktop_app(write(lines).path()),
                *want,
                "case {i}"
            );
        }
    }
}
