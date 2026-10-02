use std::{collections::HashSet, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Activity, ActivityKind, MarimoHome, Provider, SessionState, Status, store};

// 取得が途絶えても、観測時の「作業中」を現在の状態として残し続けない。期限を過ぎた行は消さずに
// 待機として残し、立ち絵とアイコンの集計に効かないようにする。
pub const TTL_MS: u64 = 120_000;
const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub thread_id: String,
    pub observed_at: u64,
    pub expires_at: u64,
    pub expired: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudSnapshot {
    observed_at: u64,
    threads: Vec<Thread>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Thread {
    id: String,
    title: String,
    cwd: Option<String>,
    status: CloudStatus,
    #[serde(rename = "hostId")]
    host_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CloudStatus {
    Active,
    Idle,
    NeedsInput,
    Completed,
    Error,
    Interrupted,
}

impl CloudStatus {
    fn status(&self) -> Status {
        match self {
            Self::Active => Status::Working,
            Self::NeedsInput => Status::Waiting,
            Self::Completed => Status::Done,
            Self::Error => Status::Error,
            // idle は完了を保証せず、中断も成功としては扱えない。
            Self::Idle | Self::Interrupted => Status::Idle,
        }
    }
}

pub fn file(home: &MarimoHome) -> PathBuf {
    home.root().join("cloud_snapshot.json")
}

fn fresh(at: u64, now: u64) -> bool {
    at != 0 && at <= now && now - at < TTL_MS
}

fn validate(snapshot: &CloudSnapshot) -> Result<(), String> {
    if snapshot.observed_at == 0 {
        return Err("observed_at is required".into());
    }
    if snapshot.threads.len() > 100 {
        return Err("at most 100 cloud threads are accepted".into());
    }
    let mut ids = HashSet::new();
    for t in &snapshot.threads {
        if t.host_id != "durable"
            || t.id.is_empty()
            || t.id.len() > 128
            || !t.id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || !ids.insert(&t.id)
            || t.title.chars().count() > 200
            || t.title.chars().any(char::is_control)
            || t.cwd
                .as_ref()
                .is_some_and(|s| s.len() > 4096 || s.chars().any(char::is_control))
        {
            return Err(
                "invalid cloud metadata: durable host, unique ID and bounded title/cwd required"
                    .into(),
            );
        }
    }
    Ok(())
}

pub fn import(home: &MarimoHome, bytes: &[u8], now: u64) -> Result<(), String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("cloud metadata exceeds 256 KiB".into());
    }
    let snapshot: CloudSnapshot = serde_json::from_slice(bytes)
        .map_err(|_| "invalid cloud metadata; only observed_at and thread id/title/cwd/status/hostId are accepted".to_owned())?;
    if !fresh(snapshot.observed_at, now) {
        return Err("cloud observation is expired or in the future; fetch fresh metadata".into());
    }
    validate(&snapshot)?;
    // 独立したファイルを置き換え、一覧から消えたスレッドも次の取得で外す。
    store::write_json_atomic(&file(home), &snapshot).map_err(|e| e.to_string())
}

pub fn clear(home: &MarimoHome) -> io::Result<()> {
    match fs::remove_file(file(home)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn read(home: &MarimoHome) -> Option<CloudSnapshot> {
    let path = file(home);
    if fs::metadata(&path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let s: CloudSnapshot = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    validate(&s).ok()?;
    Some(s)
}

/// 保存済みの観測がまだ期限内なら、その期限の時刻を返す。監視はこの時刻に起きて描き直す。
pub fn expires_at(home: &MarimoHome, now: u64) -> Option<u64> {
    read(home)
        .filter(|s| fresh(s.observed_at, now))
        .map(|s| s.observed_at.saturating_add(TTL_MS))
}

pub fn load(home: &MarimoHome, now: u64) -> Vec<SessionState> {
    let Some(snapshot) = read(home) else {
        return vec![];
    };
    let expired = !fresh(snapshot.observed_at, now);
    snapshot
        .threads
        .into_iter()
        .map(|t| SessionState {
            provider: Provider::Codex,
            cloud: Some(Observation {
                thread_id: t.id.clone(),
                observed_at: snapshot.observed_at,
                expires_at: snapshot.observed_at.saturating_add(TTL_MS),
                expired,
            }),
            cwd: t.cwd,
            title: Some(t.title),
            status: if expired {
                Status::Idle
            } else {
                t.status.status()
            },
            updated_at: snapshot.observed_at,
            activity: Some(Activity {
                kind: ActivityKind::Text,
                tool: None,
                summary: if expired {
                    "状態不明".into()
                } else {
                    "手動観測 · 自動更新なし".into()
                },
                detail: Some(format!("thread: {}", t.id)),
            }),
            ..SessionState::new(format!("cloud-{}", t.id))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const AT: u64 = 1_790_916_000_000;

    fn metadata(status: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"observed_at":AT,"threads":[{
            "id":"thread-1","title":"実タスク","cwd":"/w/project","status":status,"hostId":"durable"
        }]}))
        .unwrap()
    }

    #[test]
    fn toggle_expiry_and_replacement_preserve_local_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        let local = SessionState {
            provider: Provider::Codex,
            status: Status::Working,
            ..SessionState::new("thread-1")
        };
        store::write_json_atomic(
            &home.session_file(Provider::Codex, "thread-1").unwrap(),
            &local,
        )
        .unwrap();
        import(&home, &metadata("active"), AT).unwrap();
        let load = |enabled, now| store::load_snapshot_with_cloud(&home, enabled, now);
        assert_eq!(load(false, AT).sessions, vec![local.clone()]);
        let on = load(true, AT);
        assert_eq!(on.sessions.len(), 2);
        let cloud = on.sessions.iter().find(|s| s.cloud.is_some()).unwrap();
        assert_ne!(cloud.session_id, local.session_id);
        assert!(cloud.context.is_none() && cloud.origin.is_none() && cloud.agents.is_empty());
        assert_eq!(load(false, AT + 1).sessions, vec![local.clone()]);
        assert_eq!(load(true, AT + 2).sessions.len(), 2);
        assert_eq!(expires_at(&home, AT + 1), Some(AT + TTL_MS));
        assert_eq!(expires_at(&home, AT + TTL_MS), None);
        let expired = load(true, AT + TTL_MS);
        let row = expired.sessions.iter().find(|s| s.cloud.is_some()).unwrap();
        assert!(row.cloud.as_ref().unwrap().expired);
        assert_eq!(row.status, Status::Idle);
        import(&home, &metadata("interrupted"), AT).unwrap();
        assert_eq!(
            load(true, AT)
                .sessions
                .iter()
                .find(|s| s.cloud.is_some())
                .unwrap()
                .status,
            Status::Idle
        );
        import(
            &home,
            &serde_json::to_vec(&json!({"observed_at":AT,"threads":[]})).unwrap(),
            AT,
        )
        .unwrap();
        assert_eq!(load(true, AT).sessions, vec![local.clone()]);
        clear(&home).unwrap();
        assert_eq!(load(true, AT).sessions, vec![local]);
    }

    #[test]
    fn rejects_stale_future_body_and_wrong_host_without_replacing_good_data() {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        let good = metadata("active");
        import(&home, &good, AT).unwrap();
        assert!(import(&home, &good, AT + TTL_MS).is_err());
        assert!(import(&home, &good, AT - 1).is_err());
        for (field, value) in [
            ("body", "secret"),
            ("hostId", "local"),
            ("status", "unknown"),
            ("id", "../escape"),
        ] {
            let mut v: serde_json::Value = serde_json::from_slice(&good).unwrap();
            v["threads"][0][field] = value.into();
            assert!(import(&home, &serde_json::to_vec(&v).unwrap(), AT).is_err());
        }
        assert_eq!(load(&home, AT).len(), 1);
    }
}
