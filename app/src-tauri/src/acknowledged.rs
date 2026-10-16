use std::collections::HashSet;
use std::fs;
use std::io;

use marimo_core::{MarimoHome, SessionState, Snapshot, Status, aggregate, store};
use serde::Serialize;

// フロントエンドは続いているきっかけの分しか残さないので、ふつうは数件で済む。壊れた値や
// 手で書き換えた値で読み込みが重くならないよう、上限を超えた分は捨てる。
const MAX_KEYS: usize = 1000;

/// 読めない、または壊れているときは空を返し、すべて未読として扱わせる。
pub fn load(home: &MarimoHome) -> Vec<String> {
    let mut keys = fs::read(home.acknowledged_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default();
    keys.truncate(MAX_KEYS);
    keys
}

pub fn save(home: &MarimoHome, keys: &[String]) -> io::Result<()> {
    let kept = &keys[..keys.len().min(MAX_KEYS)];
    store::write_json_durable(&home.acknowledged_file(), &kept)
}

/// フロントエンドの triggerKey（app/src/acknowledged.ts）と同じ形の鍵。既読の集合はフロントエンドが
/// この形で作って送ってくるので、一文字でも違うと既読を見落とす。両方のテストで同じ文字列を確かめている。
pub fn trigger_key(s: &SessionState) -> String {
    let id = if s.provider == marimo_core::Provider::Claude {
        s.session_id.clone()
    } else {
        format!("{}:{}", serde_name(s.provider), s.session_id)
    };
    format!("{id}:{}:{}", serde_name(s.status), s.status_since)
}

// フロントエンドが受け取る JSON と同じ名前にするため、serde の名前を使う。
fn serde_name<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// 既読にした完了とエラーを待機とみなした集約。フロントエンドの withAcknowledged と同じ規則で、
/// メニューバーのアイコンが、画面では畳んだエラーを出し続けないようにする。
pub fn effective_aggregate(snapshot: &Snapshot, read: &HashSet<String>) -> Status {
    aggregate(snapshot.sessions.iter().map(|s| {
        let markable = matches!(s.status, Status::Done | Status::Error);
        if markable && read.contains(&trigger_key(s)) {
            Status::Idle
        } else {
            s.status
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_treats_broken_files_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        assert!(load(&home).is_empty());
        let keys = vec!["a:done:10".to_owned(), "b:waiting:20".to_owned()];
        save(&home, &keys).unwrap();
        assert_eq!(load(&home), keys);
        fs::write(home.acknowledged_file(), "{broken").unwrap();
        assert!(load(&home).is_empty());
    }

    fn session(
        provider: marimo_core::Provider,
        id: &str,
        status: Status,
        since: u64,
    ) -> SessionState {
        SessionState {
            provider,
            status,
            status_since: since,
            ..SessionState::new(id)
        }
    }

    // app/src/acknowledged.test.ts の triggerKey のテストと同じ文字列である。
    #[test]
    fn trigger_keys_match_the_frontend() {
        use marimo_core::Provider::{Claude, Opencode};
        assert_eq!(
            trigger_key(&session(Claude, "abc", Status::Done, 5)),
            "abc:done:5"
        );
        assert_eq!(
            trigger_key(&session(Opencode, "ses_x", Status::Error, 1792165830895)),
            "opencode:ses_x:error:1792165830895"
        );
    }

    #[test]
    fn read_completions_and_errors_count_as_idle() {
        use marimo_core::Provider::{Claude, Opencode};
        let error = session(Opencode, "e", Status::Error, 3);
        let done = session(Claude, "d", Status::Done, 2);
        let waiting = session(Claude, "w", Status::Waiting, 1);
        let snapshot = |sessions: Vec<SessionState>| Snapshot {
            aggregate: Status::Idle,
            sessions,
            rate_limits: None,
            codex_rate_limits: None,
        };
        let read: HashSet<String> = [&error, &done, &waiting]
            .map(trigger_key)
            .into_iter()
            .collect();
        for (sessions, keys, want) in [
            (vec![error.clone()], HashSet::new(), Status::Error),
            (
                vec![error.clone(), done.clone()],
                read.clone(),
                Status::Idle,
            ),
            // 承認待ちは見ただけでは畳まないので、既読の集合にあっても下げない。
            (
                vec![error.clone(), waiting.clone()],
                read.clone(),
                Status::Waiting,
            ),
        ] {
            assert_eq!(effective_aggregate(&snapshot(sessions), &keys), want);
        }
    }
}
