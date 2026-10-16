use std::env;
use std::path::PathBuf;

use marimo_core::paths::user_home;

const TEMPLATE: &str = include_str!("opencode_plugin.js");

/// marimo が書いたプラグインだと見分ける印。雛形の一行目にある。利用者が同じ名前で置いた別のファイルは、
/// この印を持たないので書き換えも削除もしない。
const MARKER: &str = "// marimo-opencode-plugin\n";

const HOOK_PLACEHOLDER: &str = "__MARIMO_HOOK__";

pub const FILE_NAME: &str = "marimo.js";

/// OpenCode の設定のフォルダ。OpenCode は OS によらず XDG_CONFIG_HOME（なければホームの .config）の下の
/// opencode を使う。
pub fn config_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| user_home().map(|h| h.join(".config")))?;
    Some(base.join("opencode"))
}

/// marimo-hook の絶対パスを JSON の文字列として埋め込む。JSON の文字列はそのまま JavaScript の文字列に
/// なるので、パスに空白や引用符が含まれても崩れない。
pub fn render(exe_path: &str) -> String {
    let literal = serde_json::to_string(exe_path).expect("a string always serializes");
    TEMPLATE.replacen(HOOK_PLACEHOLDER, &literal, 1)
}

pub fn is_ours(text: &str) -> bool {
    text.starts_with(MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_plugin_is_marked_and_embeds_the_path_as_a_string() {
        let text = render(r#"/Users/a b/"q"\marimo-hook"#);
        assert!(is_ours(&text));
        assert!(!text.contains(HOOK_PLACEHOLDER));
        assert!(text.contains(r#"const HOOK = "/Users/a b/\"q\"\\marimo-hook""#));
        assert!(!is_ours("export default {}\n"));
    }
}
