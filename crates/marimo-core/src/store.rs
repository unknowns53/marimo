use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::antigravity;
use crate::codex::RolloutTail;
use crate::lang::Lang;
use crate::paths::MarimoHome;
use crate::process;
use crate::repo::repo_name;
use crate::state::{
    AgentRun, CodexRateLimits, ContextUsage, HookInput, HostProcess, Origin, Provider, RateLimits,
    RateWindow, SessionState, Snapshot, Status, Transition, aggregate, clean_title,
    context_from_codex, context_from_transcript, expire_agents, refresh_repo, transition,
    without_agents,
};
use crate::time::{now_ms, parse_rfc3339_utc_ms, rfc3339_utc, utc_date};
use crate::transcript::{self, LastTurn, TranscriptTail};

// フックは Claude Code の処理を止めてしまうので、ロックを待つ時間に上限を設ける。
// 上限を過ぎたらロックなしで書く。書き込み自体は rename で原子的なので、
// 起きうるのは同時に来た更新の片方が失われることだけである。
const LOCK_WAIT: Duration = Duration::from_millis(300);

/// SessionEnd が届かずに終わったセッションを、最後の更新からこれだけ経ったら消す。
pub const STALE_SESSION_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// 調査用の記録はプロンプトやツールの引数を含みうるので、UTC の今日よりこの日数を超えて前の日のファイルは消す。
pub const RECORD_RETENTION_DAYS: u64 = 7;

// 以前のバージョンが書いた記録のファイルは名前に日付を持たないので、mtime で判定して消す。
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
    write_atomic_with(path, bytes, false)
}

/// 作り直せないユーザーの設定に使う。内容をディスクへ書き切ってから置き換えるので、電源が落ちても
/// 古い内容か新しい内容のどちらかが残り、空のファイルにはならない。
pub fn write_json_durable<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic_with(path, &bytes, true)
}

fn write_atomic_with(path: &Path, bytes: &[u8], durable: bool) -> io::Result<()> {
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
    let result = write_new(&tmp, bytes, durable).and_then(|()| rename_with_retry(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_new(path: &Path, bytes: &[u8], durable: bool) -> io::Result<()> {
    let mut f = private_file_options().truncate(true).open(path)?;
    f.write_all(bytes)?;
    // 状態ファイルは次のフックで作り直せるので、fsync は呼ばない。macOS の
    // sync_all は F_FULLFSYNC になり、数十ミリ秒の予算を一回で使い切ることがある。
    if durable {
        f.sync_all()?;
    }
    Ok(())
}

// Windows ではウイルス対策ソフトや検索のインデクサが対象を一瞬開いていると rename が失敗する。Stop のフックの
// 書き込みが失敗すると行が作業中のまま残るので、間隔を倍にしながら合わせて 500 ミリ秒ほどまで再試行する。
// フックの制限時間は 5 秒あるので、この待ちは収まる。
const RENAME_RETRY_BUDGET: Duration = Duration::from_millis(500);

fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    if !cfg!(windows) {
        return fs::rename(from, to);
    }
    retry_transient(
        RENAME_RETRY_BUDGET,
        Duration::from_millis(5),
        Duration::from_millis(100),
        || fs::rename(from, to),
    )
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

// Windows で、別のプロセスがファイルを開いているために一瞬だけ失敗する操作のエラー。ERROR_ACCESS_DENIED は
// PermissionDenied に読み替えられるが、ERROR_SHARING_VIOLATION（32）と ERROR_LOCK_VIOLATION（33）は
// Rust のバージョンによって読み替え先が変わるので、番号で見る。
fn is_transient(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(32 | 33))
}

// `op` が一時的なエラーで失敗する間、`first_delay` から倍にしながら `max_delay` までの間隔を空けて、
// 待った時間の合計が `budget` に届くまで繰り返す。
fn retry_transient<T>(
    budget: Duration,
    first_delay: Duration,
    max_delay: Duration,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut slept = Duration::ZERO;
    let mut delay = first_delay;
    loop {
        match op() {
            Err(e) if is_transient(&e) && slept < budget => {
                let wait = delay.min(budget - slept);
                thread::sleep(wait);
                slept += wait;
                delay = (delay * 2).min(max_delay);
            }
            result => return result,
        }
    }
}

// Windows では、別のプロセスやスレッドが rename でファイルを差し替えている瞬間に開くと、アクセス拒否か
// 共有違反で失敗する。読めなかったファイルは飛ばすので、そのままでは行が一回分消えたり、全体の状態が
// ちらついたりする。書く側の rename_with_retry と同じく、ただしごく短く読み直す。
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    if !cfg!(windows) {
        return fs::read(path);
    }
    let step = Duration::from_millis(2);
    retry_transient(Duration::from_millis(40), step, step, || fs::read(path))
}

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
    let bytes = read_file(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// 利用者がパネルから手動で消したセッションのファイルを削除する。すでになければ何もしない。
pub fn delete_session(home: &MarimoHome, provider: Provider, session_id: &str) -> io::Result<bool> {
    let Some(path) = home.session_file(provider, session_id) else {
        return Ok(false);
    };
    let _lock = lock_home(home);
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// フックの入力以外にフックの実行時だけ分かる情報。会話ログの読み取りはロックを取る前に済ませる。
#[derive(Debug, Clone, Default)]
pub struct HookExtras {
    pub origin: Option<Origin>,
    pub transcript: Option<TranscriptTail>,
    pub rollout: Option<RolloutTail>,
    /// Codex の session_index.jsonl か、Hermes のセッションの一覧から読んだタイトル。
    pub title: Option<String>,
    /// Hermes のセッションの一覧から組み立てた、行を押したときに開く URL。
    pub link: Option<String>,
    /// フックを起動したプロセス。
    pub host: Option<HostProcess>,
    /// フックの実行時に会話ログから求めたコンテキストの使用量。状態を変えないイベントでも、行があれば書く。
    pub context: Option<ContextUsage>,
    /// 行を作らない会話。プログラムから始めた Codex の会話は、利用者が見守る相手ではなく、Claude Code から
    /// 任されたものなら任せた側の行がすでに作業中を示しているので、並べない。既にある行は、ここで消す。
    pub hidden: bool,
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
    let transition = if extras.hidden {
        Transition::Delete
    } else {
        let input = HookInput {
            lang: Lang::resolve(home),
            ..input.clone()
        };
        transition(&input, current.as_ref(), now_ms())
    };
    match transition {
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
            if let Some(link) = extras.link.clone() {
                next.link = Some(link);
            }
            if let Some(host) = extras.host {
                next.host = Some(host);
            }
            if let Some(context) = extras.context.clone() {
                next.context = Some(context);
            }
            if matches!(input.provider, Provider::Claude | Provider::Antigravity)
                && input.subagent().is_none()
                && let Some(p) = input.transcript_path.as_deref().filter(|p| !p.is_empty())
            {
                next.transcript_path = Some(p.to_owned());
            }
            // 調べ直すかどうかは保存済みの repo_checked_cwd で決まるので、ロックの内側で調べる。調べるのは
            // cwd が変わったときだけで、セッションごとにふつうは一度なので、ロックを持つ時間はほとんど延びない。
            refresh_repo(input, &mut next, |cwd| repo_name(Path::new(cwd)));
            write_json_atomic(path, &next)
        }
        Transition::Delete => match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        Transition::Nothing => match (current, extras.context.clone()) {
            (Some(mut session), Some(context)) => {
                session.context = Some(context);
                write_json_atomic(path, &session)
            }
            _ => Ok(()),
        },
    }
}

/// 状態を変えずに、コンテキストの使用量だけを書き換える。statusLine と同じく、セッションのファイルは
/// フックだけが作るので、まだ無いセッションや消した後のセッションには書かない。
pub fn apply_context(
    home: &MarimoHome,
    provider: Provider,
    session_id: &str,
    context: ContextUsage,
) -> io::Result<()> {
    update_existing(home, provider, session_id, |s| s.context = Some(context))
}

/// 状態を変えずに、タイトルだけを書き換える。apply_context と同じく、まだ無いセッションには書かない。
pub fn apply_title(
    home: &MarimoHome,
    provider: Provider,
    session_id: &str,
    title: String,
) -> io::Result<()> {
    update_existing(home, provider, session_id, |s| s.title = Some(title))
}

fn update_existing(
    home: &MarimoHome,
    provider: Provider,
    session_id: &str,
    update: impl FnOnce(&mut SessionState),
) -> io::Result<()> {
    let _lock = lock_home(home);
    let (Some(path), Some(mut session)) = (
        home.session_file(provider, session_id),
        read_session(home, provider, session_id),
    ) else {
        return Ok(());
    };
    update(&mut session);
    write_json_atomic(&path, &session)
}

/// 項目名は https://code.claude.com/docs/en/statusline の Available data の節に従う。
pub fn apply_statusline(home: &MarimoHome, input: &Value) -> io::Result<()> {
    let now = now_ms();
    let mut first_err = None;

    if let Err(e) = observe_rate_limits(home, input, now) {
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
        let mut changed = false;
        if let Some(context) = context.filter(|c| context_needs_write(session.context.as_ref(), c))
        {
            session.context = Some(context);
            changed = true;
        }
        if let Some(title) = title.filter(|t| session.title.as_ref() != Some(t)) {
            session.title = Some(title);
            changed = true;
        }
        if changed && let Err(e) = write_json_atomic(&path, &session) {
            first_err.get_or_insert(e);
        }
    }

    first_err.map_or(Ok(()), Err)
}

// statusLine は応答の途中にも数百ミリ秒ごとに呼ばれる。書くたびに監視がスナップショットを読み直してパネルを
// 組み立て直すので、値が変わらない間は書かない。ただし statusLine の値が会話ログからの計算に上書きされないよう、
// 保存した値がこれより古くなったら同じ値でも書き直し、updated_at を STATUSLINE_FRESH_MS の内側に保つ。
const STATUSLINE_REFRESH_MS: u64 = 60 * 1000;

fn context_needs_write(stored: Option<&ContextUsage>, next: &ContextUsage) -> bool {
    let Some(stored) = stored else {
        return true;
    };
    stored.used_percentage != next.used_percentage
        || stored.total_input_tokens != next.total_input_tokens
        || stored.context_window_size != next.context_window_size
        || stored.source != next.source
        || next.updated_at.saturating_sub(stored.updated_at) >= STATUSLINE_REFRESH_MS
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct LastResponse {
    api_ms: u64,
    seen_at: u64,
}

/// statusLine は応答が届いたときのほかに、prompt cache が期限を迎えたときや利用制限のウィンドウが
/// 戻ったときにも呼ばれる（statusline のドキュメントの When it updates の節）。そのときも入力の
/// rate_limits はそのセッションが最後の応答で受け取った値のままなので、放置しているセッションが
/// 古い値を送ってくる。そこで、セッションごとに cost.total_api_duration_ms を覚えておき、前回より
/// 増えたとき、つまり新しい応答が届いたときだけ書く。初めて見たセッションの値はいつの応答のものか
/// 分からないので書かない。この値を持たない入力は、見分けられないので受け取るたびに書く。
/// ロックは内側で取るので、呼び出し側は `lock_home` を持ったまま呼ばない。
fn observe_rate_limits(home: &MarimoHome, input: &Value, now: u64) -> io::Result<()> {
    let limits = rate_limits_from(input, now);
    let id = input.get("session_id").and_then(Value::as_str);
    let api_ms = input
        .get("cost")
        .and_then(|c| c.get("total_api_duration_ms"))
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)));
    let _lock = lock_home(home);
    let (Some(id), Some(api_ms)) = (id, api_ms) else {
        return limits.map_or(Ok(()), |l| write_json_atomic(&home.rate_limits_file(), &l));
    };

    let path = home.statusline_responses_file();
    let mut seen: BTreeMap<String, LastResponse> = read_file(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let previous = seen.get(id).copied();
    let fresh = previous.is_some_and(|p| api_ms > p.api_ms);
    if previous.is_none_or(|p| p.api_ms != api_ms) {
        let stale_before = now.saturating_sub(STALE_SESSION_AGE.as_millis() as u64);
        seen.retain(|_, r| r.seen_at >= stale_before);
        seen.insert(
            id.to_owned(),
            LastResponse {
                api_ms,
                seen_at: now,
            },
        );
        write_json_atomic(&path, &seen)?;
    }
    match limits {
        Some(l) if fresh => write_json_atomic(&home.rate_limits_file(), &l),
        _ => Ok(()),
    }
}

pub fn read_rate_limits(home: &MarimoHome) -> Option<RateLimits> {
    let bytes = read_file(&home.rate_limits_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Claude Code が自分の利用制限の取得結果を `~/.claude.json` の cachedUsageUtilization に残したものを
/// 読む。項目名は Claude Code 2.1.316 が書く形に従う。この項目は文書に無いので、形が違えば None を返す。
/// Claude Code 自身と同じく、ログイン中のアカウントと違うアカウントの値は使わない。
/// updated_at には、marimo が読んだ時刻ではなく Claude Code が取得した時刻を入れる。
pub fn rate_limits_from_claude_config(config: &Value) -> Option<RateLimits> {
    let cached = config.get("cachedUsageUtilization")?;
    let account = config
        .get("oauthAccount")
        .and_then(|a| a.get("accountUuid"))
        .and_then(Value::as_str);
    if cached.get("accountUuid").and_then(Value::as_str) != account {
        return None;
    }
    let utilization = cached.get("utilization")?;
    let window = |key: &str| {
        let w = utilization.get(key)?;
        Some(RateWindow {
            used_percentage: w.get("utilization")?.as_f64()?,
            resets_at: w
                .get("resets_at")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_utc_ms)
                .map(|ms| (ms / 1000) as i64),
        })
    };
    let limits = RateLimits {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
        updated_at: cached.get("fetchedAtMs")?.as_u64()?,
    };
    (limits.five_hour.is_some() || limits.seven_day.is_some()).then_some(limits)
}

/// statusLine が保存した値と、Claude Code が `~/.claude.json` に残した値のうち、新しい時点のものを
/// 残す。保存済みの値と同じか新しい時点の値なら書かない。
pub fn write_rate_limits_if_newer(home: &MarimoHome, limits: &RateLimits) -> io::Result<()> {
    let _lock = lock_home(home);
    if read_rate_limits(home).is_some_and(|stored| stored.updated_at >= limits.updated_at) {
        return Ok(());
    }
    write_json_atomic(&home.rate_limits_file(), limits)
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
    let bytes = read_file(&home.codex_rate_limits_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn write_antigravity_rate_limits(home: &MarimoHome, limits: &RateLimits) -> io::Result<()> {
    let _lock = lock_home(home);
    write_json_atomic(&home.antigravity_rate_limits_file(), limits)
}

pub fn read_antigravity_rate_limits(home: &MarimoHome) -> Option<RateLimits> {
    let bytes = read_file(&home.antigravity_rate_limits_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn read_sessions(home: &MarimoHome) -> Vec<SessionState> {
    fs::read_dir(home.sessions_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| is_session_file_name(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| read_file(&entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<SessionState>(&bytes).ok())
        .collect()
}

pub fn load_snapshot(home: &MarimoHome) -> Snapshot {
    let mut sessions = read_sessions(home);
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
        antigravity_rate_limits: read_antigravity_rate_limits(home),
    }
}

pub fn is_session_file_name(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".json")
}

/// 書き込みの途中でフックが止められると残る一時ファイルを、最後の更新からこれだけ経ったら消す。
/// 書き込みは一瞬で終わるので、これより古い一時ファイルを書いているプロセスは無い。
pub const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);

// write_atomic_with が作る `.<名前>.<プロセス ID>.<通し番号>.tmp` の形の名前だけを一時ファイルとみなす。
// ~/.marimo の直下には利用者や他のツールが置いたファイルもありうるので、形の違う名前には触らない。
fn is_temp_file_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".tmp")) else {
        return false;
    };
    let mut parts = rest.rsplitn(3, '.');
    let digits =
        |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    digits(parts.next()) && digits(parts.next()) && parts.next().is_some_and(|n| !n.is_empty())
}

// `dir` の直下にある、`STALE_TEMP_AGE` より古い一時ファイルを消し、消した数を返す。
fn prune_temp_files(dir: &Path, now: SystemTime) -> io::Result<usize> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    let mut first_err = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_temp_file_name(&entry.file_name().to_string_lossy())
            || !entry.file_type().is_ok_and(|t| t.is_file())
            || !modified_before(&path, now, STALE_TEMP_AGE)
        {
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

/// `max_age` のあいだ更新のないセッションのファイルを消し、`STALE_TEMP_AGE` より古い一時ファイルを ~/.marimo の
/// 直下と sessions の中から消して、消した数を返す。
/// 残したセッションのうち、SubagentStop が届かないまま `AGENT_STALE_MS` を過ぎたサブエージェントを
/// 持つものは、そのサブエージェントを外して書き直す。親の会話もサブエージェントもフックを送らなく
/// なったセッションは、次のフックを待っていると作業中のまま残るからである。同じ理由で、ターンを
/// 動かしていたプロセスが終わった作業中か承認待ちのセッションは、中断されたものとして待機へ戻す。
/// 完了とエラーは、まだ読んでいない結果なので、動いていたサブエージェントだけを外す。プロセスが終わった
/// 待機のセッションは消す。
/// Claude Code で利用者が応答を止めたか承認や質問を断ったまま次のプロンプトを送っていない作業中か
/// 承認待ちのセッションも、会話ログの末尾から見分けて待機へ戻す。
/// バックグラウンドへ移した Claude Code の会話は新しい session_id の行で続くので、元の行は古さによらず消す。
///
/// 消したセッションがまだ動いていても、SessionEnd と SubagentStop 以外のフックが次に届けば
/// `transition` がファイルを作り直すので、行は次の操作で戻る。`transition` がファイルのないセッションに
/// `Transition::Nothing` を返すイベントを増やすときは、この前提を確認し直す。
pub fn prune_stale_sessions(
    home: &MarimoHome,
    now_ms: u64,
    max_age: Duration,
) -> io::Result<usize> {
    let now = UNIX_EPOCH + Duration::from_millis(now_ms);
    let mut removed = 0;
    let mut first_err = None;
    for dir in [home.root().to_path_buf(), home.sessions_dir()] {
        match prune_temp_files(&dir, now) {
            Ok(n) => removed += n,
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    let entries = match fs::read_dir(home.sessions_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return first_err.map_or(Ok(removed), Err),
        Err(e) => return Err(e),
    };
    let lang = Lang::resolve(home);
    let max_age_ms = u64::try_from(max_age.as_millis()).unwrap_or(u64::MAX);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        // 会話ログとプロセスを調べる間はロックを持たない。フックはロックを 300 ミリ秒しか待たずに書くので、
        // 持ったまま時間をかけると、その間にフックが書いた新しい状態を古い判定で消しうる。判定のあとでロックを取り、
        // ファイルが読んだときのままであることを確認してから書き直すか消す。
        if !is_session_file_name(&name) {
            continue;
        }
        let (read, stale, next) = {
            let read = read_file(&path).ok();
            let session = read
                .as_deref()
                .and_then(|bytes| serde_json::from_slice::<SessionState>(bytes).ok());
            // updated_at が 0 のファイルは、この項目を持たない古い形式で書かれたものなので、
            // 更新時刻が分からないものとして読めないファイルと同じく mtime で判定する。
            let turn = session.as_ref().map_or(LastTurn::Other, last_turn);
            let stale = match session.as_ref().map(|s| s.updated_at).filter(|&t| t != 0) {
                Some(t) => t.saturating_add(max_age_ms) < now_ms,
                None => modified_before(&path, now, max_age),
            } || turn == LastTurn::Moved
                || session.as_ref().is_some_and(abandoned);
            let next = session.filter(|_| !stale).and_then(|s| {
                release_orphan(&s, lang, now_ms)
                    .or_else(|| release_interrupted(&s, turn, lang, now_ms))
                    .or_else(|| expire_agents(&s, lang, now_ms))
            });
            (read, stale, next)
        };
        if !stale && next.is_none() {
            continue;
        }
        // 急ぐ処理ではないので、ロックが取れなければ次の機会に回す。
        let lock = lock_home(home);
        if !lock.is_held() || read_file(&path).ok() != read {
            continue;
        }
        if let Some(next) = next {
            if let Err(e) = write_json_atomic(&path, &next) {
                first_err.get_or_insert(e);
            }
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

// 会話ログが読めないときは、移ったか止めたか分からないので何もしない。
fn last_turn(session: &SessionState) -> LastTurn {
    session
        .transcript_path
        .as_deref()
        .filter(|_| session.provider == Provider::Claude)
        .and_then(|p| transcript::last_turn(Path::new(p)).ok())
        .unwrap_or(LastTurn::Other)
}

fn release_orphan(session: &SessionState, lang: Lang, now_ms: u64) -> Option<SessionState> {
    let busy = matches!(session.status, Status::Working | Status::Waiting);
    if !busy || !session.host.as_ref().is_some_and(process::has_exited) {
        return None;
    }
    // サブエージェントも同じプロセスの中で動くので、一緒に終わっている。
    interrupt(session, |_| false, lang, now_ms)
}

// Claude Code がバックグラウンドで用意しておく予備のプロセス（claude bg-spare）は、自分の session_id で SessionStart を
// 送るので待機の行を作り、使われないまま終わっても SessionEnd を送らない。待機の行はパネルに出ないので、
// まだ動いているセッションのものを消しても、次のフックで作り直されるまで見え方は変わらない。
fn abandoned(session: &SessionState) -> bool {
    session.status == Status::Idle && session.host.as_ref().is_some_and(process::has_exited)
}

// 利用者が応答を止めると Stop は発火せず、デスクトップアプリは待機の通知（idle_prompt）も送らない。
// 止めた後に次のプロンプトを送っていれば、会話ログがまだ追いついていないだけなので触らない。
// バックグラウンドで動くサブエージェントは止めた後も動き続けてフックを送るので、止めた後に動きのあったものは残す。
fn release_interrupted(
    session: &SessionState,
    turn: LastTurn,
    lang: Lang,
    now_ms: u64,
) -> Option<SessionState> {
    let LastTurn::Interrupted { at_ms } = turn else {
        return None;
    };
    let busy = matches!(session.status, Status::Working | Status::Waiting);
    if session.provider != Provider::Claude
        || !busy
        || session.turn_started_at.is_some_and(|t| t > at_ms)
    {
        return None;
    }
    interrupt(session, |a| a.last_seen > at_ms, lang, now_ms).filter(|next| next != session)
}

/// agy は承認を求めている間に呼ぶフックを持たず、利用者が承認を断ったときや、モデルの応答を待つ間に止めたときも
/// フックを出さずにターンを止めるので、作業中か承認待ちの Antigravity の行について、会話の DB の最後の step と
/// 最後のフックからの時間を読んで行に書く（`antigravity::step_event` を参照）。アプリが短い間隔で呼ぶ。
/// `read_step` は DB の場所を受け取り、最後の step を読む。読めなければ None を返す。
pub fn refresh_antigravity_steps(
    home: &MarimoHome,
    now_ms: u64,
    read_step: impl Fn(&Path) -> Option<antigravity::Step>,
) -> io::Result<()> {
    let entries = match fs::read_dir(home.sessions_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut first_err = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("antigravity-") || !is_session_file_name(&name) {
            continue;
        }
        let path = entry.path();
        let Ok(read) = read_file(&path) else { continue };
        let Ok(session) = serde_json::from_slice::<SessionState>(&read) else {
            continue;
        };
        let busy = matches!(session.status, Status::Working | Status::Waiting);
        let Some(transcript) = session.transcript_path.as_deref().filter(|_| busy) else {
            continue;
        };
        let events: Vec<HookInput> = std::iter::once(None)
            .chain(session.agents.keys().map(|k| Some(k.as_str())))
            .filter_map(|agent| {
                let id = agent.unwrap_or(&session.session_id);
                let step = antigravity::conversation_db(Path::new(transcript), id)
                    .and_then(|db| read_step(&db));
                antigravity::step_event(&session, agent, step.as_ref(), now_ms)
            })
            .collect();
        if events.is_empty() {
            continue;
        }
        // prune_stale_sessions と同じく、DB を読む間はロックを持たず、書く前にファイルが読んだときのままかを確認する。
        let lock = lock_home(home);
        if !lock.is_held() || read_file(&path).ok().as_deref() != Some(read.as_slice()) {
            continue;
        }
        let lang = Lang::resolve(home);
        let mut next = session;
        for event in &events {
            let event = &HookInput {
                lang,
                ..event.clone()
            };
            if let Transition::Write(s) = transition(event, Some(&next), now_ms) {
                next = *s;
            }
        }
        if let Err(e) = write_json_atomic(&path, &next) {
            first_err.get_or_insert(e);
        }
    }
    first_err.map_or(Ok(()), Err)
}

fn interrupt(
    session: &SessionState,
    keep: impl Fn(&AgentRun) -> bool,
    lang: Lang,
    now_ms: u64,
) -> Option<SessionState> {
    // 完了とエラーは、まだ読んでいない結果なので残す。
    if matches!(
        session.own_status.unwrap_or(session.status),
        Status::Done | Status::Error
    ) {
        return Some(without_agents(session, keep, lang, now_ms));
    }
    let input = HookInput {
        provider: session.provider,
        session_id: session.session_id.clone(),
        hook_event_name: "Interrupt".to_owned(),
        lang,
        ..HookInput::default()
    };
    let Transition::Write(next) = transition(&input, Some(session), now_ms) else {
        return None;
    };
    // Interrupt はサブエージェントをすべて外すので、止めた後も動いているものを戻し、元の表示から
    // 表示用の値を組み立て直す。status と status_since を元に戻しておくのは、表示が変わらないときに
    // その状態になった時刻を動かさないためである。
    let restored = SessionState {
        agents: session
            .agents
            .iter()
            .filter(|(_, a)| keep(a))
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect(),
        status: session.status,
        status_since: session.status_since,
        ..*next
    };
    // updated_at は変えない。expire_agents と同じく、セッションを消すまでの 24 時間を最後のフックから
    // 数え続けるためである。
    Some(SessionState {
        updated_at: session.updated_at,
        ..without_agents(&restored, |_| true, lang, now_ms)
    })
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
            let bytes = read_file(&path).unwrap();
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

        // 値が同じ間は書き直さない。書き直すと context.updated_at が変わる。
        let path = home.session_file(Provider::Claude, "s1").unwrap();
        let written = fs::read(&path).unwrap();
        thread::sleep(Duration::from_millis(5));
        apply_statusline(&home, &status).unwrap();
        assert_eq!(fs::read(&path).unwrap(), written);

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
    fn statusline_keeps_rate_limits_only_from_new_responses() {
        let (_d, home) = home();
        let five_hour = |home: &MarimoHome| {
            read_rate_limits(home).map(|rl| rl.five_hour.unwrap().used_percentage)
        };
        // (session_id, total_api_duration_ms, 5 時間の使用率, 保存されているはずの値)
        let steps = [
            ("s1", 0, None, None),
            ("s1", 100, Some(10.0), Some(10.0)),
            // 初めて見たセッションの値は、いつの応答のものか分からない。
            ("s2", 500, Some(5.0), Some(10.0)),
            // 応答が増えていない呼び出しは、prompt cache の期限などで再び呼ばれただけである。
            ("s2", 500, Some(5.0), Some(10.0)),
            ("s2", 600, Some(12.0), Some(12.0)),
            ("s1", 100, Some(10.0), Some(12.0)),
        ];
        for (i, (id, api_ms, used, expected)) in steps.into_iter().enumerate() {
            let mut status = json!({"session_id": id, "cost": {"total_api_duration_ms": api_ms}});
            if let Some(used) = used {
                status["rate_limits"] = json!({"five_hour": {"used_percentage": used}});
            }
            apply_statusline(&home, &status).unwrap();
            assert_eq!(five_hour(&home), expected, "step {i}");
        }
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
        apply_hook(&home, &stop, &with_title(Some("デスクトップのタイトル"))).unwrap();
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .title
                .as_deref(),
            Some("デスクトップのタイトル")
        );
        apply_hook(&home, &stop, &with_title(None)).unwrap();
        assert_eq!(
            read_session(&home, Provider::Claude, "s1")
                .unwrap()
                .title
                .as_deref(),
            Some("デスクトップのタイトル")
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
            title: Some("Codex のタイトル".to_owned()),
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
        assert_eq!(c.title.as_deref(), Some("Codex のタイトル"));
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
            json!({"session_id":"s1","hook_event_name":"SessionEnd","reason":"prompt_input_exit"}),
            &HookExtras::default(),
        );
        assert!(read_session(&home, Provider::Codex, "s1").is_none());
        assert!(read_session(&home, Provider::Claude, "s1").is_some());
    }

    #[test]
    fn hidden_sessions_drop_their_row_but_keep_rate_limits() {
        let (_d, home) = home();
        let mut input: HookInput = serde_json::from_value(
            json!({"session_id":"s1","hook_event_name":"SessionStart","cwd":"/w"}),
        )
        .unwrap();
        input.provider = Provider::Codex;
        apply_hook(&home, &input, &HookExtras::default()).unwrap();
        assert!(read_session(&home, Provider::Codex, "s1").is_some());

        input.hook_event_name = "PostToolUse".to_owned();
        let extras = HookExtras {
            rollout: Some(RolloutTail {
                usage: None,
                rate_limits: Some(codex_limits(10, 5.0)),
            }),
            hidden: true,
            ..HookExtras::default()
        };
        apply_hook(&home, &input, &extras).unwrap();
        assert!(read_session(&home, Provider::Codex, "s1").is_none());
        assert_eq!(read_codex_rate_limits(&home), Some(codex_limits(10, 5.0)));
    }

    // 状態を変えるイベントでも変えないイベントでも、フックが持ってきたコンテキストの使用量を書く。
    #[test]
    fn hook_context_is_written_whether_or_not_the_state_changes() {
        let (_d, home) = home();
        let usage = |tokens| ContextUsage {
            used_percentage: None,
            total_input_tokens: Some(tokens),
            context_window_size: None,
            updated_at: 1,
            source: None,
        };
        for (event, tokens) in [("SessionStart", 100), ("PreInvocation", 200)] {
            let mut input: HookInput = serde_json::from_value(
                json!({"session_id":"c1","hook_event_name":event,"cwd":"/w"}),
            )
            .unwrap();
            input.provider = Provider::Antigravity;
            let extras = HookExtras {
                context: Some(usage(tokens)),
                ..HookExtras::default()
            };
            apply_hook(&home, &input, &extras).unwrap();
            let session = read_session(&home, Provider::Antigravity, "c1").unwrap();
            assert_eq!(session.context, Some(usage(tokens)), "{event}");
        }
    }

    #[test]
    fn reads_claude_code_cached_usage() {
        let config = |cached_account: &str| {
            json!({
                "oauthAccount": {"accountUuid": "a1"},
                "cachedUsageUtilization": {
                    "fetchedAtMs": 1_793_606_269_733_u64,
                    "accountUuid": cached_account,
                    "utilization": {
                        "five_hour": {"utilization": 46, "resets_at": "2026-11-02T08:10:00.558884+00:00"},
                        "seven_day": null,
                        "seven_day_opus": null}}})
        };
        assert_eq!(
            rate_limits_from_claude_config(&config("a1")),
            Some(RateLimits {
                five_hour: Some(RateWindow {
                    used_percentage: 46.0,
                    resets_at: Some(1_793_607_000)
                }),
                seven_day: None,
                updated_at: 1_793_606_269_733,
            })
        );
        assert_eq!(rate_limits_from_claude_config(&config("other")), None);
    }

    #[test]
    fn claude_rate_limits_keep_the_newest_observation() {
        let (_d, home) = home();
        let limits = |updated_at: u64, used: f64| RateLimits {
            five_hour: Some(RateWindow {
                used_percentage: used,
                resets_at: None,
            }),
            seven_day: None,
            updated_at,
        };
        write_rate_limits_if_newer(&home, &limits(100, 10.0)).unwrap();
        write_rate_limits_if_newer(&home, &limits(100, 99.0)).unwrap();
        assert_eq!(read_rate_limits(&home), Some(limits(100, 10.0)));
        write_rate_limits_if_newer(&home, &limits(200, 20.0)).unwrap();
        assert_eq!(read_rate_limits(&home), Some(limits(200, 20.0)));
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
        put_file(&dir.join(".b.json.1.3.tmp"), b"{", NOW_MS - HOUR_MS / 6);
        put_file(&dir.join("notes.txt"), b"x", NOW_MS - 48 * HOUR_MS);
        // ~/.marimo の直下の一時ファイルも消すが、形の違う名前には触らない。
        let root = home.root();
        put_file(
            &root.join(".rate_limits.json.7.0.tmp"),
            b"{",
            NOW_MS - 2 * HOUR_MS,
        );
        put_file(&root.join(".keep.tmp"), b"x", NOW_MS - 48 * HOUR_MS);
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
        assert_eq!(removed, 6);
        assert!(!root.join(".rate_limits.json.7.0.tmp").exists());
        assert!(root.join(".keep.tmp").exists());
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
    fn prune_releases_busy_sessions_whose_process_ended() {
        let (_d, home) = home();
        let alive = process::host().expect("the test runner has a parent");
        let ended = HostProcess {
            created: alive.created + 1,
            ..alive
        };
        let agent = AgentRun {
            agent_type: None,
            started_at: NOW_MS - HOUR_MS,
            last_seen: NOW_MS - HOUR_MS,
            pending: None,
        };
        // (id, 親の会話の状態, 表示の状態, プロセス, 見回りの後の表示の状態。消えたら None)
        let cases = [
            (
                "working-alive",
                Status::Working,
                Status::Working,
                Some(alive),
                Some(Status::Working),
            ),
            (
                "working-ended",
                Status::Working,
                Status::Working,
                Some(ended),
                Some(Status::Idle),
            ),
            (
                "waiting-ended",
                Status::Waiting,
                Status::Waiting,
                Some(ended),
                Some(Status::Idle),
            ),
            (
                "done-ended",
                Status::Done,
                Status::Done,
                Some(ended),
                Some(Status::Done),
            ),
            (
                "agents-ended",
                Status::Done,
                Status::Working,
                Some(ended),
                Some(Status::Done),
            ),
            (
                "error-ended",
                Status::Error,
                Status::Working,
                Some(ended),
                Some(Status::Error),
            ),
            ("idle-ended", Status::Idle, Status::Idle, Some(ended), None),
            (
                "idle-alive",
                Status::Idle,
                Status::Idle,
                Some(alive),
                Some(Status::Idle),
            ),
            (
                "working-unknown",
                Status::Working,
                Status::Working,
                None,
                Some(Status::Working),
            ),
        ];
        for (id, own, status, host, _) in &cases {
            let session = SessionState {
                provider: Provider::Hermes,
                status: *status,
                own_status: Some(*own),
                updated_at: NOW_MS - HOUR_MS,
                host: *host,
                agents: (own != status)
                    .then(|| ("a1".to_owned(), agent.clone()))
                    .into_iter()
                    .collect(),
                ..SessionState::new(*id)
            };
            let path = home.session_file(Provider::Hermes, id).unwrap();
            write_json_atomic(&path, &session).unwrap();
        }
        prune_stale_sessions(&home, NOW_MS, STALE_SESSION_AGE).unwrap();
        for (id, _, _, _, expected) in cases {
            let after = read_session(&home, Provider::Hermes, id);
            assert_eq!(after.as_ref().map(|s| s.status), expected, "{id}");
            if let Some(after) = after {
                assert_eq!(
                    (after.updated_at, after.agents.len()),
                    (NOW_MS - HOUR_MS, 0),
                    "{id}"
                );
            }
        }
    }

    #[test]
    fn prune_removes_sessions_continued_in_the_background() {
        let (dir, home) = home();
        for (id, last) in [("moved", "continued-in"), ("here", "user")] {
            let log = dir.path().join(format!("{id}.jsonl"));
            fs::write(&log, format!("{}\n", json!({"type": last}))).unwrap();
            hook(
                &home,
                json!({"session_id": id, "hook_event_name": "UserPromptSubmit", "prompt": "x",
                       "transcript_path": log}),
            );
        }
        assert_eq!(
            prune_stale_sessions(&home, now_ms(), STALE_SESSION_AGE).unwrap(),
            1
        );
        assert!(read_session(&home, Provider::Claude, "moved").is_none());
        assert!(read_session(&home, Provider::Claude, "here").is_some());
    }

    #[test]
    fn prune_releases_sessions_interrupted_by_the_user() {
        let (dir, home) = home();
        let at_ms = NOW_MS - 5 * 60_000;
        let marker = json!({"type": "user", "timestamp": rfc3339_utc(at_ms), "message": {
            "role": "user",
            "content": [{"type": "text", "text": "[Request interrupted by user for tool use]"}]
        }});
        let reply = json!({"type": "assistant", "message": {"content": []}});
        let agent = |last_seen| AgentRun {
            agent_type: None,
            started_at: at_ms - 60_000,
            last_seen,
            pending: None,
        };
        // (id, 表示の状態, 会話ログの最後の行, ターンの始まり, サブエージェントの最後の動き, 見回りの後の表示の状態)
        let cases = [
            (
                "rejected",
                Status::Waiting,
                &marker,
                None,
                None,
                Status::Idle,
            ),
            (
                "stopped",
                Status::Working,
                &marker,
                Some(at_ms - 9),
                Some(at_ms - 5),
                Status::Idle,
            ),
            (
                "background",
                Status::Working,
                &marker,
                None,
                Some(at_ms + 1),
                Status::Working,
            ),
            (
                "next-prompt",
                Status::Working,
                &marker,
                Some(at_ms + 1),
                None,
                Status::Working,
            ),
            (
                "answered",
                Status::Working,
                &reply,
                None,
                None,
                Status::Working,
            ),
        ];
        let updated_at = |seen: Option<u64>| seen.unwrap_or(0).max(at_ms - 60_000);
        for (id, status, last, turn, seen, _) in &cases {
            let log = dir.path().join(format!("{id}.jsonl"));
            fs::write(&log, format!("{last}\n")).unwrap();
            let session = SessionState {
                status: *status,
                own_status: Some(*status),
                updated_at: updated_at(*seen),
                turn_started_at: *turn,
                transcript_path: Some(log.to_string_lossy().into_owned()),
                agents: seen.iter().map(|&t| ("a1".to_owned(), agent(t))).collect(),
                ..SessionState::new(*id)
            };
            write_json_atomic(&home.session_file(Provider::Claude, id).unwrap(), &session).unwrap();
        }
        assert_eq!(
            prune_stale_sessions(&home, NOW_MS, STALE_SESSION_AGE).unwrap(),
            0
        );
        for (id, _, _, _, seen, expected) in cases {
            let after = read_session(&home, Provider::Claude, id).unwrap();
            assert_eq!(
                (after.status, after.updated_at),
                (expected, updated_at(seen)),
                "{id}"
            );
        }
        // 止めた後も動いているサブエージェントだけを残し、親の会話は待機へ戻す。
        let after = read_session(&home, Provider::Claude, "background").unwrap();
        assert_eq!(
            (after.own_status, after.agents.len()),
            (Some(Status::Idle), 1)
        );
        let after = read_session(&home, Provider::Claude, "stopped").unwrap();
        assert!(after.agents.is_empty());
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

    #[test]
    fn refresh_marks_antigravity_permission_prompts() {
        let (_dir, home) = home();
        let id = "a4476239-3e70-42fc-accd-2a5220efcdd2";
        let transcript = format!("/agy/brain/{id}/.system_generated/logs/transcript_full.jsonl");
        let input = HookInput {
            provider: Provider::Antigravity,
            session_id: id.to_owned(),
            hook_event_name: "PreToolUse".to_owned(),
            tool_name: Some("Bash".to_owned()),
            tool_input: Some(json!({"command": "echo hi"})),
            transcript_path: Some(transcript.clone()),
            ..HookInput::default()
        };
        apply_hook(&home, &input, &HookExtras::default()).unwrap();
        let db = Path::new("/agy/conversations").join(format!("{id}.db"));
        let waiting = |path: &Path| {
            (path == db).then(|| antigravity::Step {
                status: 9,
                ..antigravity::Step::default()
            })
        };
        refresh_antigravity_steps(&home, NOW_MS, waiting).unwrap();
        let s = read_session(&home, Provider::Antigravity, id).unwrap();
        assert_eq!(s.status, Status::Waiting);
        assert_eq!(s.transcript_path.as_deref(), Some(transcript.as_str()));
        assert_eq!(
            s.activity.map(|a| a.detail),
            Some(Some("echo hi".to_owned()))
        );
    }

    #[test]
    fn delete_session_removes_existing_file_and_ignores_missing() {
        let (_dir, home) = home();
        let input = HookInput {
            provider: Provider::Claude,
            session_id: "s-del".to_owned(),
            hook_event_name: "UserPromptSubmit".to_owned(),
            ..HookInput::default()
        };
        apply_hook(&home, &input, &HookExtras::default()).unwrap();
        assert!(read_session(&home, Provider::Claude, "s-del").is_some());
        assert!(delete_session(&home, Provider::Claude, "s-del").unwrap());
        assert!(read_session(&home, Provider::Claude, "s-del").is_none());
        assert!(!delete_session(&home, Provider::Claude, "s-del").unwrap());
    }
}
