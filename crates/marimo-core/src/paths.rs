use std::env;
use std::path::{Path, PathBuf};

pub const HOME_ENV: &str = "MARIMO_HOME";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarimoHome {
    root: PathBuf,
}

impl MarimoHome {
    pub fn resolve() -> Option<Self> {
        if let Some(dir) = env::var_os(HOME_ENV).filter(|v| !v.is_empty()) {
            return Some(Self::at(dir));
        }
        user_home().map(|home| Self::at(home.join(".marimo")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// session_id はフックの入力をそのまま使うので、`sessions/` の外を指しうる値や
    /// ファイル名にできない値には `None` を返す。
    pub fn session_file(&self, session_id: &str) -> Option<PathBuf> {
        is_safe_id(session_id).then(|| self.sessions_dir().join(format!("{session_id}.json")))
    }

    pub fn rate_limits_file(&self) -> PathBuf {
        self.root.join("rate_limits.json")
    }

    pub fn record_file(&self) -> PathBuf {
        self.root.join("logs").join("record.jsonl")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join(".lock")
    }

    pub fn window_file(&self) -> PathBuf {
        self.root.join("window.json")
    }

    pub fn dialogue_file(&self) -> PathBuf {
        self.root.join("dialogue.json")
    }
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn user_home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_file_rejects_path_tricks() {
        let home = MarimoHome::at("/tmp/m");
        assert!(home.session_file("abc-123_x").is_some());
        assert!(home.session_file("").is_none());
        assert!(home.session_file("../evil").is_none());
        assert!(home.session_file("a/b").is_none());
        assert!(home.session_file("a\\b").is_none());
        assert!(home.session_file(".hidden").is_none());
    }
}
