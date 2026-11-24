use marimo_core::Lang;
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
/// 渡るかが利用者の設定で変わり、cmd /C には全体を二重引用符で囲んで渡されるので、どちらでも
/// 同じ一つのパスとして読まれる一語のパスだけを使い、そう書けなければ None を返す。
pub fn codex_hook_command(exe_path: &str, windows: bool) -> Option<String> {
    hook_command(exe_path, windows, "codex-hook")
}

/// Antigravity のフックに登録するコマンドの文字列。agy もハンドラに args を持たず、command を Unix では
/// `sh -c`、Windows では `cmd /c` に書いたとおりに渡す（docs/hooks.md の Hook Handler Fields）ので、
/// Codex と同じ書き方にする。登録するときは、イベントごとにイベント名を引数として足す。
pub fn antigravity_hook_command(exe_path: &str, windows: bool) -> Option<String> {
    hook_command(exe_path, windows, ANTIGRAVITY_SUBCOMMAND)
}

const ANTIGRAVITY_SUBCOMMAND: &str = "antigravity-hook";

fn hook_command(exe_path: &str, windows: bool, sub: &str) -> Option<String> {
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
    Some(format!("{exe} {sub}"))
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
    // 両者で引用の規則が違うので、どちらのシェルでも同じように読める引用なしの語だけで書く。
    // plain は marimo-hook のパスをそう書いた一語で、そう書けないときは None になる。
    // 既存のコマンドも、同じように読める語を空白一つずつで並べたものに限って、sh -c を介さず
    // そのまま -- の後ろに続けてラップする。
    ShellAgnostic { plain: Option<String> },
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
            status_line: StatusLinePolicy::ShellAgnostic {
                plain: shell_agnostic_path(exe_path, home),
            },
            raw,
        }
    }

    /// install が Claude Code の各イベントに足すハンドラ。
    pub fn hook_handler(&self) -> Value {
        json!({
            "type": "command",
            "command": self.exe,
            "args": HOOK_ARGS,
            "timeout": HOOK_TIMEOUT_SECS,
        })
    }

    // 今の場所の marimo-hook を exec form で呼ぶハンドラ。
    fn is_current_handler(&self, handler: &Value) -> bool {
        handler.get("command").and_then(Value::as_str) == Some(self.exe.as_str())
            && handler.get("args") == Some(&json!(HOOK_ARGS))
    }

    // 利用者が足した項目とキーの順序を残したまま、command と args だけを今の場所の exec form へ書き換える。
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
            StatusLinePolicy::ShellAgnostic { plain } => {
                plain.as_ref().map(|p| format!("{p} statusline"))
            }
        }
    }

    fn wrapped_statusline(&self, original: &str, lang: Lang) -> Result<String, String> {
        match &self.status_line {
            StatusLinePolicy::Wrap => Ok(format!(
                "{} statusline -- sh -c {}",
                self.quoted,
                quote_always(original)
            )),
            StatusLinePolicy::ShellAgnostic { plain: None } => Err(match lang {
                Lang::Ja => format!(
                    "marimo-hook のパスに空白などが含まれ、Git Bash と PowerShell のどちらでも同じように読める形で書けないため、既存の statusLine を書き換えませんでした。{LIMITS_HINT_JA}"
                ),
                Lang::En => format!(
                    "Left the existing statusLine unchanged because the marimo-hook path contains spaces or other characters that cannot be written so that Git Bash and PowerShell read it the same way. {LIMITS_HINT_EN}"
                ),
            }),
            StatusLinePolicy::ShellAgnostic { plain: Some(p) } => {
                if plain_words(original) {
                    Ok(format!("{p} statusline -- {original}"))
                } else {
                    Err(match lang {
                        Lang::Ja => format!(
                            "既存の statusLine に Git Bash と PowerShell で読み方の違う引用符や記号が含まれるため、書き換えませんでした。{LIMITS_HINT_JA}"
                        ),
                        Lang::En => format!(
                            "Left the existing statusLine unchanged because it contains quotes or symbols that Git Bash and PowerShell read differently. {LIMITS_HINT_EN}"
                        ),
                    })
                }
            }
        }
    }

    // wrapped_statusline がラップする前の元のコマンドを返す。Windows でも、以前のバージョンは sh -c の形で
    // ラップしていたので、その形も読む。
    fn unwrapped_statusline(&self, rest: &str) -> Option<String> {
        let tail = rest.strip_prefix(" -- ")?;
        if matches!(self.status_line, StatusLinePolicy::ShellAgnostic { .. }) && plain_words(tail) {
            return Some(tail.to_owned());
        }
        tail.strip_prefix("sh -c ").and_then(unquote_single_word)
    }

    // statusLine のコマンドから、marimo-hook statusline のラップを外側から順にはがす。別の場所の marimo-hook で
    // 登録したものや、二重にラップしたものも、元のコマンドまでたどる。
    fn peel_statusline(&self, command: &str) -> Peeled {
        let mut current = command.to_owned();
        let mut outer_current = None;
        for depth in 0..MAX_STATUSLINE_LAYERS {
            let layer = match self.statusline_rest(&current) {
                Some(rest) => Some((true, rest.to_owned())),
                None => marimo_statusline_rest(&current).map(|rest| (false, rest.to_owned())),
            };
            let Some((is_current, rest)) = layer else {
                return match outer_current {
                    None => Peeled::NotMarimo,
                    Some(outer_current) => Peeled::Marimo {
                        outer_current,
                        nested: depth > 1,
                        original: Some(current),
                    },
                };
            };
            let outer_current = *outer_current.get_or_insert(is_current);
            if rest.is_empty() {
                return Peeled::Marimo {
                    outer_current,
                    nested: depth > 0,
                    original: None,
                };
            }
            match self.unwrapped_statusline(&rest) {
                Some(original) => current = original,
                None => return Peeled::Unreadable { outer_current },
            }
        }
        Peeled::Unreadable {
            outer_current: outer_current.unwrap_or(false),
        }
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
        if let StatusLinePolicy::ShellAgnostic { plain: Some(p) } = &self.status_line {
            spellings.push(p);
        }
        let rest = self
            .args_after(command, &spellings)?
            .strip_prefix(" statusline")?;
        (rest.is_empty() || rest.starts_with(" -- ")).then_some(rest)
    }
}

// 二重を超えてラップされていても元のコマンドまでたどれるよう、はがす層の数に余裕を持たせる。
const MAX_STATUSLINE_LAYERS: usize = 8;

enum Peeled {
    NotMarimo,
    // marimo-hook statusline で始まるが、marimo がラップした形として読み取れない。
    Unreadable {
        outer_current: bool,
    },
    // outer_current は一番外側の marimo-hook が今の場所のものか、nested はラップが二重以上か。
    // original はすべての層をはがした元のコマンドで、marimo だけだったときは None になる。
    Marimo {
        outer_current: bool,
        nested: bool,
        original: Option<String>,
    },
}

// 置き場所によらず、ファイル名が marimo-hook の実行ファイルで始まる statusline のコマンドなら、その後ろを返す。
fn marimo_statusline_rest(command: &str) -> Option<&str> {
    let (program, rest) = split_first_word(command)?;
    if !is_marimo_program(&program) {
        return None;
    }
    let rest = rest.strip_prefix(" statusline")?;
    (rest.is_empty() || rest.starts_with(" -- ")).then_some(rest)
}

// シェルの一語目を、単一引用符の区間と \ で逃がした文字を読み解いて返す。marimo は二重引用符で書かないので、
// 二重引用符を含む語は読まない。
fn split_first_word(command: &str) -> Option<(String, &str)> {
    let mut word = String::new();
    let mut chars = command.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            ' ' => return Some((word, &command[i..])),
            '\'' => loop {
                match chars.next()?.1 {
                    '\'' => break,
                    ch => word.push(ch),
                }
            },
            '\\' => word.push(chars.next()?.1),
            '"' => return None,
            _ => word.push(c),
        }
    }
    Some((word, ""))
}

// パスの最後の部分が marimo-hook（Windows では marimo-hook.exe）なら、どこに置いたものでも marimo とみなす。
fn is_marimo_program(path: &str) -> bool {
    matches!(
        path.rsplit(['/', '\\']).next(),
        Some("marimo-hook" | "marimo-hook.exe")
    )
}

/// Claude Code の settings.json にある marimo のハンドラかどうか。marimo-hook に hook だけを渡す exec form と、
/// シェルに `<marimo-hook のパス> hook` を渡す以前の shell form のどちらも、置き場所によらず marimo のものとみなす。
/// args を持つハンドラは command をシェルに渡さないので、shell form と同じ文字列でも marimo ではない。
fn is_marimo_claude_handler(handler: &Value) -> bool {
    let Some(command) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    match handler.get("args") {
        Some(args) => args == &json!(HOOK_ARGS) && is_marimo_program(command),
        None => command.strip_suffix(" hook").is_some_and(|program| {
            is_marimo_program(&unquote_single_word(program).unwrap_or_else(|| program.to_owned()))
        }),
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
// PowerShell は ‘ ’ ‚ ‛ “ ” „ を引用符として、– — ― をハイフンとして読むので、ASCII 以外でもこれらは避ける。
fn plain_word(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('~')
        && s.chars().all(|c| {
            if c.is_ascii() {
                c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ':' | '~')
            } else {
                !c.is_whitespace()
                    && !matches!(c, '\u{2013}'..='\u{2015}' | '\u{2018}'..='\u{201E}')
            }
        })
}

// 空白一つずつで区切った plain_word の並びなら、どちらのシェルも同じ語に分けて同じ引数を渡す。
// 先頭や末尾の空白と続いた空白は、分けたときに空の語になるので受け付けない。
fn plain_words(s: &str) -> bool {
    s.split(' ').all(plain_word)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusLineChange {
    Added { after: String },
    Wrapped { before: String, after: String },
    // 別の場所の marimo-hook を通るものや二重のラップを、今の marimo-hook の一重のラップへ書き換える。
    Replaced { before: String, after: String },
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
                StatusLineChange::Added { .. }
                    | StatusLineChange::Wrapped { .. }
                    | StatusLineChange::Replaced { .. }
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

pub fn install(settings: &mut Value, marimo: &Marimo, lang: Lang) -> Result<InstallReport, String> {
    let root = settings.as_object_mut().ok_or(lang.pick(
        "settings.json の最上位が JSON のオブジェクトではありません",
        "The top level of settings.json is not a JSON object",
    ))?;
    check_hooks_shape(root, &EVENTS, lang)?;

    let mut added = Vec::new();
    let mut migrated = Vec::new();
    let mut already = Vec::new();
    for event in EVENTS {
        match root
            .get_mut("hooks")
            .and_then(|h| h.get_mut(event))
            .and_then(Value::as_array_mut)
            .map(|groups| replace_stale_handlers(groups, marimo))
        {
            Some(Existing::Rewritten) => migrated.push(event),
            Some(Existing::Current) => already.push(event),
            Some(Existing::None) | None => {
                push_group(root, event, marimo.hook_handler(), lang)?;
                added.push(event);
            }
        }
    }

    let status_line = install_status_line(root, marimo, lang);
    Ok(InstallReport {
        added,
        migrated,
        already,
        status_line,
    })
}

enum Existing {
    None,
    Current,
    Rewritten,
}

// 一つのイベントのグループの中の marimo のハンドラを、今の場所の exec form の一つにまとめる。
// 今の形のハンドラがあれば、ほかの marimo のハンドラ（以前の shell form や、別の場所の marimo-hook を
// 指すもの）を取り除く。なければ最初の一つを同じグループの同じ位置で今の形に書き換え、残りを取り除く。
// 取り除いて空になったグループは、グループごと取り除く。
fn replace_stale_handlers(groups: &mut Vec<Value>, marimo: &Marimo) -> Existing {
    let has_current = groups.iter().any(|g| {
        g.get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hs| hs.iter().any(|h| marimo.is_current_handler(h)))
    });
    let mut kept = has_current;
    let mut rewritten = false;
    groups.retain_mut(|group| {
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        let n = handlers.len();
        let mut removed = false;
        handlers.retain_mut(|handler| {
            if marimo.is_current_handler(handler) || !is_marimo_claude_handler(handler) {
                return true;
            }
            rewritten = true;
            if kept {
                removed = true;
                return false;
            }
            kept = true;
            if let Some(stale) = handler.as_object() {
                *handler = marimo.migrated(stale);
            }
            true
        });
        !(removed && n > 0 && handlers.is_empty())
    });
    match (rewritten, kept) {
        (true, _) => Existing::Rewritten,
        (false, true) => Existing::Current,
        (false, false) => Existing::None,
    }
}

fn push_group(
    root: &mut Map<String, Value>,
    event: &str,
    handler: Value,
    lang: Lang,
) -> Result<(), String> {
    root.entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| not_an_object(lang))?
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| not_an_array(event, lang))?
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
pub fn install_codex(
    file: &mut Value,
    command: &str,
    lang: Lang,
) -> Result<CodexInstallReport, String> {
    let root = file
        .as_object_mut()
        .ok_or_else(|| hooks_json_not_an_object(lang))?;
    check_hooks_shape(root, &CODEX_EVENTS, lang)?;
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
        push_group(root, event, handler, lang)?;
        added.push(event);
    }
    Ok(CodexInstallReport { added, already })
}

pub fn uninstall_codex(file: &mut Value, lang: Lang) -> Result<Vec<String>, String> {
    let root = file
        .as_object_mut()
        .ok_or_else(|| hooks_json_not_an_object(lang))?;
    check_hooks_shape(root, &CODEX_EVENTS, lang)?;
    Ok(remove_handlers(root, is_marimo_codex_handler))
}

fn is_codex_handler(handler: &Value, command: &str) -> bool {
    handler.get("type").and_then(Value::as_str) == Some("command")
        && handler.get("command").and_then(Value::as_str) == Some(command)
}

// codex_hook_command が書いた形、つまり一語の marimo-hook のパスに codex-hook が続くものだけを
// marimo のものとみなす。どこに置いた marimo-hook でも当たるので、古い場所の登録も見つかる。
fn is_marimo_codex_handler(handler: &Value) -> bool {
    handler.get("type").and_then(Value::as_str) == Some("command")
        && is_marimo_handler(handler, "codex-hook")
}

// agy の handler は type を省略でき、そのときは command として扱われる（docs/hooks.md）。
fn is_marimo_handler(handler: &Value, sub: &str) -> bool {
    let kind = handler.get("type").and_then(Value::as_str);
    let Some(program) = handler
        .get("command")
        .and_then(Value::as_str)
        .filter(|_| kind.is_none_or(|k| k == "command"))
        .and_then(|c| c.strip_suffix(sub))
        .and_then(|c| c.strip_suffix(' '))
    else {
        return false;
    };
    is_marimo_program(&unquote_single_word(program).unwrap_or_else(|| program.to_owned()))
}

/// marimo が agy の hooks.json に足す、名前付きのフックの名前。
pub const ANTIGRAVITY_HOOK_NAME: &str = "marimo";

/// 状態を決めるのに使うイベント。ツールのイベントは matcher を持つグループに入れる。
const ANTIGRAVITY_EVENTS: [(&str, bool); 4] = [
    ("PreInvocation", false),
    ("PreToolUse", true),
    ("PostToolUse", true),
    ("Stop", false),
];

// Gemini のデスクトップアプリが動かす agy は入力に hookEventName を入れてこないので、イベント名を
// コマンドの引数で渡す。フックは agy の処理を止めて同期で動くので、時間の上限を短くする。
fn antigravity_entry(command: &str) -> Value {
    let events = ANTIGRAVITY_EVENTS.map(|(event, grouped)| {
        let handler =
            json!({ "type": "command", "command": format!("{command} {event}"), "timeout": 10 });
        let list = if grouped {
            json!([{ "matcher": "*", "hooks": [handler] }])
        } else {
            json!([handler])
        };
        (event.to_owned(), list)
    });
    Value::Object(events.into_iter().collect())
}

// イベントの配列の要素は、handler そのもの（SessionStart などの flat な形）か、matcher と hooks を持つ
// グループ（PreToolUse と PostToolUse）である。
fn entry_handlers(entry: &Value) -> Vec<&Value> {
    let Some(events) = entry.as_object() else {
        return Vec::new();
    };
    events
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .flat_map(|item| match item.get("hooks").and_then(Value::as_array) {
            Some(hooks) => hooks.iter().collect(),
            None => vec![item],
        })
        .collect()
}

// handler がすべて marimo の antigravity-hook（イベント名の引数があってもなくてもよい）であるものだけを
// marimo のフックとみなす。
fn is_marimo_antigravity_entry(entry: &Value) -> bool {
    let handlers = entry_handlers(entry);
    !handlers.is_empty()
        && handlers.iter().all(|h| {
            let mut bare: Value = (*h).clone();
            if let Some(c) = h.get("command").and_then(Value::as_str) {
                let base = ANTIGRAVITY_EVENTS
                    .iter()
                    .find_map(|(e, _)| c.strip_suffix(e).and_then(|c| c.strip_suffix(' ')))
                    .unwrap_or(c);
                bare["command"] = Value::String(base.to_owned());
            }
            is_marimo_handler(&bare, ANTIGRAVITY_SUBCOMMAND)
        })
}

/// agy の hooks.json に marimo のフックを足す。既に同じものがあれば false を返す。最上位は
/// フックの名前ごとの設定なので、marimo の名前の項目を一つだけ書き、ほかの項目には触らない。
pub fn install_antigravity(file: &mut Value, command: &str, lang: Lang) -> Result<bool, String> {
    let root = file
        .as_object_mut()
        .ok_or_else(|| hooks_json_not_an_object(lang))?;
    let entry = antigravity_entry(command);
    if root.get(ANTIGRAVITY_HOOK_NAME) == Some(&entry) {
        return Ok(false);
    }
    if root
        .get(ANTIGRAVITY_HOOK_NAME)
        .is_some_and(|e| !is_marimo_antigravity_entry(e))
    {
        return Err(match lang {
            Lang::Ja => format!(
                "hooks.json に、marimo のものではない {ANTIGRAVITY_HOOK_NAME:?} という名前のフックがあります"
            ),
            Lang::En => format!(
                "hooks.json already has a hook named {ANTIGRAVITY_HOOK_NAME:?} that is not marimo's"
            ),
        });
    }
    root.retain(|_, e| !is_marimo_antigravity_entry(e));
    root.insert(ANTIGRAVITY_HOOK_NAME.to_owned(), entry);
    Ok(true)
}

/// marimo のフックを取り除き、取り除いた名前を返す。古い場所の marimo-hook を指すものも取り除く。
pub fn uninstall_antigravity(file: &mut Value, lang: Lang) -> Result<Vec<String>, String> {
    let root = file
        .as_object_mut()
        .ok_or_else(|| hooks_json_not_an_object(lang))?;
    let removed: Vec<String> = root
        .iter()
        .filter(|(_, e)| is_marimo_antigravity_entry(e))
        .map(|(k, _)| k.clone())
        .collect();
    root.retain(|_, e| !is_marimo_antigravity_entry(e));
    Ok(removed)
}

const LIMITS_HINT_JA: &str =
    "marimo が statusLine を受け取れないので、Claude Code の利用制限は statusLine からは届きません";
const LIMITS_HINT_EN: &str = "marimo cannot receive the statusLine input, so Claude Code usage limits will not come from the statusLine.";

fn install_status_line(
    root: &mut Map<String, Value>,
    marimo: &Marimo,
    lang: Lang,
) -> StatusLineChange {
    let Some(existing) = root.get_mut("statusLine") else {
        let Some(after) = marimo.statusline_command() else {
            return StatusLineChange::Untouched {
                reason: match lang {
                    Lang::Ja => format!(
                        "marimo-hook のパスに空白などが含まれ、Git Bash と PowerShell のどちらでも同じように読める形で書けないため、statusLine を登録しませんでした。{LIMITS_HINT_JA}"
                    ),
                    Lang::En => format!(
                        "Did not register the statusLine because the marimo-hook path contains spaces or other characters that cannot be written so that Git Bash and PowerShell read it the same way. {LIMITS_HINT_EN}"
                    ),
                },
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
            reason: lang
                .pick(
                    "statusLine が command 形式ではないため、書き換えませんでした",
                    "Left unchanged because the statusLine type is not command",
                )
                .to_owned(),
        };
    };
    let original = match marimo.peel_statusline(&command) {
        Peeled::NotMarimo => {
            let after = match marimo.wrapped_statusline(&command, lang) {
                Ok(after) => after,
                Err(reason) => return StatusLineChange::Untouched { reason },
            };
            existing["command"] = Value::String(after.clone());
            return StatusLineChange::Wrapped {
                before: command,
                after,
            };
        }
        Peeled::Marimo {
            outer_current: true,
            nested: false,
            ..
        }
        | Peeled::Unreadable {
            outer_current: true,
        } => return StatusLineChange::AlreadyMarimo { current: command },
        Peeled::Unreadable {
            outer_current: false,
        } => {
            return StatusLineChange::Untouched {
                reason: lang
                    .pick(
                        "statusLine は別の場所の marimo-hook を通っていて、元のコマンドを読み取れないため、書き換えませんでした",
                        "Left unchanged because the statusLine goes through a marimo-hook at another location and the original command cannot be read",
                    )
                    .to_owned(),
            };
        }
        Peeled::Marimo { original, .. } => original,
    };
    let after = match original {
        Some(original) => marimo.wrapped_statusline(&original, lang),
        // statusline_command が None になるのはパスを書けないときで、そのとき wrapped_statusline も同じ理由で Err を返す。
        None => marimo
            .statusline_command()
            .map_or_else(|| marimo.wrapped_statusline(&command, lang), Ok),
    };
    let after = match after {
        Ok(after) => after,
        Err(reason) => return StatusLineChange::Untouched { reason },
    };
    existing["command"] = Value::String(after.clone());
    StatusLineChange::Replaced {
        before: command,
        after,
    }
}

pub fn uninstall(
    settings: &mut Value,
    marimo: &Marimo,
    lang: Lang,
) -> Result<UninstallReport, String> {
    let root = settings.as_object_mut().ok_or(lang.pick(
        "settings.json の最上位が JSON のオブジェクトではありません",
        "The top level of settings.json is not a JSON object",
    ))?;
    check_hooks_shape(root, &EVENTS, lang)?;
    let removed = remove_handlers(root, is_marimo_claude_handler);
    let status_line = uninstall_status_line(root, marimo, lang);
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

fn uninstall_status_line(
    root: &mut Map<String, Value>,
    marimo: &Marimo,
    lang: Lang,
) -> StatusLineChange {
    let Some(command) = root.get("statusLine").and_then(command_status_line) else {
        return StatusLineChange::Untouched {
            reason: lang
                .pick(
                    "marimo の statusLine は登録されていません",
                    "No marimo statusLine is registered",
                )
                .to_owned(),
        };
    };
    // 別の場所の marimo-hook で登録したものや二重のラップも、すべての層をはがして元に戻す。
    match marimo.peel_statusline(&command) {
        Peeled::NotMarimo => StatusLineChange::Untouched {
            reason: lang
                .pick(
                    "statusLine は marimo のものではないので、そのままにしました",
                    "Left unchanged because the statusLine is not marimo's",
                )
                .to_owned(),
        },
        Peeled::Marimo { original: None, .. } => {
            root.shift_remove("statusLine");
            StatusLineChange::Removed { before: command }
        }
        Peeled::Marimo {
            original: Some(original),
            ..
        } => {
            root["statusLine"]["command"] = Value::String(original.clone());
            StatusLineChange::Restored {
                before: command,
                after: original,
            }
        }
        Peeled::Unreadable { .. } => StatusLineChange::Untouched {
            reason: lang
                .pick(
                    "statusLine は marimo がラップした形として読み取れないので、そのままにしました",
                    "Left unchanged because the statusLine is not in the form marimo wraps",
                )
                .to_owned(),
        },
    }
}

fn check_hooks_shape(root: &Map<String, Value>, events: &[&str], lang: Lang) -> Result<(), String> {
    let Some(hooks) = root.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks.as_object().ok_or_else(|| not_an_object(lang))?;
    for event in events {
        if let Some(v) = hooks.get(*event)
            && !v.is_array()
        {
            return Err(not_an_array(event, lang));
        }
    }
    Ok(())
}

fn not_an_object(lang: Lang) -> String {
    lang.pick(
        "hooks がオブジェクトではありません",
        "hooks is not an object",
    )
    .to_owned()
}

fn not_an_array(event: &str, lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("hooks.{event} が配列ではありません"),
        Lang::En => format!("hooks.{event} is not an array"),
    }
}

fn hooks_json_not_an_object(lang: Lang) -> String {
    lang.pick(
        "hooks.json の最上位が JSON のオブジェクトではありません",
        "The top level of hooks.json is not a JSON object",
    )
    .to_owned()
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

// quote_always の逆。marimo がラップした形だけを読めればよいので、単一引用符の区間と
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
    fn antigravity_hooks_replace_only_marimo_entries() {
        let new = "/new/marimo-hook antigravity-hook";
        let old = "/old/marimo-hook antigravity-hook";
        let other = json!({ "Stop": [{ "command": "./lint.sh" }] });
        let cases = [
            (json!({}), Ok(true), vec!["marimo"]),
            (
                json!({ "marimo": antigravity_entry(new), "lint": other }),
                Ok(false),
                vec!["lint", "marimo"],
            ),
            (
                json!({ "marimo-old": antigravity_entry(old), "lint": other }),
                Ok(true),
                vec!["lint", "marimo"],
            ),
            // イベント名の引数を持たない以前の登録も marimo のものとして置き換える。
            (
                json!({ "marimo": { "Stop": [{ "type": "command", "command": new, "timeout": 10 }] } }),
                Ok(true),
                vec!["marimo"],
            ),
            (json!({ "marimo": other }), Err(()), vec!["marimo"]),
        ];
        for (mut file, changed, names) in cases {
            let before = file.clone();
            assert_eq!(
                install_antigravity(&mut file, new, Lang::Ja).map_err(|_| ()),
                changed,
                "{before}"
            );
            let mut keys: Vec<&str> = file
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort();
            assert_eq!(keys, names, "{before}");
        }
        let mut file = json!({ "marimo": antigravity_entry(new), "lint": other });
        assert_eq!(
            uninstall_antigravity(&mut file, Lang::Ja),
            Ok(vec!["marimo".to_owned()])
        );
        assert_eq!(file, json!({ "lint": other }));
    }

    fn shell_form(command: &str) -> Value {
        json!({"type": "command", "command": command})
    }

    #[test]
    fn recognizes_marimo_commands_by_file_name() {
        for c in [
            "'/Users/a b/.marimo/bin/marimo-hook' hook",
            "/Users/a b/.marimo/bin/marimo-hook hook",
            "/other/marimo-hook hook",
            "'C:/Users/a b/.marimo/bin/marimo-hook.exe' hook",
        ] {
            assert!(is_marimo_claude_handler(&shell_form(c)), "{c}");
        }
        assert!(is_marimo_claude_handler(&exec(
            "/other/marimo-hook",
            json!(["hook"])
        )));
        for c in [
            "'/Users/a b/.marimo/bin/marimo-hook' record Stop",
            "/x/marimo-hook-backup hook",
            "/x/marimo-hooky hook",
            "/x/marimo-hook hook --extra",
            "/x/marimo-hook  hook",
            "'/x y/marimo-hook'-backup hook",
            "/x y/marimo-hook-backup hook",
        ] {
            assert!(!is_marimo_claude_handler(&shell_form(c)), "{c}");
        }
        // args を持つハンドラは command をシェルに渡さないので、shell form と同じ文字列でも marimo ではない。
        assert!(!is_marimo_claude_handler(&exec(
            "/x/marimo-hook hook",
            json!([])
        )));
        assert!(!is_marimo_claude_handler(&exec(
            "/x/marimo-hook",
            json!(["record", "Stop"])
        )));

        let m = Marimo::new("/x/marimo-hook");
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
        install(&mut settings, &m, Lang::Ja).unwrap();
        let hooks = settings["hooks"].as_object_mut().unwrap();
        hooks.remove("SubagentStart");
        hooks.remove("SubagentStop");
        assert_eq!(hooks.len(), 14);
        let before = settings.clone();

        let report = install(&mut settings, &m, Lang::Ja).unwrap();
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
        install(&mut settings, &m, Lang::Ja).unwrap();
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0],
            exec(r"C:\Users\a b\.marimo\bin\marimo-hook.exe", json!(["hook"]))
        );
    }

    // groups の中の marimo のハンドラ。
    fn marimo_handlers(groups: &Value) -> Vec<&Value> {
        groups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g["hooks"].as_array())
            .flatten()
            .filter(|h| is_marimo_claude_handler(h))
            .collect()
    }

    fn has_handler(groups: &Value, handler: &Value) -> bool {
        groups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g["hooks"].as_array())
            .flatten()
            .any(|h| h == handler)
    }

    // statusLine が今の marimo-hook の一重のラップで、元のコマンドが original であるか。
    fn wraps_once(m: &Marimo, command: &str, original: &str) -> bool {
        matches!(
            m.peel_statusline(command),
            Peeled::Marimo { outer_current: true, nested: false, original: Some(o) } if o == original
        )
    }

    #[test]
    fn install_and_uninstall_handle_marimo_hook_at_another_location() {
        let m = Marimo::new("/x/marimo-hook");
        let mut settings = json!({
            "hooks": {"Stop": [
                {"hooks": [exec("/old/marimo-hook", json!(["hook"])), shell_form("echo done")]},
                {"hooks": [shell_form("/older/marimo-hook hook")]}
            ]},
            "statusLine": {"type": "command", "command": "/old/marimo-hook statusline -- sh -c 'echo hi'"}
        });
        let report = install(&mut settings, &m, Lang::Ja).unwrap();
        assert_eq!(report.migrated, ["Stop"]);
        // 古い場所のハンドラは今の場所の一つにまとめ、利用者のハンドラは残す。
        let marimo = marimo_handlers(&settings["hooks"]["Stop"]);
        assert_eq!(marimo.len(), 1);
        assert!(m.is_current_handler(marimo[0]));
        assert!(has_handler(
            &settings["hooks"]["Stop"],
            &shell_form("echo done")
        ));
        assert!(matches!(
            report.status_line,
            StatusLineChange::Replaced { .. }
        ));
        let command = settings["statusLine"]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(wraps_once(&m, &command, "echo hi"), "{command}");

        // 二重にラップされたものも、元のコマンドを今の marimo-hook で一重にラップし直す。
        let double = format!(
            "/x/marimo-hook statusline -- sh -c {}",
            quote_always("/old/marimo-hook statusline -- sh -c 'echo hi'")
        );
        settings["statusLine"]["command"] = json!(double);
        let report = install(&mut settings, &m, Lang::Ja).unwrap();
        assert!(report.changed());
        let command = settings["statusLine"]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(wraps_once(&m, &command, "echo hi"), "{command}");

        // uninstall は、別の場所の marimo-hook を指すものも含めて取り除き、元の statusLine へ戻す。
        settings["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(shell_form("/older/marimo-hook hook"));
        settings["statusLine"]["command"] = json!("/old/marimo-hook statusline -- sh -c 'echo hi'");
        let report = uninstall(&mut settings, &m, Lang::Ja).unwrap();
        assert!(report.removed.contains(&"Stop".to_owned()));
        assert!(marimo_handlers(&settings["hooks"]["Stop"]).is_empty());
        assert!(has_handler(
            &settings["hooks"]["Stop"],
            &shell_form("echo done")
        ));
        assert_eq!(settings["statusLine"]["command"], "echo hi");
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
    fn windows_status_line_is_written_only_in_words_both_shells_read_alike() {
        let m = Marimo::windows(
            r"C:\Users\user\.marimo\bin\marimo-hook.exe",
            Some(r"C:\Users\user"),
        );
        let mut settings = json!({"model": "opus"});
        let report = install(&mut settings, &m, Lang::Ja).unwrap();
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
        let again = install(&mut settings, &m, Lang::Ja).unwrap();
        assert!(!again.changed());
        assert_eq!(
            again.status_line,
            StatusLineChange::AlreadyMarimo {
                current: command.to_owned()
            }
        );
        assert_eq!(settings, before);

        let report = uninstall(&mut settings, &m, Lang::Ja).unwrap();
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
        install(&mut settings, &outside, Lang::Ja).unwrap();
        assert_eq!(
            settings["statusLine"]["command"],
            "D:/tools/marimo-hook.exe statusline"
        );
        uninstall(&mut settings, &outside, Lang::Ja).unwrap();
        assert_eq!(settings, json!({}));

        // 既存の statusLine は、どちらのシェルでも同じ語に分かれる場合だけ -- の後ろへそのまま続けてラップし、
        // uninstall で元に戻す。sh -c で始まっていても、引用符が無ければ元のコマンドの一部として扱う。
        let exe = "~/.marimo/bin/marimo-hook.exe";
        let cases = [
            ("bash C:/Users/user/.claude/statusline.sh", true),
            ("statusline.exe", true),
            ("sh -c statusline", true),
            ("bash ~/.claude/statusline.sh", false),
            ("bash 'C:/Users/user/.claude/statusline.sh'", false),
            (r#"bash "C:/Users/user/.claude/statusline.sh""#, false),
            (r"bash C:\Users\user\.claude\statusline.sh", false),
            ("echo $HOME", false),
            ("cat | head -1", false),
            ("bash C:/Users/user/\u{201C}a\u{201D}.sh", false),
            ("bash C:/Users/user/a.sh \u{2013}x", false),
            ("bash  statusline.sh", false),
            (" bash statusline.sh", false),
            ("bash statusline.sh ", false),
            ("", false),
        ];
        for (command, wrapped) in cases {
            let original = json!({"statusLine": {"type": "command", "command": command}});
            let mut settings = original.clone();
            let report = install(&mut settings, &m, Lang::Ja).unwrap();
            let after = format!("{exe} statusline -- {command}");
            if wrapped {
                assert_eq!(
                    report.status_line,
                    StatusLineChange::Wrapped {
                        before: command.to_owned(),
                        after: after.clone()
                    }
                );
                assert_eq!(settings["statusLine"]["command"], after.as_str());
                let before = settings.clone();
                let again = install(&mut settings, &m, Lang::Ja).unwrap();
                assert_eq!(
                    again.status_line,
                    StatusLineChange::AlreadyMarimo {
                        current: after.clone()
                    }
                );
                assert_eq!(settings, before);
            } else {
                let StatusLineChange::Untouched { reason } = &report.status_line else {
                    panic!("{command:?}: {:?}", report.status_line);
                };
                assert!(reason.contains("引用符や記号"), "{command:?}: {reason}");
                assert!(
                    reason.contains("利用制限は statusLine からは届きません"),
                    "{reason}"
                );
                assert_eq!(settings["statusLine"], original["statusLine"]);
            }
            let report = uninstall(&mut settings, &m, Lang::Ja).unwrap();
            if wrapped {
                assert_eq!(
                    report.status_line,
                    StatusLineChange::Restored {
                        before: after,
                        after: command.to_owned()
                    }
                );
            }
            assert_eq!(settings, original, "{command:?}");
        }

        // 以前のバージョンが Windows でも sh -c の形でラップしていたものは、その形のまま元に戻す。
        let legacy = "C:/Users/user/.marimo/bin/marimo-hook.exe statusline -- sh -c 'bash ~/.claude/statusline.sh'";
        let mut settings = json!({"statusLine": {"type": "command", "command": legacy}});
        let report = install(&mut settings, &m, Lang::Ja).unwrap();
        assert!(matches!(
            report.status_line,
            StatusLineChange::AlreadyMarimo { .. }
        ));
        uninstall(&mut settings, &m, Lang::Ja).unwrap();
        assert_eq!(
            settings["statusLine"]["command"],
            "bash ~/.claude/statusline.sh"
        );

        // どちらのシェルでも同じに読める形で書けないパスでは、statusLine を登録しない。
        let spaced = Marimo::windows(r"D:\My Tools\marimo-hook.exe", Some(r"C:\Users\user"));
        let mut settings = json!({});
        let report = install(&mut settings, &spaced, Lang::Ja).unwrap();
        let StatusLineChange::Untouched { reason } = &report.status_line else {
            panic!("{:?}", report.status_line);
        };
        assert!(
            reason.contains("利用制限は statusLine からは届きません"),
            "{reason}"
        );
        assert!(settings.get("statusLine").is_none());
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["command"],
            r"D:\My Tools\marimo-hook.exe"
        );
        // 既存の statusLine もラップできない。
        let original =
            json!({"statusLine": {"type": "command", "command": "bash C:/statusline.sh"}});
        let mut settings = original.clone();
        let report = install(&mut settings, &spaced, Lang::Ja).unwrap();
        let StatusLineChange::Untouched { reason } = &report.status_line else {
            panic!("{:?}", report.status_line);
        };
        assert!(reason.contains("marimo-hook のパス"), "{reason}");
        assert_eq!(settings["statusLine"], original["statusLine"]);
    }

    #[test]
    fn reasons_and_errors_follow_the_language() {
        let home = Some(r"C:\Users\user");
        let spaced = Marimo::windows(r"D:\My Tools\marimo-hook.exe", home);
        let m = Marimo::windows(r"C:\Users\user\.marimo\bin\marimo-hook.exe", home);
        let status = |command: &str| json!({"statusLine": {"type": "command", "command": command}});
        let cases = [
            (&spaced, json!({}), "Did not register the statusLine"),
            (
                &spaced,
                status("bash C:/statusline.sh"),
                "Left the existing statusLine unchanged because the marimo-hook path",
            ),
            (&m, status("echo $HOME"), "quotes or symbols"),
            (
                &m,
                json!({"statusLine": {"type": "static"}}),
                "type is not command",
            ),
        ];
        for (marimo, mut settings, want) in cases {
            let report = install(&mut settings, marimo, Lang::En).unwrap();
            let StatusLineChange::Untouched { reason } = &report.status_line else {
                panic!("{:?}", report.status_line);
            };
            assert!(reason.contains(want), "{reason}");
            assert!(reason.is_ascii(), "{reason}");
        }

        let cases = [
            (json!({}), "No marimo statusLine is registered"),
            (status("bash statusline.sh"), "is not marimo's"),
        ];
        for (mut settings, want) in cases {
            let report = uninstall(&mut settings, &m, Lang::En).unwrap();
            let StatusLineChange::Untouched { reason } = &report.status_line else {
                panic!("{:?}", report.status_line);
            };
            assert!(reason.contains(want), "{reason}");
        }

        let err = install(&mut json!([]), &m, Lang::En).unwrap_err();
        assert_eq!(err, "The top level of settings.json is not a JSON object");
        let err = install(&mut json!({"hooks": {"Stop": {}}}), &m, Lang::En).unwrap_err();
        assert_eq!(err, "hooks.Stop is not an array");
        let err = install_antigravity(&mut json!({"marimo": {}}), "x", Lang::En).unwrap_err();
        assert!(err.contains("not marimo's"), "{err}");
    }
}
