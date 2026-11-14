use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use marimo_core::time::now_ms;
use marimo_core::{MarimoHome, antigravity, store};
use notify::{Event, RecursiveMode, Watcher};
use rusqlite::{Connection, OpenFlags};
use tauri::{AppHandle, Emitter};

use crate::tray;

pub const SNAPSHOT_EVENT: &str = "snapshot";

// 並列のツール呼び出しでは同じ瞬間にフックがいくつも書くので、静かになるまで待って
// 一度にまとめて送る。止まずに書かれ続けても、最大の遅れで打ち切って送る。
const QUIET: Duration = Duration::from_millis(120);
const MAX_DELAY: Duration = Duration::from_millis(500);

const PRUNE_INTERVAL: Duration = Duration::from_secs(10 * 60);

pub fn spawn(app: AppHandle, home: MarimoHome) {
    thread::spawn(move || {
        let (tx, rx) = mpsc::channel::<()>();
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res
                && event.paths.iter().any(|p| is_relevant(p))
            {
                let _ = tx.send(());
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("marimo: cannot start file watcher: {e}");
                return;
            }
        };
        for (dir, what) in [
            (home.sessions_dir(), "sessions"),
            (home.root().to_path_buf(), "home"),
        ] {
            if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
                eprintln!("marimo: cannot watch {what} ({}): {e}", dir.display());
            }
        }

        let mut previous = None;
        while rx.recv().is_ok() {
            let first = Instant::now();
            loop {
                match rx.recv_timeout(QUIET) {
                    Ok(()) if first.elapsed() < MAX_DELAY => continue,
                    Ok(()) | Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            let snapshot = store::load_snapshot(&home);
            if previous.as_ref() == Some(&snapshot) {
                continue;
            }
            previous = Some(snapshot.clone());
            tray::show_status(&app, &snapshot);
            let _ = app.emit(SNAPSHOT_EVENT, snapshot);
        }
    });
}

// 消したファイルや書き直したファイルは監視が受け取り、スナップショットを送り直すので、
// ここから画面へ知らせる必要はない。
pub fn spawn_pruner(home: MarimoHome) {
    thread::spawn(move || {
        loop {
            if let Err(e) = store::prune_stale_sessions(&home, now_ms(), store::STALE_SESSION_AGE) {
                eprintln!("marimo: cannot remove stale sessions: {e}");
            }
            thread::sleep(PRUNE_INTERVAL);
        }
    });
}

// agy の承認待ちはフックで届かないので、会話の DB を読みに行く。承認の画面が出てから行が変わるまでの遅れを
// この間隔に収める。DB を開くのは、作業中か承認待ちの Antigravity の行があるときだけである。
const ANTIGRAVITY_STEP_INTERVAL: Duration = Duration::from_secs(1);

pub fn spawn_antigravity_steps(home: MarimoHome) {
    thread::spawn(move || {
        loop {
            if let Err(e) = store::refresh_antigravity_steps(&home, now_ms(), read_last_step) {
                eprintln!("marimo: cannot refresh Antigravity steps: {e}");
            }
            thread::sleep(ANTIGRAVITY_STEP_INTERVAL);
        }
    });
}

// agy が書いている最中の DB を読むので、読み取り専用で開き、ファイルを作らない。agy は会話を作るときに DB の表を
// 作り、そのときほかの接続が開いていると「database is locked」で SQLite への保存をやめて別の形式へ切り替える。
// そこで、agy が WAL で開いている印の -wal のファイルがあるときだけ開く。agy が動いていない会話の DB も、
// このファイルが無いので開かない。
// 一つの応答で複数のツールを呼ぶと、承認を待つ step の後ろに順番を待つ step が並びうるので、最後のモデルの応答
// （step_type が 15 の PLANNER_RESPONSE）より後の step のうち、承認を待つ（status が 9 の）ものを先に選ぶ。
// 前のターンで承認を待ったまま agy が終わった step は、この範囲に入らない。
fn read_last_step(db: &Path) -> Option<antigravity::Step> {
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    if !Path::new(&wal).exists() {
        return None;
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(db, flags).ok()?;
    let (status, metadata) = conn
        .query_row(
            "SELECT status, metadata FROM steps \
             WHERE idx >= (SELECT COALESCE(MAX(idx), 0) FROM steps WHERE step_type = 15) \
             ORDER BY status = 9 DESC, idx DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .ok()?;
    Some(antigravity::step(
        status,
        metadata.as_deref().unwrap_or_default(),
    ))
}

// macOS の FSEvents は /var を /private/var のように実体のパスで報告するので、
// 絶対パスどうしを比べず、ファイル名と親ディレクトリ名だけで判定する。
fn is_relevant(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if matches!(
        name,
        "rate_limits.json" | "codex_rate_limits.json" | "antigravity_rate_limits.json"
    ) {
        return true;
    }
    let in_sessions = path
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "sessions");
    in_sessions && store::is_session_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevance_filter() {
        assert!(is_relevant(Path::new("/h/sessions/abc.json")));
        assert!(is_relevant(Path::new("/h/rate_limits.json")));
        assert!(is_relevant(Path::new("/h/codex_rate_limits.json")));
        assert!(is_relevant(Path::new("/h/antigravity_rate_limits.json")));
        assert!(!is_relevant(Path::new("/h/display.json")));
        assert!(!is_relevant(Path::new("/h/sessions/.abc.json.1.2.tmp")));
        assert!(!is_relevant(Path::new("/h/window.json")));
        assert!(!is_relevant(Path::new("/h/logs/record.jsonl")));
        assert!(!is_relevant(Path::new("/h/logs/record-2026-01-01.jsonl")));
        assert!(!is_relevant(Path::new("/h/.lock")));
    }
}
