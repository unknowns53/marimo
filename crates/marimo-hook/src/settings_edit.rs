use serde_json::{Map, Value, json};

// フックの設定の形（イベントごとの matcher グループの配列と、その中の hooks 配列）は
// https://code.claude.com/docs/en/hooks の Configuration の節に従う。
pub const EVENTS: [&str; 14] = [
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
];

const HOOK_TIMEOUT_SECS: u64 = 5;

pub struct Marimo {
    raw: String,
    quoted: String,
}

impl Marimo {
    pub fn new(exe_path: &str) -> Self {
        Self {
            raw: exe_path.to_owned(),
            quoted: shell_quote(exe_path),
        }
    }

    pub fn hook_command(&self) -> String {
        format!("{} hook", self.quoted)
    }

    fn statusline_command(&self) -> String {
        format!("{} statusline", self.quoted)
    }

    fn wrapped_statusline(&self, original: &str) -> String {
        format!(
            "{} -- sh -c {}",
            self.statusline_command(),
            quote_always(original)
        )
    }

    pub fn is_hook_command(&self, command: &str) -> bool {
        self.args_after_exe(command) == Some(" hook")
    }

    // 実行ファイルのパスの直後が空白でなければ marimo ではない。前方一致だけで判定すると、
    // marimo-hook-backup のように名前の先頭が同じ別のコマンドまで marimo のものとして扱ってしまう。
    fn args_after_exe<'a>(&self, command: &'a str) -> Option<&'a str> {
        [self.quoted.as_str(), self.raw.as_str()]
            .into_iter()
            .find_map(|exe| {
                command
                    .strip_prefix(exe)
                    .filter(|rest| rest.starts_with(' '))
            })
    }

    fn statusline_rest<'a>(&self, command: &'a str) -> Option<&'a str> {
        let rest = self.args_after_exe(command)?.strip_prefix(" statusline")?;
        (rest.is_empty() || rest.starts_with(" -- ")).then_some(rest)
    }
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
    pub already: Vec<&'static str>,
    pub status_line: StatusLineChange,
}

impl InstallReport {
    pub fn changed(&self) -> bool {
        !self.added.is_empty()
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

pub fn install(
    settings: &mut Value,
    marimo: &Marimo,
    wrap_status_line: bool,
) -> Result<InstallReport, String> {
    let root = settings
        .as_object_mut()
        .ok_or("settings.json の最上位が JSON のオブジェクトではありません")?;
    check_hooks_shape(root)?;

    let mut added = Vec::new();
    let mut already = Vec::new();
    for event in EVENTS {
        let has_marimo = root
            .get("hooks")
            .and_then(|h| h.get(event))
            .and_then(Value::as_array)
            .is_some_and(|groups| groups.iter().any(|g| group_has_marimo(g, marimo)));
        if has_marimo {
            already.push(event);
            continue;
        }
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or("hooks がオブジェクトではありません")?;
        hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} が配列ではありません"))?
            .push(json!({
                "hooks": [{
                    "type": "command",
                    "command": marimo.hook_command(),
                    "timeout": HOOK_TIMEOUT_SECS,
                }]
            }));
        added.push(event);
    }

    let status_line = if wrap_status_line {
        install_status_line(root, marimo)
    } else {
        StatusLineChange::Untouched {
            reason: "Windows での statusLine の包み方は検証していないため、書き換えませんでした"
                .to_owned(),
        }
    };
    Ok(InstallReport {
        added,
        already,
        status_line,
    })
}

fn install_status_line(root: &mut Map<String, Value>, marimo: &Marimo) -> StatusLineChange {
    let Some(existing) = root.get_mut("statusLine") else {
        let after = marimo.statusline_command();
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
    check_hooks_shape(root)?;

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
                    handlers.retain(|h| !handler_is_marimo(h, marimo));
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

    let status_line = uninstall_status_line(root, marimo);
    Ok(UninstallReport {
        removed,
        status_line,
    })
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

fn check_hooks_shape(root: &Map<String, Value>) -> Result<(), String> {
    let Some(hooks) = root.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks
        .as_object()
        .ok_or("hooks がオブジェクトではありません")?;
    for event in EVENTS {
        if let Some(v) = hooks.get(event)
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
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| marimo.is_hook_command(c))
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
    fn quoting_round_trips() {
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
    }

    #[test]
    fn shell_quote_only_when_needed() {
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
    fn detects_marimo_commands_quoted_or_not() {
        let m = Marimo::new("/Users/a b/.marimo/bin/marimo-hook");
        assert!(m.is_hook_command("'/Users/a b/.marimo/bin/marimo-hook' hook"));
        assert!(m.is_hook_command("/Users/a b/.marimo/bin/marimo-hook hook"));
        assert!(!m.is_hook_command("'/Users/a b/.marimo/bin/marimo-hook' record Stop"));
        assert!(!m.is_hook_command("/other/marimo-hook hook"));
    }

    #[test]
    fn commands_sharing_the_executable_prefix_are_not_marimo() {
        let m = Marimo::new("/x/marimo-hook");
        assert!(m.is_hook_command("/x/marimo-hook hook"));
        for c in [
            "/x/marimo-hook-backup hook",
            "/x/marimo-hooky hook",
            "/x/marimo-hook hook --extra",
            "/x/marimo-hook  hook",
        ] {
            assert!(!m.is_hook_command(c), "{c}");
        }
        let quoted = Marimo::new("/x y/marimo-hook");
        assert!(!quoted.is_hook_command("'/x y/marimo-hook'-backup hook"));
        assert!(!quoted.is_hook_command("/x y/marimo-hook-backup hook"));

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

    #[test]
    fn install_and_uninstall_leave_foreign_prefix_commands_alone() {
        let m = Marimo::new("/x/marimo-hook");
        let foreign_hook = "/x/marimo-hook-backup hook";
        let foreign_status = "/x/marimo-hooky statusline";
        let mut settings = json!({
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": foreign_hook}]}]},
            "statusLine": {"type": "command", "command": foreign_status}
        });
        let report = install(&mut settings, &m, true).unwrap();
        assert!(report.added.contains(&"Stop"));
        assert_eq!(
            report.status_line,
            StatusLineChange::Wrapped {
                before: foreign_status.to_owned(),
                after: format!("/x/marimo-hook statusline -- sh -c '{foreign_status}'"),
            }
        );
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);

        let report = uninstall(&mut settings, &m).unwrap();
        assert!(report.removed.contains(&"Stop".to_owned()));
        assert_eq!(
            settings,
            json!({
                "hooks": {"Stop": [{"hooks": [{"type": "command", "command": foreign_hook}]}]},
                "statusLine": {"type": "command", "command": foreign_status}
            })
        );

        // marimo が入っていない状態の uninstall は、名前の似た他人のフックに触れない。
        let report = uninstall(&mut settings, &m).unwrap();
        assert!(!report.changed());
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["command"],
            foreign_hook
        );
    }
}
