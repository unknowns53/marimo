use std::fs;
use std::io;

use marimo_core::{MarimoHome, store};

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
    store::write_json_atomic(&home.acknowledged_file(), &kept)
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
}
