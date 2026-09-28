use serde_json::{Map, Value, json};

// フックの設定の形（イベントごとの matcher グループの配列と、その中の hooks 配列）は
// https://code.claude.com/docs/en/hooks の Configuration の節に従う。
pub const EVENTS: [&str; 16] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PermissionDenied",
    "PostToolUse",
    "PostToolUseFailure",
    "Notification",
    "Elicitation",
    "ElicitationResult",
    "Stop",
    "StopFailure",
    "SessionEnd",
    "MessageDisplay",
    "SubagentStart",
    "SubagentStop",
];

const HOOK_TIMEOUT_SECS: u64 = 5;
const HOOK_ARGS: [&str; 1] = ["hook"];

// Codex の hooks.json は Claude Code の hooks と同じ形をしている（codex-rs/config/src/hook_config.rs の
// HooksFile と HookEventsToml）。PreCompact と PostCompact は状態を変えないので登録しない。
pub const CODEX_EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
    "Interrupt",
    "SessionEnd",
    "SubagentStart",
    "SubagentStop",
];

/// Codex のフックに登録するコマンドの文字列。Codex のハンドラには args が無く、command は常に
/// シェルを通して実行される（codex-rs/hooks/src/engine/command_runner.rs の build_command）。
/// Unix では sh 系のシェルなので単一引用符で引用する。Windows では PowerShell か cmd のどちらに
/// 渡るかが利用者の設定で変わり、cmd /C には全体を二重引用符で包んで渡されるので、どちらでも
/// 同じ一つのパスとして読まれる一語のパスだけを使い、そう書けなければ None を返す。
pub fn codex_hook_command(exe_path: &str, windows: bool) -> Option<String> {
    let exe = if windows {
        let plain = !exe_path.is_empty()
            && exe_path.chars().all(|c| {
                if c.is_ascii() {
                    c.is_ascii_alphanumeric() || matches!(c, '\\' | ':' | '.' | '_' | '-')
                } else {
                    !c.is_whitespace()
                }
            });
        plain.then(|| exe_path.to_owned())?
    } else {
        shell_quote(exe_path)
    };
    Some(format!("{exe} codex-hook"))
}

// フックは args を持つ exec form で登録する。hooks のドキュメントの Exec form and shell form の
// 節にあるとおり、exec form はシェルを通さずに実行ファイルを直接起動するので、パスの引用や
// シェルの設定ファイルが出力する文字列の混入を気にしなくてよい。
// statusLine には exec form がないので、シェルを通すコマンドの文字列で登録する。
pub struct Marimo {
    exe: String,
    raw: String,
    quoted: String,
    status_line: StatusLinePolicy,
}

enum StatusLinePolicy {
    Wrap,
    // Windows の statusLine は Git Bash があれば Git Bash、なければ PowerShell で動く。
    // 両者で引用の規則が違い、既存のコマンドを包む方法も確かめていないので、statusLine が
    // 無いときだけ、どちらのシェルでも同じように読める一語のパスで登録する。
    // plain はその一語で、そう書けないときは None になる。
    AddOnly { plain: Option<String> },
}

impl Marimo {
    pub fn new(exe_path: &str) -> Self {
        Self {
            exe: exe_path.to_owned(),
            raw: exe_path.to_owned(),
            quoted: shell_quote(exe_path),
            status_line: StatusLinePolicy::Wrap,
        }
    }

    // exec form の command には、Windows でも本物の実行ファイルとして解決できる元の形のパスを書く。
    // シェルを通す statusLine と、以前の shell form のフックでは、Git Bash が引用していない \ を
    // エスケープとして消してしまうので、statusline のドキュメントの Windows configuration の節に
    // あるとおり区切りを / にする。
    pub fn windows(exe_path: &str, home: Option<&str>) -> Self {
        let raw = exe_path.replace('\\', "/");
        Self {
            exe: exe_path.to_owned(),
            quoted: shell_quote(&raw),
            status_line: StatusLinePolicy::AddOnly {
                plain: shell_agnostic_path(exe_path, home),
            },
            raw,
        }
    }

    fn hook_handler(&self) -> Value {
        json!({
            "type": "command",
            "command": self.exe,
            "args": HOOK_ARGS,
            "timeout": HOOK_TIMEOUT_SECS,
        })
    }

    fn is_exec_handler(&self, handler: &Value) -> bool {
        handler.get("command").and_then(Value::as_str) == Some(self.exe.as_str())
            && handler.get("args") == Some(&json!(HOOK_ARGS))
    }

    // args を持つハンドラは command をシェルに渡さないので、以前の形と同じ文字列でも marimo ではない。
    fn is_legacy_handler(&self, handler: &Value) -> bool {
        handler.get("args").is_none()
            && handler
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|c| self.is_legacy_hook_command(c))
    }

    // 利用者が足した項目とキーの順序を残したまま、command だけを exec form へ書き換える。
    fn migrated(&self, legacy: &Map<String, Value>) -> Value {
        let mut out = Map::new();
        for (key, value) in legacy {
            if key == "command" {
                out.insert(key.clone(), Value::String(self.exe.clone()));
                out.insert("args".to_owned(), json!(HOOK_ARGS));
            } else {
                out.insert(key.clone(), value.clone());
            }
        }
        Value::Object(out)
    }

    fn statusline_command(&self) -> Option<String> {
        match &self.status_line {
            StatusLinePolicy::Wrap => Some(format!("{} statusline", self.quoted)),
            StatusLinePolicy::AddOnly { plain } => {
                plain.as_ref().map(|p| format!("{p} statusline"))
            }
        }
    }

    fn wrapped_statusline(&self, original: &str) -> String {
        format!(
            "{} statusline -- sh -c {}",
            self.quoted,
            quote_always(original)
        )
    }

    fn is_legacy_hook_command(&self, command: &str) -> bool {
        self.args_after(command, &[&self.quoted, &self.raw]) == Some(" hook")
    }

    // 実行ファイルのパスの直後が空白でなければ marimo ではない。前方一致だけで判定すると、
    // marimo-hook-backup のように名前の先頭が同じ別のコマンドまで marimo のものとして扱ってしまう。
    fn args_after<'a>(&self, command: &'a str, spellings: &[&str]) -> Option<&'a str> {
        spellings.iter().find_map(|exe| {
            command
                .strip_prefix(exe)
                .filter(|rest| rest.starts_with(' '))
        })
    }

    fn statusline_rest<'a>(&self, command: &'a str) -> Option<&'a str> {
        let mut spellings = vec![self.quoted.as_str(), self.raw.as_str()];
        if let StatusLinePolicy::AddOnly { plain: Some(p) } = &self.status_line {
            spellings.push(p);
        }
        let rest = self
            .args_after(command, &spellings)?
            .strip_prefix(" statusline")?;
        (rest.is_empty() || rest.starts_with(" -- ")).then_some(rest)
    }
}

// Git Bash と PowerShell のどちらに渡しても、引用なしで同じ一つのパスとして読まれる書き方を返す。
// ホームフォルダの下なら、ホームのパスに空白があっても避けられるよう ~/ で始める。
// statusline のドキュメントの Windows configuration の節にあるとおり、~ はどちらのシェルでも
// Windows のホームフォルダに展開される。Windows のパスは大文字と小文字を区別しないので、
// ホームの下かどうかも区別せずに判定する。
fn shell_agnostic_path(exe_path: &str, home: Option<&str>) -> Option<String> {
    let exe = exe_path.replace('\\', "/");
    let under_home = home
        .map(|h| h.replace('\\', "/").trim_end_matches('/').to_owned())
        .filter(|h| !h.is_empty())
        .and_then(|h| {
            let head = exe.get(..h.len())?;
            let rest = exe.get(h.len()..)?.strip_prefix('/')?;
            head.eq_ignore_ascii_case(&h).then(|| rest.to_owned())
        });
    match under_home {
        Some(rest) => plain_word(&rest).then(|| format!("~/{rest}")),
        None => plain_word(&exe).then_some(exe),
    }
}

// 空白のほか、どちらかのシェルが特別に扱う記号を含まない一語だけを受け付ける。
// 先頭の ~ はホームへの展開になるので、明示的に ~/ を付ける場合のほかは避ける。
fn plain_word(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('~')
        && s.chars().all(|c| {
            if c.is_ascii() {
                c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ':' | '~')
            } else {
                !c.is_whitespace()
            }
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusLineChange {
    Added { after: String },
    Wrapped { before: String, after: String },
    AlreadyMarimo { current: String },
    Removed { before: String },
    Restored { before: String, after: String },
    Untouched { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub added: Vec<&'static str>,
    pub migrated: Vec<&'static str>,
    pub already: Vec<&'static str>,
    pub status_line: StatusLineChange,
}

impl InstallReport {
    pub fn changed(&self) -> bool {
        !self.added.is_empty()
            || !self.migrated.is_empty()
            || matches!(
                self.status_line,
                StatusLineChange::Added { .. } | StatusLineChange::Wrapped { .. }
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    pub removed: Vec<String>,
    pub status_line: StatusLineChange,
}

impl UninstallReport {
    pub fn changed(&self) -> bool {
        !self.removed.is_empty()
            || matches!(
                self.status_line,
                StatusLineChange::Removed { .. } | StatusLineChange::Restored { .. }
            )
    }
}

pub fn install(settings: &mut Value, marimo: &Marimo) -> Result<InstallReport, String> {
    let root = settings
        .as_object_mut()
        .ok_or("settings.json の最上位が JSON のオブジェクトではありません")?;
    check_hooks_shape(root, &EVENTS)?;

    let mut added = Vec::new();
    let mut migrated = Vec::new();
    let mut already = Vec::new();
    for event in EVENTS {
        if let Some(groups) = root
            .get_mut("hooks")
            .and_then(|h| h.get_mut(event))
            .and_then(Value::as_array_mut)
        {
            // 以前の shell form で登録したハンドラは、同じグループの同じ位置で exec form に置き換える。
            let mut replaced = false;
            for handler in groups
                .iter_mut()
                .filter_map(|g| g.get_mut("hooks").and_then(Value::as_array_mut))
                .flatten()
            {
                if marimo.is_legacy_handler(handler)
                    && let Some(legacy) = handler.as_object()
                {
                    *handler = marimo.migrated(legacy);
                    replaced = true;
                }
            }
            if replaced {
                migrated.push(event);
                continue;
            }
            if groups.iter().any(|g| group_has_marimo(g, marimo)) {
                already.push(event);
                continue;
            }
        }
        push_group(root, event, marimo.hook_handler())?;
        added.push(event);
    }

    let status_line = install_status_line(root, marimo);
    Ok(InstallReport {
        added,
        migrated,
        already,
        status_line,
    })
}

fn push_group(root: &mut Map<String, Value>, event: &str, handler: Value) -> Result<(), String> {
    root.entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("hooks がオブジェクトではありません")?
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| format!("hooks.{event} が配列ではありません"))?
        .push(json!({ "hooks": [handler] }));
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexInstallReport {
    pub added: Vec<&'static str>,
    pub already: Vec<&'static str>,
}

/// Codex はフックの定義を正規化したもののハッシュで信頼を覚えるので、すでに同じコマンドの
/// ハンドラがあるイベントには手を付けない。書き直すと利用者が /hooks で信頼し直すことになる。
/// marimo-hook の場所が変わって別のコマンドで登録されているものは、二重に動かないよう取り除く。
pub fn install_codex(file: &mut Value, command: &str) -> Result<CodexInstallReport, String> {
    let root = file
        .as_object_mut()
        .ok_or("hooks.json の最上位が JSON のオブジェクトではありません")?;
    check_hooks_shape(root, &CODEX_EVENTS)?;
    let mut added = Vec::new();
    let mut already = Vec::new();
    for event in CODEX_EVENTS {
        let registered = root
            .get("hooks")
            .and_then(|h| h.get(event))
            .and_then(Value::as_array)
            .is_some_and(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|hs| hs.iter().any(|h| is_codex_handler(h, command)))
                })
            });
        if registered {
            already.push(event);
            continue;
        }
        if let Some(groups) = root
            .get_mut("hooks")
            .and_then(|h| h.get_mut(event))
            .and_then(Value::as_array_mut)
        {
            groups.retain_mut(|group| {
                let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                    return true;
                };
                let n = handlers.len();
                handlers.retain(|h| !is_marimo_codex_handler(h));
                handlers.len() == n || !handlers.is_empty()
            });
        }
        let handler = json!({
            "type": "command",
            "command": command,
            "timeout": HOOK_TIMEOUT_SECS,
        });
        push_group(root, event, handler)?;
        added.push(event);
    }
    Ok(CodexInstallReport { added, already })
}

pub fn uninstall_codex(file: &mut Value) -> Result<Vec<String>, String> {
    let root = file
        .as_object_mut()
        .ok_or("hooks.json の最上位が JSON のオブジェクトではありません")?;
    check_hooks_shape(root, &CODEX_EVENTS)?;
    Ok(remove_handlers(root, is_marimo_codex_handler))
}

fn is_codex_handler(handler: &Value, command: &str) -> bool {
    handler.get("type").and_then(Value::as_str) == Some("command")
        && handler.get("command").and_then(Value::as_str) == Some(command)
}

// codex_hook_command が書いた形、つまり一語の marimo-hook のパスに codex-hook が続くものだけを
// marimo のものとみなす。どこに置いた marimo-hook でも当たるので、古い場所の登録も見つかる。
fn is_marimo_codex_handler(handler: &Value) -> bool {
    let Some(program) = handler
        .get("command")
        .and_then(Value::as_str)
        .filter(|_| handler.get("type").and_then(Value::as_str) == Some("command"))
        .and_then(|c| c.strip_suffix(" codex-hook"))
    else {
        return false;
    };
    let path = unquote_single_word(program).unwrap_or_else(|| program.to_owned());
    matches!(
        path.rsplit(['/', '\\']).next(),
        Some("marimo-hook" | "marimo-hook.exe")
    )
}

const API_HINT: &str = "利用制限を表示するには、アプリの右クリックメニューで「利用制限を API から取得」を有効にしてください";

fn install_status_line(root: &mut Map<String, Value>, marimo: &Marimo) -> StatusLineChange {
    let Some(existing) = root.get_mut("statusLine") else {
        let Some(after) = marimo.statusline_command() else {
            return StatusLineChange::Untouched {
                reason: format!(
                    "marimo-hook のパスに空白などが含まれ、Git Bash と PowerShell のどちらでも同じように読める形で書けないため、statusLine を登録しませんでした。{API_HINT}"
                ),
            };
        };
        root.insert(
            "statusLine".to_owned(),
            json!({ "type": "command", "command": after }),
        );
        return StatusLineChange::Added { after };
    };
    let Some(command) = command_status_line(existing) else {
        return StatusLineChange::Untouched {
            reason: "statusLine が command 形式ではないため、書き換えませんでした".to_owned(),
        };
    };
    if marimo.statusline_rest(&command).is_some() {
        return StatusLineChange::AlreadyMarimo { current: command };
    }
    if let StatusLinePolicy::AddOnly { .. } = marimo.status_line {
        return StatusLineChange::Untouched {
            reason: format!(
                "Windows では既存の statusLine を包む方法を検証していないため、書き換えませんでした。{API_HINT}"
            ),
        };
    }
    let after = marimo.wrapped_statusline(&command);
    existing["command"] = Value::String(after.clone());
    StatusLineChange::Wrapped {
        before: command,
        after,
    }
}

pub fn uninstall(settings: &mut Value, marimo: &Marimo) -> Result<UninstallReport, String> {
    let root = settings
        .as_object_mut()
        .ok_or("settings.json の最上位が JSON のオブジェクトではありません")?;
    check_hooks_shape(root, &EVENTS)?;
    let removed = remove_handlers(root, |h| handler_is_marimo(h, marimo));
    let status_line = uninstall_status_line(root, marimo);
    Ok(UninstallReport {
        removed,
        status_line,
    })
}

// is_ours に当たるハンドラを取り除き、手を入れたイベントの名前を返す。
fn remove_handlers(root: &mut Map<String, Value>, is_ours: impl Fn(&Value) -> bool) -> Vec<String> {
    let mut removed = Vec::new();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let mut emptied = Vec::new();
        for (event, groups) in hooks.iter_mut() {
            let Some(groups) = groups.as_array_mut() else {
                continue;
            };
            let mut touched = false;
            for group in groups.iter_mut() {
                if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    let n = handlers.len();
                    handlers.retain(|h| !is_ours(h));
                    touched |= handlers.len() != n;
                }
            }
            if !touched {
                continue;
            }
            // marimo の手順だけを持っていたグループは、グループごと取り除く。
            groups.retain(|g| {
                g.get("hooks")
                    .and_then(Value::as_array)
                    .is_none_or(|h| !h.is_empty())
            });
            removed.push(event.clone());
            if groups.is_empty() {
                emptied.push(event.clone());
            }
        }
        for event in &emptied {
            hooks.shift_remove(event);
        }
    }
    // install の前から空だった hooks を消さないよう、marimo の分を取り除いて空になった場合だけ消す。
    if !removed.is_empty()
        && root
            .get("hooks")
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty)
    {
        root.shift_remove("hooks");
    }
    removed
}

fn uninstall_status_line(root: &mut Map<String, Value>, marimo: &Marimo) -> StatusLineChange {
    let Some(command) = root.get("statusLine").and_then(command_status_line) else {
        return StatusLineChange::Untouched {
            reason: "marimo の statusLine は登録されていません".to_owned(),
        };
    };
    let Some(rest) = marimo.statusline_rest(&command) else {
        return StatusLineChange::Untouched {
            reason: "statusLine は marimo のものではないので、そのままにしました".to_owned(),
        };
    };
    if rest.trim().is_empty() {
        root.shift_remove("statusLine");
        return StatusLineChange::Removed { before: command };
    }
    match rest
        .strip_prefix(" -- sh -c ")
        .and_then(unquote_single_word)
    {
        Some(original) => {
            root["statusLine"]["command"] = Value::String(original.clone());
            StatusLineChange::Restored {
                before: command,
                after: original,
            }
        }
        None => StatusLineChange::Untouched {
            reason: "statusLine は marimo が包んだ形として読み取れないので、そのままにしました"
                .to_owned(),
        },
    }
}

fn check_hooks_shape(root: &Map<String, Value>, events: &[&str]) -> Result<(), String> {
    let Some(hooks) = root.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks
        .as_object()
        .ok_or("hooks がオブジェクトではありません")?;
    for event in events {
        if let Some(v) = hooks.get(*event)
            && !v.is_array()
        {
            return Err(format!("hooks.{event} が配列ではありません"));
        }
    }
    Ok(())
}

fn command_status_line(status_line: &Value) -> Option<String> {
    let obj = status_line.as_object()?;
    if obj.get("type").and_then(Value::as_str) != Some("command") {
        return None;
    }
    obj.get("command")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn group_has_marimo(group: &Value, marimo: &Marimo) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hs| hs.iter().any(|h| handler_is_marimo(h, marimo)))
}

fn handler_is_marimo(handler: &Value, marimo: &Marimo) -> bool {
    marimo.is_exec_handler(handler) || marimo.is_legacy_handler(handler)
}

pub fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'/' | b'.' | b'_' | b'-' | b'+' | b':' | b'@' | b'%' | b'=' | b','
                )
        });
    if safe { s.to_owned() } else { quote_always(s) }
}

// 単一引用符の中では何も展開されないので、元のコマンドは sh -c に文字どおり届く。
// 引用符そのものだけは '\'' で一度閉じてから書き足す。
fn quote_always(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// quote_always の逆。marimo が包んだ形だけを読めればよいので、単一引用符の区間と
// \ で逃がした文字だけからなる一語を受け付け、それ以外は None にする。
fn unquote_single_word(word: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next()? {
                    '\'' => break,
                    ch => out.push(ch),
                }
            },
            '\\' => out.push(chars.next()?),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting() {
        for s in [
            "bash ~/.claude/statusline.sh",
            "jq -r '\"[\\(.model.display_name)]\"'",
            "it's a 'test' | cat",
            "",
            "'",
        ] {
            assert_eq!(unquote_single_word(&quote_always(s)).as_deref(), Some(s));
        }
        assert_eq!(unquote_single_word("'a' b"), None);
        assert_eq!(unquote_single_word("'open"), None);

        assert_eq!(
            shell_quote("/Users/a/.marimo/bin/marimo-hook"),
            "/Users/a/.marimo/bin/marimo-hook"
        );
        assert_eq!(
            shell_quote("/Users/a b/.marimo/bin/marimo-hook"),
            "'/Users/a b/.marimo/bin/marimo-hook'"
        );
        assert_eq!(shell_quote("/tmp/it's"), r"'/tmp/it'\''s'");
    }

    #[test]
    fn codex_command_is_one_shell_word_or_nothing() {
        let cases = [
            (
                "/Users/a/.marimo/bin/marimo-hook",
                false,
                Some("/Users/a/.marimo/bin/marimo-hook codex-hook"),
            ),
            (
                "/Users/a b/.marimo/bin/marimo-hook",
                false,
                Some("'/Users/a b/.marimo/bin/marimo-hook' codex-hook"),
            ),
            (
                r"C:\Users\a\.marimo\bin\marimo-hook.exe",
                true,
                Some(r"C:\Users\a\.marimo\bin\marimo-hook.exe codex-hook"),
            ),
            (
                r"C:\Users\山田\.marimo\bin\marimo-hook.exe",
                true,
                Some(r"C:\Users\山田\.marimo\bin\marimo-hook.exe codex-hook"),
            ),
            (r"C:\Users\a b\.marimo\bin\marimo-hook.exe", true, None),
            (r"C:\Users\a&b\marimo-hook.exe", true, None),
            (r"C:\Users\$a\marimo-hook.exe", true, None),
            ("", true, None),
        ];
        for (exe, windows, expected) in cases {
            assert_eq!(
                codex_hook_command(exe, windows).as_deref(),
                expected,
                "{exe}"
            );
        }
    }

    #[test]
    fn recognizes_marimo_commands_by_exact_path() {
        let spaced = Marimo::new("/Users/a b/.marimo/bin/marimo-hook");
        assert!(spaced.is_legacy_hook_command("'/Users/a b/.marimo/bin/marimo-hook' hook"));
        assert!(spaced.is_legacy_hook_command("/Users/a b/.marimo/bin/marimo-hook hook"));
        assert!(!spaced.is_legacy_hook_command("'/Users/a b/.marimo/bin/marimo-hook' record Stop"));
        assert!(!spaced.is_legacy_hook_command("/other/marimo-hook hook"));

        let m = Marimo::new("/x/marimo-hook");
        assert!(m.is_legacy_hook_command("/x/marimo-hook hook"));
        for c in [
            "/x/marimo-hook-backup hook",
            "/x/marimo-hooky hook",
            "/x/marimo-hook hook --extra",
            "/x/marimo-hook  hook",
        ] {
            assert!(!m.is_legacy_hook_command(c), "{c}");
        }
        let quoted = Marimo::new("/x y/marimo-hook");
        assert!(!quoted.is_legacy_hook_command("'/x y/marimo-hook'-backup hook"));
        assert!(!quoted.is_legacy_hook_command("/x y/marimo-hook-backup hook"));

        assert_eq!(m.statusline_rest("/x/marimo-hook statusline"), Some(""));
        assert_eq!(
            m.statusline_rest("/x/marimo-hook statusline -- sh -c 'echo'"),
            Some(" -- sh -c 'echo'")
        );
        for c in [
            "/x/marimo-hook-backup statusline",
            "/x/marimo-hooky statusline -- sh -c 'echo'",
            "/x/marimo-hook statusline-other",
            "/x/marimo-hook statusline --verbose",
        ] {
            assert_eq!(m.statusline_rest(c), None, "{c}");
        }
    }

    fn exec(command: &str, args: Value) -> Value {
        json!({"type": "command", "command": command, "args": args, "timeout": 5})
    }

    #[test]
    fn reinstall_adds_only_the_missing_events() {
        let m = Marimo::new("/x/marimo-hook");
        let mut settings = json!({});
        install(&mut settings, &m).unwrap();
        let hooks = settings["hooks"].as_object_mut().unwrap();
        hooks.remove("SubagentStart");
        hooks.remove("SubagentStop");
        assert_eq!(hooks.len(), 14);
        let before = settings.clone();

        let report = install(&mut settings, &m).unwrap();
        assert_eq!(report.added, ["SubagentStart", "SubagentStop"]);
        assert_eq!(report.already.len(), 14);
        for event in ["SubagentStart", "SubagentStop"] {
            assert_eq!(
                settings["hooks"][event],
                json!([{"hooks": [exec("/x/marimo-hook", json!(["hook"]))]}])
            );
        }
        for (event, groups) in before["hooks"].as_object().unwrap() {
            assert_eq!(&settings["hooks"][event], groups, "{event}");
        }
    }

    #[test]
    fn exec_form_uses_the_native_windows_path() {
        let m = Marimo::windows(r"C:\Users\a b\.marimo\bin\marimo-hook.exe", None);
        let mut settings = json!({});
        install(&mut settings, &m).unwrap();
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0],
            exec(r"C:\Users\a b\.marimo\bin\marimo-hook.exe", json!(["hook"]))
        );
        // 以前の shell form は / 区切りで書いていたので、その形も marimo のものとして読む。
        assert!(m.is_legacy_hook_command("'C:/Users/a b/.marimo/bin/marimo-hook.exe' hook"));
        assert!(!m.is_legacy_hook_command(r"'C:\Users\a b\.marimo\bin\marimo-hook.exe' hook"));
    }

    #[test]
    fn windows_status_line_path_reads_the_same_in_git_bash_and_powershell() {
        let home = Some(r"C:\Users\user");
        let cases = [
            (
                r"C:\Users\user\.marimo\bin\marimo-hook.exe",
                home,
                Some("~/.marimo/bin/marimo-hook.exe"),
            ),
            // ホームのパスに空白があっても、~/ で始めれば引用せずに済む。
            (
                r"C:\Users\A User\.marimo\bin\marimo-hook.exe",
                Some(r"C:\Users\A User"),
                Some("~/.marimo/bin/marimo-hook.exe"),
            ),
            (
                r"C:\Users\USER\.marimo\bin\marimo-hook.exe",
                Some(r"c:\users\user\"),
                Some("~/.marimo/bin/marimo-hook.exe"),
            ),
            (
                r"C:\Users\山田\.marimo\bin\marimo-hook.exe",
                Some(r"C:\Users\山田"),
                Some("~/.marimo/bin/marimo-hook.exe"),
            ),
            (
                r"D:\tools\marimo\bin\marimo-hook.exe",
                home,
                Some("D:/tools/marimo/bin/marimo-hook.exe"),
            ),
            (
                r"D:\tools\marimo-hook.exe",
                None,
                Some("D:/tools/marimo-hook.exe"),
            ),
            // 名前の先頭だけがホームと同じフォルダは、ホームの下ではない。
            (
                r"C:\Users\username\.marimo\bin\marimo-hook.exe",
                home,
                Some("C:/Users/username/.marimo/bin/marimo-hook.exe"),
            ),
            // 8.3 形式の短い名前の途中にある ~ は、どちらのシェルでも展開されない。
            (
                r"C:\Users\RUNNER~1\AppData\Local\Temp\m\bin\marimo-hook.exe",
                Some(r"C:\Users\runneradmin"),
                Some("C:/Users/RUNNER~1/AppData/Local/Temp/m/bin/marimo-hook.exe"),
            ),
            (r"D:\My Tools\marimo-hook.exe", home, None),
            (r"C:\Users\user\marimo home\bin\marimo-hook.exe", home, None),
            (
                r"C:\Users\user\マリモ　ホーム\bin\marimo-hook.exe",
                home,
                None,
            ),
            (r"D:\a$b\marimo-hook.exe", home, None),
            (r"D:\a(b)\marimo-hook.exe", home, None),
            (r"D:\it's\marimo-hook.exe", home, None),
            (r"D:\a;b\marimo-hook.exe", home, None),
        ];
        for (exe, home, expected) in cases {
            assert_eq!(shell_agnostic_path(exe, home).as_deref(), expected, "{exe}");
        }
    }

    #[test]
    fn windows_status_line_is_added_only_when_absent_and_writable() {
        let m = Marimo::windows(
            r"C:\Users\user\.marimo\bin\marimo-hook.exe",
            Some(r"C:\Users\user"),
        );
        let mut settings = json!({"model": "opus"});
        let report = install(&mut settings, &m).unwrap();
        let command = "~/.marimo/bin/marimo-hook.exe statusline";
        assert_eq!(
            report.status_line,
            StatusLineChange::Added {
                after: command.to_owned()
            }
        );
        assert_eq!(
            settings["statusLine"],
            json!({"type": "command", "command": command})
        );

        let before = settings.clone();
        let again = install(&mut settings, &m).unwrap();
        assert!(!again.changed());
        assert_eq!(
            again.status_line,
            StatusLineChange::AlreadyMarimo {
                current: command.to_owned()
            }
        );
        assert_eq!(settings, before);

        let report = uninstall(&mut settings, &m).unwrap();
        assert_eq!(
            report.status_line,
            StatusLineChange::Removed {
                before: command.to_owned()
            }
        );
        assert_eq!(settings, json!({"model": "opus"}));

        // ホームの外に置いた場合の / 区切りの絶対パスも、marimo のものとして取り除く。
        let outside = Marimo::windows(r"D:\tools\marimo-hook.exe", Some(r"C:\Users\user"));
        let mut settings = json!({});
        install(&mut settings, &outside).unwrap();
        assert_eq!(
            settings["statusLine"]["command"],
            "D:/tools/marimo-hook.exe statusline"
        );
        uninstall(&mut settings, &outside).unwrap();
        assert_eq!(settings, json!({}));

        // 既存の statusLine は包まずに残す。
        let original =
            json!({"statusLine": {"type": "command", "command": "bash ~/.claude/statusline.sh"}});
        let mut settings = original.clone();
        let report = install(&mut settings, &m).unwrap();
        let StatusLineChange::Untouched { reason } = &report.status_line else {
            panic!("{:?}", report.status_line);
        };
        assert!(reason.contains("「利用制限を API から取得」"), "{reason}");
        assert_eq!(settings["statusLine"], original["statusLine"]);
        uninstall(&mut settings, &m).unwrap();
        assert_eq!(settings, original);

        // どちらのシェルでも同じに読める形で書けないパスでは、statusLine を登録しない。
        let spaced = Marimo::windows(r"D:\My Tools\marimo-hook.exe", Some(r"C:\Users\user"));
        let mut settings = json!({});
        let report = install(&mut settings, &spaced).unwrap();
        let StatusLineChange::Untouched { reason } = &report.status_line else {
            panic!("{:?}", report.status_line);
        };
        assert!(reason.contains("「利用制限を API から取得」"), "{reason}");
        assert!(settings.get("statusLine").is_none());
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["command"],
            r"D:\My Tools\marimo-hook.exe"
        );
    }
}
