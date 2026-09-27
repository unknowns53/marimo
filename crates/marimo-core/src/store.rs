use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::paths::MarimoHome;
use crate::state::{
    ContextUsage, HookInput, Origin, RateLimits, RateWindow, SessionState, Snapshot, Transition,
    aggregate, context_from_transcript, transition,
};
use crate::time::{now_ms, rfc3339_utc};
use crate::transcript::TranscriptUsage;

// フックは Claude Code の処理を止めてしまうので、ロックを待つ時間に上限を設ける。
// 上限を過ぎたらロックなしで書く。書き込み自体は rename で原子的なので、
// 起きうるのは同時に来た更新の片方が失われることだけである。
const LOCK_WAIT: Duration = Duration::from_millis(300);

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    // 監視側は `.` で始まる名前と `.tmp` を無視する。同じディレクトリに置くのは、
    // rename が同一ボリューム内でしか原子的にならないためである。
    // 一時ファイルの名前はプロセスとプロセス内の通し番号で一意にする。macOS の時計は
    // マイクロ秒単位なので、時刻を使うと同じプロセスのスレッドどうしで名前がぶつかる。
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        // 状態ファイルは次のフックで作り直せるので、fsync は呼ばない。macOS の
        // sync_all は F_FULLFSYNC になり、数十ミリ秒の予算を一回で使い切ることがある。
    }
    let result = rename_with_retry(&tmp, path);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

// Windows ではウイルス対策ソフトなどが対象を一瞬開いていると rename が失敗するので、
// ごく短く数回だけ再試行する。
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < 5 && cfg!(windows) => {
                attempt += 1;
                thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(e),
        }
    }
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

pub struct HomeLock {
    _file: Option<File>,
}

pub fn lock_home(home: &MarimoHome) -> HomeLock {
    HomeLock {
        _file: try_lock_file(&home.lock_file()),
    }
}

fn try_lock_file(path: &Path) -> Option<File> {
    fs::create_dir_all(path.parent()?).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Some(file),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return None,
        }
    }
}

pub fn read_session(home: &MarimoHome, session_id: &str) -> Option<SessionState> {
    let path = home.session_file(session_id)?;
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// フックの入力以外にフックの実行時だけ分かる情報。会話ログの読み取りはロックを取る前に済ませる。
#[derive(Debug, Clone, Default)]
pub struct HookExtras {
    pub origin: Option<Origin>,
    pub transcript: Option<TranscriptUsage>,
}

pub fn apply_hook(home: &MarimoHome, input: &HookInput, extras: &HookExtras) -> io::Result<()> {
    let path = home
        .session_file(&input.session_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unusable session_id"))?;
    let _lock = lock_home(home);
    let current = read_session(home, &input.session_id);
    match transition(input, current.as_ref(), now_ms()) {
        Transition::Write(mut next) => {
            let entrypoint = extras
                .transcript
                .as_ref()
                .and_then(|t| t.entrypoint.as_deref());
            next.origin = Origin::merge(next.origin.as_ref(), extras.origin.as_ref(), entrypoint);
            if let Some(t) = &extras.transcript
                && let Some(ctx) =
                    context_from_transcript(next.context.as_ref(), t.context_tokens, now_ms())
            {
                next.context = Some(ctx);
            }
            write_json_atomic(&path, &next)
        }
        Transition::Delete => match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        Transition::Nothing => Ok(()),
    }
}

/// 項目名は https://code.claude.com/docs/en/statusline の Available data の節に従う。
pub fn apply_statusline(home: &MarimoHome, input: &Value) -> io::Result<()> {
    let now = now_ms();
    let mut first_err = None;

    if let Some(limits) = rate_limits_from(input, now)
        && let Err(e) = write_rate_limits(home, &limits)
    {
        first_err.get_or_insert(e);
    }

    let _lock = lock_home(home);

    // セッションのファイルはフックだけが作る。SessionEnd で消した後に statusLine が
    // 遅れて届いても、終わったセッションを復活させないためである。
    if let (Some(id), Some(context)) = (
        input.get("session_id").and_then(Value::as_str),
        context_from(input, now),
    ) && let (Some(path), Some(mut session)) = (home.session_file(id), read_session(home, id))
    {
        session.context = Some(context);
        if let Err(e) = write_json_atomic(&path, &session) {
            first_err.get_or_insert(e);
        }
    }

    first_err.map_or(Ok(()), Err)
}

fn context_from(input: &Value, now: u64) -> Option<ContextUsage> {
    let cw = input.get("context_window")?;
    let usage = ContextUsage {
        used_percentage: cw.get("used_percentage").and_then(Value::as_f64),
        total_input_tokens: cw.get("total_input_tokens").and_then(Value::as_u64),
        context_window_size: cw.get("context_window_size").and_then(Value::as_u64),
        updated_at: now,
        source: Some("statusline".to_owned()),
    };
    (usage.used_percentage.is_some() || usage.total_input_tokens.is_some()).then_some(usage)
}

fn rate_limits_from(input: &Value, now: u64) -> Option<RateLimits> {
    let rl = input.get("rate_limits")?;
    let window = |key: &str| {
        let w = rl.get(key)?;
        Some(RateWindow {
            used_percentage: w.get("used_percentage")?.as_f64()?,
            resets_at: w
                .get("resets_at")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))),
        })
    };
    let limits = RateLimits {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
        updated_at: now,
    };
    (limits.five_hour.is_some() || limits.seven_day.is_some()).then_some(limits)
}

/// 利用制限は statusLine とアプリの API の取得の両方から届くので、どちらもここを通して書く。
/// ロックは内側で取るので、呼び出し側は `lock_home` を持ったまま呼ばない。
pub fn write_rate_limits(home: &MarimoHome, limits: &RateLimits) -> io::Result<()> {
    let _lock = lock_home(home);
    write_json_atomic(&home.rate_limits_file(), limits)
}

pub fn read_rate_limits(home: &MarimoHome) -> Option<RateLimits> {
    let bytes = fs::read(home.rate_limits_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn load_snapshot(home: &MarimoHome) -> Snapshot {
    let mut sessions: Vec<SessionState> = fs::read_dir(home.sessions_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| is_session_file_name(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<SessionState>(&bytes).ok())
        .collect();
    sessions.sort_by(|a, b| {
        b.status
            .priority()
            .cmp(&a.status.priority())
            .then(b.updated_at.cmp(&a.updated_at))
    });
    Snapshot {
        aggregate: aggregate(sessions.iter().map(|s| s.status)),
        sessions,
        rate_limits: read_rate_limits(home),
    }
}

pub fn is_session_file_name(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".json")
}

pub fn append_record(home: &MarimoHome, label: &str, stdin: &[u8]) -> io::Result<()> {
    let now = now_ms();
    let payload: Value = serde_json::from_slice(stdin)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(stdin) }));
    let mut line = serde_json::to_vec(&json!({
        "received_at": rfc3339_utc(now),
        "received_at_ms": now,
        "label": label,
        "payload": payload,
    }))
    .map_err(io::Error::other)?;
    line.push(b'\n');

    let path = home.record_file();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let _lock = lock_home(home);
    // 一行を一回の write で追記し、並行する記録どうしが行の途中で混ざらないようにする。
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(&line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Status;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn home() -> (tempfile::TempDir, MarimoHome) {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        (dir, home)
    }

    fn hook(home: &MarimoHome, v: Value) {
        apply_hook(
            home,
            &serde_json::from_value(v).unwrap(),
            &HookExtras::default(),
        )
        .unwrap();
    }

    #[test]
    fn atomic_write_leaves_no_temp_files() {
        let (_d, home) = home();
        let path = home.sessions_dir().join("a.json");
        write_atomic(&path, b"{\"v\":1}").unwrap();
        write_atomic(&path, b"{\"v\":2}").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"v\":2}");
        let names: Vec<_> = fs::read_dir(home.sessions_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("a.json")]);
    }

    #[test]
    fn readers_never_see_partial_writes() {
        let (_d, home) = home();
        let path = home.sessions_dir().join("big.json");
        let big = |n: usize| {
            serde_json::to_vec(&json!({ "n": n, "pad": "x".repeat(256 * 1024) })).unwrap()
        };
        write_atomic(&path, &big(0)).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..4)
            .map(|w| {
                let path = path.clone();
                let stop = stop.clone();
                thread::spawn(move || {
                    let mut i = 0;
                    while !stop.load(Ordering::Relaxed) {
                        write_atomic(&path, &big(w * 100_000 + i)).unwrap();
                        i += 1;
                    }
                })
            })
            .collect();

        let deadline = Instant::now() + Duration::from_millis(700);
        let mut reads = 0;
        while Instant::now() < deadline {
            let bytes = fs::read(&path).unwrap();
            let parsed: Result<Value, _> = serde_json::from_slice(&bytes);
            assert!(
                parsed.is_ok(),
                "reader saw a partial file of {} bytes",
                bytes.len()
            );
            reads += 1;
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
        assert!(reads > 10);
    }

    #[test]
    fn hook_lifecycle_on_disk() {
        let (_d, home) = home();
        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"SessionStart","source":"startup","cwd":"/w/a"}),
        );
        assert_eq!(read_session(&home, "s1").unwrap().status, Status::Idle);
        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test"}}),
        );
        let s = read_session(&home, "s1").unwrap();
        assert_eq!(s.status, Status::Working);
        let a = s.activity.unwrap();
        assert_eq!(
            (a.summary.as_str(), a.detail.as_deref()),
            ("cargo test", Some("cargo test"))
        );
        assert_eq!(s.cwd.as_deref(), Some("/w/a"));
        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"SessionEnd","reason":"other"}),
        );
        assert!(read_session(&home, "s1").is_none());
        assert!(!home.session_file("s1").unwrap().exists());
    }

    #[test]
    fn statusline_updates_context_and_rate_limits() {
        let (_d, home) = home();
        let status = json!({
            "session_id": "s1",
            "context_window": {"total_input_tokens": 15500, "context_window_size": 200000, "used_percentage": 8},
            "rate_limits": {
                "five_hour": {"used_percentage": 23.5, "resets_at": 1738425600},
                "seven_day": {"used_percentage": 41.2, "resets_at": 1738857600}
            }
        });
        // フックがまだファイルを作っていないセッションは作らない。
        apply_statusline(&home, &status).unwrap();
        assert!(read_session(&home, "s1").is_none());
        let rl = read_rate_limits(&home).unwrap();
        assert_eq!(rl.five_hour.unwrap().resets_at, Some(1738425600));
        assert_eq!(rl.seven_day.unwrap().used_percentage, 41.2);

        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","prompt":"hi"}),
        );
        apply_statusline(&home, &status).unwrap();
        let ctx = read_session(&home, "s1").unwrap().context.unwrap();
        assert_eq!(ctx.used_percentage, Some(8.0));
        assert_eq!(ctx.total_input_tokens, Some(15500));
        assert_eq!(read_session(&home, "s1").unwrap().status, Status::Working);
    }

    #[test]
    fn statusline_without_rate_limits_keeps_previous_file() {
        let (_d, home) = home();
        apply_statusline(
            &home,
            &json!({"rate_limits": {"five_hour": {"used_percentage": 10}}}),
        )
        .unwrap();
        apply_statusline(
            &home,
            &json!({"session_id": "x", "context_window": {"used_percentage": null}}),
        )
        .unwrap();
        assert_eq!(
            read_rate_limits(&home)
                .unwrap()
                .five_hour
                .unwrap()
                .used_percentage,
            10.0
        );
    }

    #[test]
    fn rate_limits_round_trip_through_the_shared_writer() {
        let (_d, home) = home();
        let limits = RateLimits {
            five_hour: Some(RateWindow {
                used_percentage: 23.0,
                resets_at: Some(1_790_535_600),
            }),
            seven_day: None,
            updated_at: 1_790_509_325_123,
        };
        write_rate_limits(&home, &limits).unwrap();
        assert_eq!(read_rate_limits(&home), Some(limits));
    }

    #[test]
    fn snapshot_aggregates_and_orders() {
        let (_d, home) = home();
        hook(
            &home,
            json!({"session_id":"a","hook_event_name":"UserPromptSubmit"}),
        );
        hook(
            &home,
            json!({"session_id":"b","hook_event_name":"Stop","last_assistant_message":"ok"}),
        );
        hook(
            &home,
            json!({"session_id":"c","hook_event_name":"SessionStart","source":"startup"}),
        );
        write_atomic(&home.sessions_dir().join(".c.json.1.2.tmp"), b"{broken").unwrap();
        write_atomic(&home.sessions_dir().join("junk.json"), b"not json").unwrap();
        let snap = load_snapshot(&home);
        assert_eq!(snap.aggregate, Status::Working);
        let ids: Vec<_> = snap
            .sessions
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);

        hook(
            &home,
            json!({"session_id":"c","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"rm -rf x"}}),
        );
        assert_eq!(load_snapshot(&home).aggregate, Status::Waiting);
    }

    #[test]
    fn record_appends_lines() {
        let (_d, home) = home();
        append_record(&home, "PreToolUse", br#"{"a":1}"#).unwrap();
        append_record(&home, "statusline", b"not json").unwrap();
        let text = fs::read_to_string(home.record_file()).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["label"], "PreToolUse");
        assert_eq!(lines[0]["payload"]["a"], 1);
        assert_eq!(lines[1]["payload"]["raw"], "not json");
        assert!(lines[1]["received_at"].as_str().unwrap().ends_with('Z'));
    }
}
