use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

use crate::codex::RolloutTail;
use crate::paths::MarimoHome;
use crate::repo::repo_name;
use crate::state::{
    CodexRateLimits, ContextUsage, HookInput, Origin, Provider, RateLimits, RateWindow,
    SessionState, Snapshot, Transition, aggregate, clean_title, context_from_codex,
    context_from_transcript, expire_agents, refresh_repo, transition,
};
use crate::time::{now_ms, rfc3339_utc, utc_date};
use crate::transcript::TranscriptTail;

// フックは Claude Code の処理を止めてしまうので、ロックを待つ時間に上限を設ける。
// 上限を過ぎたらロックなしで書く。書き込み自体は rename で原子的なので、
// 起きうるのは同時に来た更新の片方が失われることだけである。
const LOCK_WAIT: Duration = Duration::from_millis(300);

/// SessionEnd が届かずに終わったセッションを、最後の更新からこれだけ経ったら消す。
pub const STALE_SESSION_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// 調査用の記録はプロンプトやツールの引数を含みうるので、UTC の今日よりこの日数を超えて前の日のファイルは消す。
pub const RECORD_RETENTION_DAYS: u64 = 7;

// 以前の版が書いた記録のファイルは名前に日付を持たないので、mtime で判定して消す。
const LEGACY_RECORD_FILE: &str = "record.jsonl";

// セッションのファイルは実行中のコマンドの文字列を含むので、marimo が作るフォルダと
// ファイルは本人だけが読めるようにする。すでにあるフォルダの権限は利用者が決めたものとして変えない。
pub fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    create_private_dir_all(dir)?;
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
        let mut f = private_file_options().truncate(true).open(&tmp)?;
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

impl HomeLock {
    fn is_held(&self) -> bool {
        self._file.is_some()
    }
}

pub fn lock_home(home: &MarimoHome) -> HomeLock {
    HomeLock {
        _file: try_lock_file(&home.lock_file()),
    }
}

fn try_lock_file(path: &Path) -> Option<File> {
    create_private_dir_all(path.parent()?).ok()?;
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

pub fn read_session(
    home: &MarimoHome,
    provider: Provider,
    session_id: &str,
) -> Option<SessionState> {
    let path = home.session_file(provider, session_id)?;
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// フックの入力以外にフックの実行時だけ分かる情報。会話ログの読み取りはロックを取る前に済ませる。
#[derive(Debug, Clone, Default)]
pub struct HookExtras {
    pub origin: Option<Origin>,
    pub transcript: Option<TranscriptTail>,
    pub rollout: Option<RolloutTail>,
    /// Codex の session_index.jsonl から読んだ題名。
    pub title: Option<String>,
}

pub fn apply_hook(home: &MarimoHome, input: &HookInput, extras: &HookExtras) -> io::Result<()> {
    let path = home
        .session_file(input.provider, &input.session_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unusable session_id"))?;
    let result = apply_session(home, &path, input, extras);
    // 利用制限はアカウント全体の値なので、セッションの状態が変わらないイベントでも書く。
    // write_codex_rate_limits が自分でロックを取るので、セッションのロックを放してから呼ぶ。
    let limits = extras.rollout.as_ref().and_then(|r| r.rate_limits.as_ref());
    match limits.map(|l| write_codex_rate_limits(home, l)) {
        Some(Err(e)) if result.is_ok() => Err(e),
        _ => result,
    }
}

fn apply_session(
    home: &MarimoHome,
    path: &Path,
    input: &HookInput,
    extras: &HookExtras,
) -> io::Result<()> {
    let _lock = lock_home(home);
    let current = read_session(home, input.provider, &input.session_id);
    match transition(input, current.as_ref(), now_ms()) {
        Transition::Write(mut next) => {
            let usage = extras.transcript.as_ref().and_then(|t| t.usage.as_ref());
            let entrypoint = usage.and_then(|u| u.entrypoint.as_deref());
            next.origin = Origin::merge(next.origin.as_ref(), extras.origin.as_ref(), entrypoint);
            if let Some(u) = usage
                && let Some(ctx) =
                    context_from_transcript(next.context.as_ref(), u.context_tokens, now_ms())
            {
                next.context = Some(ctx);
            }
            if let Some(u) = extras.rollout.as_ref().and_then(|r| r.usage.as_ref()) {
                next.context = Some(context_from_codex(u, now_ms()));
            }
            let title = extras.transcript.as_ref().and_then(|t| t.title.clone());
            if let Some(title) = title.or_else(|| extras.title.clone()) {
                next.title = Some(title);
            }
            // 調べ直すかどうかは保存済みの cwd で決まるので、ロックの内側で調べる。
            // 調べるのは cwd が変わったときなどに限られ、数回の stat で済む。
            refresh_repo(input, current.as_ref(), &mut next, |cwd| {
                repo_name(Path::new(cwd))
            });
            write_json_atomic(path, &next)
        }
        Transition::Delete => match fs::remove_file(path) {
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
    let context = context_from(input, now);
    let title = input
        .get("session_name")
        .and_then(Value::as_str)
        .and_then(clean_title);
    if let Some(id) = input.get("session_id").and_then(Value::as_str)
        && (context.is_some() || title.is_some())
        && let (Some(path), Some(mut session)) = (
            home.session_file(Provider::Claude, id),
            read_session(home, Provider::Claude, id),
        )
    {
        if let Some(context) = context {
            session.context = Some(context);
        }
        if let Some(title) = title {
            session.title = Some(title);
        }
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

/// Codex のセッションはそれぞれ自分の rollout の末尾にある時点の利用制限を持つので、しばらく
/// 動いていないセッションのフックが古い値を届けることがある。保存済みの値と同じか新しい時点の
/// 値なら書かない。
pub fn write_codex_rate_limits(home: &MarimoHome, limits: &CodexRateLimits) -> io::Result<()> {
    let _lock = lock_home(home);
    if read_codex_rate_limits(home).is_some_and(|stored| stored.observed_at >= limits.observed_at) {
        return Ok(());
    }
    write_json_atomic(&home.codex_rate_limits_file(), limits)
}

pub fn read_codex_rate_limits(home: &MarimoHome) -> Option<CodexRateLimits> {
    let bytes = fs::read(home.codex_rate_limits_file()).ok()?;
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
        codex_rate_limits: read_codex_rate_limits(home),
    }
}

pub fn is_session_file_name(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".json")
}

fn is_temp_file_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}

/// `max_age` のあいだ更新のないセッションのファイルと、残った一時ファイルを消し、消した数を返す。
/// 残したセッションのうち、SubagentStop が届かないまま `AGENT_STALE_MS` を過ぎたサブエージェントを
/// 持つものは、そのサブエージェントを外して書き直す。親の会話もサブエージェントもフックを送らなく
/// なったセッションは、次のフックを待っていると作業中のまま残るからである。
///
/// 消したセッションがまだ動いていても、SessionEnd と SubagentStop 以外のフックが次に届けば
/// `transition` がファイルを作り直すので、行は次の操作で戻る。`transition` がファイルのないセッションに
/// `Transition::Nothing` を返すイベントを増やすときは、この前提を確かめ直す。
pub fn prune_stale_sessions(
    home: &MarimoHome,
    now_ms: u64,
    max_age: Duration,
) -> io::Result<usize> {
    // ロックなしで消すと、判定と削除のあいだにフックが書いた新しい状態まで消しうる。
    // 急ぐ処理ではないので、ロックが取れなければ次の機会に回す。
    let lock = lock_home(home);
    if !lock.is_held() {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "the marimo home is locked",
        ));
    }
    let entries = match fs::read_dir(home.sessions_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let now = UNIX_EPOCH + Duration::from_millis(now_ms);
    let max_age_ms = u64::try_from(max_age.as_millis()).unwrap_or(u64::MAX);
    let mut removed = 0;
    let mut first_err = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        let stale = if is_session_file_name(&name) {
            let session = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<SessionState>(&bytes).ok());
            // updated_at が 0 のファイルは、この項目を持たない古い形式で書かれたものなので、
            // 更新時刻が分からないものとして読めないファイルと同じく mtime で判定する。
            let stale = match session.as_ref().map(|s| s.updated_at).filter(|&t| t != 0) {
                Some(t) => t.saturating_add(max_age_ms) < now_ms,
                None => modified_before(&path, now, max_age),
            };
            if !stale
                && let Some(next) = session.and_then(|s| expire_agents(&s, now_ms))
                && let Err(e) = write_json_atomic(&path, &next)
            {
                first_err.get_or_insert(e);
            }
            stale
        } else {
            is_temp_file_name(&name) && modified_before(&path, now, max_age)
        };
        if !stale {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    first_err.map_or(Ok(removed), Err)
}

// mtime が未来にあるファイルは、時計が戻ったものとみなして古くないと判定する。
fn modified_before(path: &Path, now: SystemTime, max_age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age > max_age)
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

    create_private_dir_all(&home.logs_dir())?;
    {
        let _lock = lock_home(home);
        // 一行を一回の write で追記し、並行する記録どうしが行の途中で混ざらないようにする。
        private_file_options()
            .append(true)
            .open(home.record_file(&utc_date(now)))?
            .write_all(&line)?;
    }
    // 古いファイルを消せなくても記録そのものは済んでいるので、失敗は返さない。
    prune_records(home, now);
    Ok(())
}

fn prune_records(home: &MarimoHome, now_ms: u64) {
    let Ok(entries) = fs::read_dir(home.logs_dir()) else {
        return;
    };
    let retention = Duration::from_secs(RECORD_RETENTION_DAYS * 24 * 60 * 60);
    let oldest_kept = utc_date(now_ms.saturating_sub(retention.as_millis() as u64));
    let now = UNIX_EPOCH + Duration::from_millis(now_ms);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        let expired = match record_file_date(&name) {
            // 日付は固定長の数字なので、文字列の大小がそのまま日付の前後になる。
            Some(date) => date < oldest_kept.as_str(),
            None => name == LEGACY_RECORD_FILE && modified_before(&path, now, retention),
        };
        if expired {
            let _ = fs::remove_file(&path);
        }
    }
}

fn record_file_date(name: &str) -> Option<&str> {
    let date = name.strip_prefix("record-")?.strip_suffix(".jsonl")?;
    let shaped = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    shaped.then_some(date)
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

    fn record_files(home: &MarimoHome) -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = fs::read_dir(home.logs_dir())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        files
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

    #[cfg(unix)]
    #[test]
    fn created_files_and_folders_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path().join("new-home"));
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"SessionStart","source":"startup"}),
        );
        append_record(&home, "Stop", b"{}").unwrap();
        assert_eq!(mode(home.root()), 0o700);
        assert_eq!(mode(&home.sessions_dir()), 0o700);
        assert_eq!(mode(&home.logs_dir()), 0o700);
        assert_eq!(
            mode(&home.session_file(Provider::Claude, "s1").unwrap()),
            0o600
        );
        assert_eq!(mode(&record_files(&home)[0]), 0o600);

        // 利用者がすでに用意していたフォルダの権限は変えない。
        let existing = dir.path().join("existing");
        fs::create_dir(&existing).unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
        write_atomic(&existing.join("a.json"), b"{}").unwrap();
        assert_eq!(mode(&existing), 0o755);
        assert_eq!(mode(&existing.join("a.json")), 0o600);
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
        assert!(read_session(&home, Provider::Claude, "s1").is_none());
        let rl = read_rate_limits(&home).unwrap();
        assert_eq!(rl.five_hour.unwrap().resets_at, Some(1738425600));
        assert_eq!(rl.seven_day.unwrap().used_percentage, 41.2);

        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","prompt":"hi"}),
        );
        apply_statusline(&home, &status).unwrap();
        let ctx = read_session(&home, Provider::Claude, "s1")
            .unwrap()
            .context
            .unwrap();
        assert_eq!(ctx.used_percentage, Some(8.0));
        assert_eq!(ctx.total_input_tokens, Some(15500));
        assert_eq!(
            read_session(&home, Provider::Claude, "s1").unwrap().status,
            Status::Working
        );

        // rate_limits を持たない入力は、前に保存した利用制限を消さない。
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
            23.5
        );
    }

    #[test]
    fn title_comes_from_statusline_and_transcript() {
        let (_d, home) = home();
        let named = json!({"session_id": "s1", "session_name": "  ウィジェットの改修  "});
        apply_statusline(&home, &named).unwrap();
        assert!(read_session(&home, Provider::Claude, "s1").is_none());

        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","prompt":"hi"}),
        );
        apply_statusline(&home, &named).unwrap();
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .title
                .as_deref(),
            Some("ウィジェットの改修")
        );
        apply_statusline(&home, &json!({"session_id": "s1", "session_name": ""})).unwrap();
        apply_statusline(&home, &json!({"session_id": "s1"})).unwrap();
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .title
                .as_deref(),
            Some("ウィジェットの改修")
        );

        let stop: HookInput = serde_json::from_value(
            json!({"session_id":"s1","hook_event_name":"Stop","transcript_path":"/t.jsonl"}),
        )
        .unwrap();
        let with_title = |title: Option<&str>| HookExtras {
            origin: None,
            transcript: Some(TranscriptTail {
                usage: None,
                title: title.map(str::to_owned),
            }),
            ..HookExtras::default()
        };
        apply_hook(&home, &stop, &with_title(Some("デスクトップの題名"))).unwrap();
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .title
                .as_deref(),
            Some("デスクトップの題名")
        );
        apply_hook(&home, &stop, &with_title(None)).unwrap();
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .title
                .as_deref(),
            Some("デスクトップの題名")
        );
    }

    #[test]
    fn repo_follows_the_main_thread_cwd() {
        let (_d, home) = home();
        let work = tempfile::tempdir().unwrap();
        let repo = work.path().join("marimo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("app")).unwrap();
        let plain = work.path().join("notes");
        fs::create_dir_all(&plain).unwrap();
        let at = |cwd: &Path| json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","cwd":cwd.to_str().unwrap()});

        hook(&home, at(&repo.join("app")));
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .repo
                .as_deref(),
            Some("marimo")
        );
        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"SubagentStart","agent_id":"a1","cwd":plain.to_str().unwrap()}),
        );
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .repo
                .as_deref(),
            Some("marimo")
        );
        hook(&home, at(&plain));
        assert_eq!(
            read_session(&home, Provider::Claude, "s1").unwrap().repo,
            None
        );
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

    fn codex_limits(observed_at: u64, used: f64) -> CodexRateLimits {
        CodexRateLimits {
            windows: vec![crate::state::CodexRateWindow {
                window_minutes: Some(10080),
                used_percentage: used,
                resets_at: Some(1_800_000_000),
            }],
            plan_type: Some("plus".to_owned()),
            observed_at,
            updated_at: observed_at + 1,
        }
    }

    #[test]
    fn codex_sessions_live_beside_claude_sessions() {
        let (_d, home) = home();
        let codex = |v: Value, extras: &HookExtras| {
            let mut input: HookInput = serde_json::from_value(v).unwrap();
            input.provider = Provider::Codex;
            apply_hook(&home, &input, extras).unwrap();
        };
        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","cwd":"/w/a"}),
        );
        let extras = HookExtras {
            rollout: Some(RolloutTail {
                usage: Some(crate::state::CodexTokenUsage {
                    last_total_tokens: 142_000,
                    model_context_window: Some(272_000),
                }),
                rate_limits: Some(codex_limits(10, 5.0)),
            }),
            title: Some("Codex の題名".to_owned()),
            ..HookExtras::default()
        };
        codex(
            json!({"session_id":"s1","hook_event_name":"Stop","cwd":"/w/b","last_assistant_message":null}),
            &extras,
        );
        assert!(home.sessions_dir().join("s1.json").exists());
        assert!(home.sessions_dir().join("codex-s1.json").exists());
        let c = read_session(&home, Provider::Codex, "s1").unwrap();
        assert_eq!((c.provider, c.status), (Provider::Codex, Status::Done));
        assert_eq!(c.title.as_deref(), Some("Codex の題名"));
        let ctx = c.context.unwrap();
        assert_eq!(ctx.used_percentage, Some(50.0));
        assert_eq!(ctx.source.as_deref(), Some("codex-rollout"));
        let claude = read_session(&home, Provider::Claude, "s1").unwrap();
        assert_eq!(
            (claude.provider, claude.status, claude.cwd.as_deref()),
            (Provider::Claude, Status::Working, Some("/w/a"))
        );

        let snap = load_snapshot(&home);
        let keys: Vec<_> = snap
            .sessions
            .iter()
            .map(|s| (s.provider, s.session_id.as_str()))
            .collect();
        assert_eq!(
            keys,
            vec![(Provider::Claude, "s1"), (Provider::Codex, "s1")]
        );
        assert_eq!(snap.codex_rate_limits, Some(codex_limits(10, 5.0)));
        assert_eq!(snap.rate_limits, None);

        codex(
            json!({"session_id":"s1","hook_event_name":"SessionEnd","reason":"exit"}),
            &HookExtras::default(),
        );
        assert!(read_session(&home, Provider::Codex, "s1").is_none());
        assert!(read_session(&home, Provider::Claude, "s1").is_some());
    }

    #[test]
    fn codex_rate_limits_keep_the_newest_observation() {
        let (_d, home) = home();
        assert_eq!(read_codex_rate_limits(&home), None);
        write_codex_rate_limits(&home, &codex_limits(100, 10.0)).unwrap();
        for stale in [codex_limits(50, 99.0), codex_limits(100, 99.0)] {
            write_codex_rate_limits(&home, &stale).unwrap();
            assert_eq!(read_codex_rate_limits(&home), Some(codex_limits(100, 10.0)));
        }
        write_codex_rate_limits(&home, &codex_limits(200, 20.0)).unwrap();
        assert_eq!(read_codex_rate_limits(&home), Some(codex_limits(200, 20.0)));
    }

    const HOUR_MS: u64 = 60 * 60 * 1000;
    const NOW_MS: u64 = 1_790_509_325_123;

    fn set_mtime(path: &Path, ms: u64) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_millis(ms))
            .unwrap();
    }

    fn put_session(
        home: &MarimoHome,
        provider: Provider,
        id: &str,
        updated_at: u64,
        mtime_ms: u64,
    ) {
        let session = SessionState {
            updated_at,
            provider,
            ..SessionState::new(id)
        };
        let path = home.session_file(provider, id).unwrap();
        write_json_atomic(&path, &session).unwrap();
        set_mtime(&path, mtime_ms);
    }

    fn put_file(path: &Path, bytes: &[u8], mtime_ms: u64) {
        write_atomic(path, bytes).unwrap();
        set_mtime(path, mtime_ms);
    }

    #[test]
    fn prune_removes_only_stale_sessions_and_temp_files() {
        let (_d, home) = home();
        assert_eq!(
            prune_stale_sessions(&home, NOW_MS, STALE_SESSION_AGE).unwrap(),
            0
        );

        let dir = home.sessions_dir();
        // updated_at があれば mtime より優先する。
        put_session(
            &home,
            Provider::Claude,
            "stale",
            NOW_MS - 25 * HOUR_MS,
            NOW_MS,
        );
        put_session(
            &home,
            Provider::Claude,
            "fresh",
            NOW_MS - 23 * HOUR_MS,
            NOW_MS - 48 * HOUR_MS,
        );
        // 読めないファイルと updated_at が 0 のファイルは mtime で判定する。
        put_file(
            &dir.join("broken-old.json"),
            b"not json",
            NOW_MS - 25 * HOUR_MS,
        );
        put_file(&dir.join("broken-new.json"), b"not json", NOW_MS - HOUR_MS);
        put_session(
            &home,
            Provider::Claude,
            "zero-old",
            0,
            NOW_MS - 25 * HOUR_MS,
        );
        put_session(&home, Provider::Claude, "zero-new", 0, NOW_MS - HOUR_MS);
        put_file(&dir.join(".a.json.1.2.tmp"), b"{", NOW_MS - 25 * HOUR_MS);
        put_file(&dir.join(".b.json.1.3.tmp"), b"{", NOW_MS - HOUR_MS);
        put_file(&dir.join("notes.txt"), b"x", NOW_MS - 48 * HOUR_MS);
        // Codex のセッションは同じ session_id の Claude Code のセッションと別に判定する。
        put_session(&home, Provider::Codex, "stale", NOW_MS - HOUR_MS, NOW_MS);
        put_session(
            &home,
            Provider::Codex,
            "fresh",
            NOW_MS - 25 * HOUR_MS,
            NOW_MS,
        );

        let removed = prune_stale_sessions(&home, NOW_MS, STALE_SESSION_AGE).unwrap();
        assert_eq!(removed, 5);
        assert!(read_session(&home, Provider::Codex, "stale").is_some());
        assert!(read_session(&home, Provider::Codex, "fresh").is_none());
        assert!(read_session(&home, Provider::Claude, "stale").is_none());
        assert!(read_session(&home, Provider::Claude, "fresh").is_some());
        assert!(!dir.join("broken-old.json").exists());
        assert!(dir.join("broken-new.json").exists());
        assert!(read_session(&home, Provider::Claude, "zero-old").is_none());
        assert!(read_session(&home, Provider::Claude, "zero-new").is_some());
        assert!(!dir.join(".a.json.1.2.tmp").exists());
        assert!(dir.join(".b.json.1.3.tmp").exists());
        assert!(dir.join("notes.txt").exists());

        // 消したセッションも、次のフックで作り直される。
        hook(
            &home,
            json!({"session_id":"stale","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}),
        );
        assert_eq!(
            read_session(&home, Provider::Claude, "stale")
                .unwrap()
                .status,
            Status::Working
        );
    }

    #[test]
    fn prune_expires_agents_that_never_stopped() {
        let (_d, home) = home();
        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"SubagentStart","agent_id":"a1","agent_type":"Explore"}),
        );
        hook(
            &home,
            json!({"session_id":"s1","hook_event_name":"Stop","last_assistant_message":"ok"}),
        );
        let before = read_session(&home, Provider::Claude, "s1").unwrap();
        assert_eq!(before.status, Status::Working);

        let soon = before.updated_at + 60_000;
        assert_eq!(
            prune_stale_sessions(&home, soon, STALE_SESSION_AGE).unwrap(),
            0
        );
        assert_eq!(read_session(&home, Provider::Claude, "s1").unwrap(), before);

        let later = before.updated_at + crate::state::AGENT_STALE_MS + 60_000;
        assert_eq!(
            prune_stale_sessions(&home, later, STALE_SESSION_AGE).unwrap(),
            0
        );
        let after = read_session(&home, Provider::Claude, "s1").unwrap();
        assert_eq!((after.status, after.agents.len()), (Status::Done, 0));
        assert_eq!(after.updated_at, before.updated_at);
    }

    #[test]
    fn record_appends_lines() {
        let (_d, home) = home();
        let expired = home.record_file(&utc_date(now_ms() - 30 * 24 * HOUR_MS));
        put_file(&expired, b"{}\n", now_ms());
        let before = utc_date(now_ms());
        append_record(&home, "PreToolUse", br#"{"a":1}"#).unwrap();
        append_record(&home, "statusline", b"not json").unwrap();
        let after = utc_date(now_ms());
        // 追記のたびに保存期間を過ぎたファイルを消す。
        assert!(!expired.exists());
        // 二回の追記のあいだに UTC の日付が変わった場合だけ、ファイルが二つに分かれる。
        let files = record_files(&home);
        let expected: Vec<_> = [before, after]
            .iter()
            .map(|d| home.record_file(d))
            .collect();
        assert!(files.iter().all(|f| expected.contains(f)), "{files:?}");
        let text: String = files
            .iter()
            .map(|f| fs::read_to_string(f).unwrap())
            .collect();
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

    #[test]
    fn record_retention_removes_only_expired_files() {
        let (_d, home) = home();
        let logs = home.logs_dir();
        let names = [
            "record-2026-09-19.jsonl",
            "record-2026-09-20.jsonl",
            "record-2026-09-27.jsonl",
            "record-2026-09-28.jsonl",
            "record-2026-9-1.jsonl",
            "record-notes.jsonl",
            "other.jsonl",
        ];
        for name in names {
            put_file(&logs.join(name), b"{}\n", NOW_MS - 30 * 24 * HOUR_MS);
        }
        // NOW_MS は 2026-09-27 なので、7 日より前の 09-19 だけが消える。
        prune_records(&home, NOW_MS);
        let left: Vec<_> = record_files(&home)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let mut expected: Vec<_> = names[1..].iter().map(|n| n.to_string()).collect();
        expected.sort();
        assert_eq!(left, expected);

        // 日付を持たない以前の record.jsonl は mtime で判定する。
        let legacy = logs.join("record.jsonl");
        put_file(&legacy, b"{}\n", NOW_MS - 6 * 24 * HOUR_MS);
        prune_records(&home, NOW_MS);
        assert!(legacy.exists());
        set_mtime(&legacy, NOW_MS - 8 * 24 * HOUR_MS);
        prune_records(&home, NOW_MS);
        assert!(!legacy.exists());
    }
}
