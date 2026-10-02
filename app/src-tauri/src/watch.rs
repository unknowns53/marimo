use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use marimo_core::time::now_ms;
use marimo_core::{MarimoHome, store};
use notify::{Event, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

use crate::{scale, tray};

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
        loop {
            // 手動観測の期限はファイルが変わらないまま切れるので、期限内の観測があればその時刻にも起きる。
            let woke = match marimo_core::cloud::expires_at(&home, now_ms()) {
                Some(at) => {
                    match rx.recv_timeout(Duration::from_millis(at.saturating_sub(now_ms()))) {
                        Ok(()) => true,
                        Err(RecvTimeoutError::Timeout) => false,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match rx.recv() {
                    Ok(()) => true,
                    Err(_) => return,
                },
            };
            if woke {
                let first = Instant::now();
                loop {
                    match rx.recv_timeout(QUIET) {
                        Ok(()) if first.elapsed() < MAX_DELAY => continue,
                        Ok(()) | Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
            let snapshot = scale::snapshot(&home);
            if previous.as_ref() == Some(&snapshot) {
                continue;
            }
            previous = Some(snapshot.clone());
            tray::show_status(&app, snapshot.aggregate);
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

// macOS の FSEvents は /var を /private/var のように実体のパスで報告するので、
// 絶対パスどうしを比べず、ファイル名と親ディレクトリ名だけで判定する。
fn is_relevant(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if matches!(
        name,
        "rate_limits.json" | "codex_rate_limits.json" | "cloud_snapshot.json" | "display.json"
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
        assert!(is_relevant(Path::new("/h/cloud_snapshot.json")));
        assert!(is_relevant(Path::new("/h/display.json")));
        assert!(!is_relevant(Path::new("/h/sessions/.abc.json.1.2.tmp")));
        assert!(!is_relevant(Path::new("/h/window.json")));
        assert!(!is_relevant(Path::new("/h/logs/record.jsonl")));
        assert!(!is_relevant(Path::new("/h/logs/record-2026-01-01.jsonl")));
        assert!(!is_relevant(Path::new("/h/.lock")));
    }
}
