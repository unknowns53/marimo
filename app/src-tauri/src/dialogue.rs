use std::fs;
use std::io;

use marimo_core::MarimoHome;
use serde_json::{Map, Value};

// これまでのアプリは既定のセリフを $MARIMO_HOME/dialogue.json へ丸ごと書き出していた。そのままだと
// 既定のセリフを直しても利用者の手元に届かないので、過去に出荷した既定と中身が同じファイルは
// 利用者が編集していないものとみなし、読まずに退避する。
const SHIPPED_DEFAULTS: [&str; 3] = [
    include_str!("../shipped-dialogue/1-plain.json"),
    include_str!("../shipped-dialogue/2-folder.json"),
    include_str!("../shipped-dialogue/3-reaction.json"),
];

pub const RETIRED_SUFFIX: &str = "unused-default";

fn is_shipped_default(value: &Value) -> bool {
    SHIPPED_DEFAULTS
        .iter()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .any(|d| &d == value)
}

/// 退避したら true を返す。ファイルが無い、読めない、編集されている場合は何もしない。
pub fn retire_shipped_default(home: &MarimoHome) -> io::Result<bool> {
    let path = home.dialogue_file();
    let Ok(bytes) = fs::read(&path) else {
        return Ok(false);
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(false);
    };
    if !is_shipped_default(&value) {
        return Ok(false);
    }
    let retired = path.with_extension(format!("json.{RETIRED_SUFFIX}"));
    fs::rename(&path, retired)?;
    Ok(true)
}

/// 利用者が上書きしたい分類だけを返す。既定との重ね合わせはフロントエンドが分類ごとに行う。
/// 読めない、形式が違う場合は空にして、既定のセリフで動き続けられるようにする。
pub fn user_overrides(home: &MarimoHome) -> Value {
    let parsed = fs::read(home.dialogue_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let Some(Value::Object(obj)) = parsed.filter(|v| !is_shipped_default(v)) else {
        return Value::Object(Map::new());
    };
    let valid: Map<String, Value> = obj
        .into_iter()
        .filter(|(_, v)| {
            v.as_array()
                .is_some_and(|lines| !lines.is_empty() && lines.iter().all(Value::is_string))
        })
        .collect();
    Value::Object(valid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn home() -> (tempfile::TempDir, MarimoHome) {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        (dir, home)
    }

    #[test]
    fn retires_only_unedited_shipped_defaults() {
        for shipped in SHIPPED_DEFAULTS {
            let (_d, home) = home();
            fs::write(home.dialogue_file(), shipped).unwrap();
            assert!(retire_shipped_default(&home).unwrap());
            assert!(!home.dialogue_file().exists());
            let retired = home.root().join("dialogue.json.unused-default");
            assert_eq!(fs::read_to_string(retired).unwrap(), shipped);
        }

        let (_d, home) = home();
        assert!(!retire_shipped_default(&home).unwrap());
        fs::write(home.dialogue_file(), r#"{"done": ["自分で書いたセリフ"]}"#).unwrap();
        assert!(!retire_shipped_default(&home).unwrap());
        assert!(home.dialogue_file().exists());
        fs::write(home.dialogue_file(), "{broken").unwrap();
        assert!(!retire_shipped_default(&home).unwrap());
        assert!(home.dialogue_file().exists());
    }

    #[test]
    fn overrides_keep_only_valid_categories() {
        let (_d, home) = home();
        assert_eq!(user_overrides(&home), json!({}));
        fs::write(
            home.dialogue_file(),
            r#"{"done": ["終わり"], "error": [], "waiting": "not a list", "reaction": [1, 2]}"#,
        )
        .unwrap();
        assert_eq!(user_overrides(&home), json!({"done": ["終わり"]}));
        fs::write(home.dialogue_file(), "[1,2]").unwrap();
        assert_eq!(user_overrides(&home), json!({}));
        fs::write(home.dialogue_file(), SHIPPED_DEFAULTS[1]).unwrap();
        assert_eq!(user_overrides(&home), json!({}));
    }
}
