use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;

use marimo_core::time::now_ms;
use marimo_core::{
    HookInput, HostProcess, MarimoHome, Provider, SessionState, codex, hermes, process, store,
    transcript,
};

mod install;
mod origin;
mod settings_edit;

// hook、codex-hook、hermes-hook、statusline、record は、marimo の内部で何が失敗しても終了コード 0 で抜ける。
// フックの終了コードと stdout は Claude Code に読まれ、終了コード 2 は操作を止め、
// UserPromptSubmit や SessionStart の stdout は Claude の文脈に加わるためである。
// Codex も同じで、終了コード 0 で stdout が空なら、PermissionRequest では判断を示さず通常の承認へ進み、
// PreToolUse、UserPromptSubmit、Stop では何も止めない（codex-rs/hooks/src/events の
// permission_request.rs、pre_tool_use.rs、user_prompt_submit.rs、stop.rs の parse_completed）。
// Hermes も、pre_tool_call の終了コード 2 と stdout の JSON で操作を止め、pre_llm_call の stdout の
// context を会話に加える（agent/shell_hooks.py の _parse_response）。
// install と uninstall は利用者が端末で実行するので、失敗は非 0 と stderr で知らせる。
fn main() -> ExitCode {
    // 既定の panic メッセージは stderr に出るだけで害はないが、Claude Code の
    // デバッグログに紛らわしい行を残さないよう一行にまとめる。
    panic::set_hook(Box::new(|info| {
        let _ = writeln!(io::stderr(), "marimo-hook: panic: {info}");
    }));
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let interactive = matches!(
        args.first().and_then(|a| a.to_str()),
        Some("install" | "uninstall" | "cloud-snapshot")
    );
    let on_panic = if interactive {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    };
    panic::catch_unwind(|| run(&args)).unwrap_or(on_panic)
}

fn run(args: &[OsString]) -> ExitCode {
    let sub = args.first().and_then(|a| a.to_str()).unwrap_or("");
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "hook" => {
            report(hook(Provider::Claude));
            ExitCode::SUCCESS
        }
        "codex-hook" => {
            report(hook(Provider::Codex));
            ExitCode::SUCCESS
        }
        "hermes-hook" => {
            report(hermes_hook());
            ExitCode::SUCCESS
        }
        "statusline" => statusline(rest),
        "cloud-snapshot" => interactive(cloud_snapshot(rest)),
        "install" => interactive(install::run(install::Mode::Install, rest)),
        "uninstall" => interactive(install::run(install::Mode::Uninstall, rest)),
        "record" => {
            let label = rest.first().map(|l| l.to_string_lossy().into_owned());
            report(record(label.as_deref().unwrap_or("")));
            ExitCode::SUCCESS
        }
        _ => {
            report(Err(format!("unknown subcommand {sub:?}")));
            ExitCode::SUCCESS
        }
    }
}

fn cloud_snapshot(rest: &[OsString]) -> Result<(), String> {
    let home = home()?;
    if rest.first().and_then(|a| a.to_str()) == Some("--clear") && rest.len() == 1 {
        return marimo_core::cloud::clear(&home).map_err(|e| e.to_string());
    }
    if !rest.is_empty() {
        return Err("usage: cloud-snapshot [--clear]; read metadata JSON from stdin".into());
    }
    marimo_core::cloud::import(&home, &read_stdin(), now_ms())
}

fn interactive(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            let _ = writeln!(io::stderr(), "marimo-hook: {msg}");
            ExitCode::FAILURE
        }
    }
}

// フックの stderr は exit 0 のとき Claude Code のデバッグログにだけ残るので、
// 失敗の理由はそこへ書く。
fn report(result: Result<(), String>) {
    if let Err(msg) = result {
        let _ = writeln!(io::stderr(), "marimo-hook: {msg}");
    }
}

fn read_stdin() -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = io::stdin().lock().read_to_end(&mut buf);
    buf
}

fn home() -> Result<MarimoHome, String> {
    MarimoHome::resolve().ok_or_else(|| "cannot resolve marimo home".to_owned())
}

fn hook(provider: Provider) -> Result<(), String> {
    let input = read_stdin();
    let mut parsed: HookInput =
        serde_json::from_slice(&input).map_err(|e| format!("invalid hook input: {e}"))?;
    parsed.provider = provider;
    // 会話ログが読めなくても、状態の更新は続ける。
    let log = parsed
        .transcript_path
        .as_deref()
        .filter(|_| parsed.wants_transcript_usage())
        .map(std::path::Path::new);
    let (transcript, rollout) = match provider {
        Provider::Claude => (log.and_then(|p| transcript::read_tail(p).ok()), None),
        Provider::Codex => (
            None,
            log.and_then(|p| codex::read_rollout_tail(p, now_ms()).ok()),
        ),
        Provider::Hermes => (None, None),
    };
    let title = codex::codex_home()
        .filter(|_| parsed.wants_codex_title())
        .and_then(|home| codex::thread_name(&home, &parsed.session_id));
    let codex_rollout = parsed
        .transcript_path
        .as_deref()
        .filter(|_| provider == Provider::Codex)
        .map(std::path::Path::new);
    let home = home()?;
    let current = store::read_session(&home, provider, &parsed.session_id);
    // サブエージェントのフックで親の行を書くのは承認待ちの出入りだけなので、起動元の手がかりは
    // 親の会話のフックからだけ取る。
    let extras = store::HookExtras {
        origin: parsed
            .subagent()
            .is_none()
            .then(|| origin::detect(&parsed.hook_event_name, codex_rollout)),
        transcript,
        rollout,
        title,
        host: host(&parsed.hook_event_name, current.as_ref()),
        ..store::HookExtras::default()
    };
    store::apply_hook(&home, &parsed, &extras).map_err(|e| format!("write failed: {e}"))
}

// Windows で祖先をたどるには全プロセスの一覧を取るので、ターンの始まりと、まだ記録の無い行でだけ調べる。
// --resume や、Hermes の再起動の後に再開したターンは、新しいプロセスで始まるのでここで置き換わる。
fn host(event: &str, current: Option<&SessionState>) -> Option<HostProcess> {
    let starts = matches!(event, "SessionStart" | "UserPromptSubmit");
    (starts || current.is_none_or(|c| c.host.is_none()))
        .then(process::host)
        .flatten()
}

// Hermes の入力は Claude Code の入力に読み替えてから、同じ規則で状態を決める。Hermes は gateway の
// 裏で動き、フックの祖先のプロセスは会話のウィンドウではないので、起動元の手がかりは取らない。
fn hermes_hook() -> Result<(), String> {
    let input = read_stdin();
    let raw: hermes::HermesInput =
        serde_json::from_slice(&input).map_err(|e| format!("invalid hook input: {e}"))?;
    let home = home()?;
    let hermes_home = hermes::hermes_home();
    let current = store::read_session(&home, Provider::Hermes, &raw.session_id);
    match hermes::translate(&raw, current.as_ref(), hermes_home.as_deref(), now_ms()) {
        hermes::Event::Hook(parsed) => {
            let starts = matches!(
                parsed.hook_event_name.as_str(),
                "SessionStart" | "UserPromptSubmit"
            );
            // 題名とリンクはターンの始まりに読み直す。会話の一覧はターンの前に gateway が書く。
            let meta = hermes_home
                .as_deref()
                .filter(|_| starts)
                .map(|h| hermes::chat_meta(h, &parsed.session_id))
                .unwrap_or_default();
            let extras = store::HookExtras {
                title: meta.title,
                link: meta.link,
                host: host(&parsed.hook_event_name, current.as_ref()),
                ..store::HookExtras::default()
            };
            store::apply_hook(&home, &parsed, &extras)
        }
        hermes::Event::Context(context) => {
            store::apply_context(&home, Provider::Hermes, &raw.session_id, context)
        }
        hermes::Event::Ignore => Ok(()),
    }
    .map_err(|e| format!("write failed: {e}"))
}

fn record(label: &str) -> Result<(), String> {
    let input = read_stdin();
    store::append_record(&home()?, label, &input).map_err(|e| format!("record failed: {e}"))
}

fn statusline(rest: &[OsString]) -> ExitCode {
    let command = match rest.first().and_then(|a| a.to_str()) {
        Some("--") => &rest[1..],
        _ => rest,
    };
    let input = read_stdin();

    // 元のコマンドを先に起動して入力を渡してから保存に移る。保存がどう失敗しても
    // 受け渡しが済んでいるようにするためと、両者を並行させて待ち時間を縮めるためである。
    let child = spawn_original(command, &input);

    let saved = panic::catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        let value: serde_json::Value =
            serde_json::from_slice(&input).map_err(|e| format!("invalid statusline input: {e}"))?;
        store::apply_statusline(&home()?, &value).map_err(|e| format!("write failed: {e}"))
    }));
    match saved {
        Ok(result) => report(result),
        Err(_) => report(Err("statusline save panicked".to_owned())),
    }

    match child {
        Some(child) => wait_original(child),
        None => ExitCode::SUCCESS,
    }
}

fn spawn_original(command: &[OsString], input: &[u8]) -> Option<Child> {
    let (program, args) = command.split_first()?;
    #[cfg(windows)]
    let program = &resolve_program(program);
    // stdout と stderr は継承し、元のコマンドの出力を一切加工せずに Claude Code へ届ける。
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| report(Err(format!("cannot start statusline command: {e}"))))
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_vec();
        // 子が stdin を読まずにいてもパイプの容量で詰まらないよう、別スレッドで書く。
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    Some(child)
}

// std の Command は Windows では PATH より先に実行ファイルのフォルダや System32 を探すので、
// WSL が入っていると bash が System32 の bash.exe になり、Git Bash や PowerShell が同じ名前で
// 起動するものと食い違う。そこで名前だけのプログラムは PATH の順に自分で探す。
#[cfg(windows)]
fn resolve_program(program: &std::ffi::OsStr) -> OsString {
    let found = program.to_str().and_then(|name| {
        let path = std::env::var("PATH").ok()?;
        let pathext = std::env::var("PATHEXT").ok();
        find_in_path(name, &path, pathext.as_deref(), |p| p.is_file())
    });
    found.map_or_else(|| program.to_owned(), std::path::PathBuf::into_os_string)
}

// 見つからなければ None を返し、呼び出し側は名前をそのまま std に渡す。
#[cfg(any(windows, test))]
fn find_in_path(
    name: &str,
    path: &str,
    pathext: Option<&str>,
    exists: impl Fn(&std::path::Path) -> bool,
) -> Option<std::path::PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    let exts: Vec<&str> = pathext
        .unwrap_or(".COM;.EXE;.BAT;.CMD")
        .split(';')
        .filter(|e| !e.is_empty())
        .collect();
    let has_ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            exts.iter()
                .any(|x| x.strip_prefix('.').unwrap_or(x).eq_ignore_ascii_case(e))
        });
    path.split(';')
        .map(|dir| dir.replace('"', ""))
        .filter(|dir| !dir.is_empty())
        .find_map(|dir| {
            let dir = std::path::Path::new(&dir);
            let as_is = has_ext.then(|| dir.join(name));
            as_is
                .into_iter()
                .chain(exts.iter().map(|ext| dir.join(format!("{name}{ext}"))))
                .find(|p| exists(p))
        })
}

// 元のコマンドの終了コードはそのまま返す。statusLine は非 0 で終わると表示が空に
// なるので、ここで 0 に揃えると元の見た目が変わってしまう。
fn wait_original(mut child: Child) -> ExitCode {
    match child.wait() {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(code.clamp(0, 255) as u8),
            None => ExitCode::FAILURE,
        },
        Err(e) => {
            report(Err(format!("statusline command wait failed: {e}")));
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn program_is_searched_in_path_order_then_pathext_order() {
        let files = [
            Path::new("git").join("bash.exe"),
            Path::new("system32").join("bash.exe"),
            Path::new("tools").join("run.cmd"),
            Path::new("tools").join("run.bat"),
            Path::new("tools").join("script.sh"),
            Path::new("tools").join("tool.exe"),
            Path::new("tools").join("tool.exe.COM"),
        ];
        // Windows のファイル名は大文字と小文字を区別しない。
        let exists = |p: &Path| {
            files.iter().any(|f| {
                f.to_string_lossy()
                    .eq_ignore_ascii_case(&p.to_string_lossy())
            })
        };
        let pathext = Some(".COM;.EXE;.BAT;.CMD");
        let cases = [
            (
                "bash",
                "git;system32",
                pathext,
                Some(Path::new("git").join("bash.EXE")),
            ),
            (
                "bash",
                "system32;git",
                pathext,
                Some(Path::new("system32").join("bash.EXE")),
            ),
            (
                "bash",
                ";\"git\";;system32",
                pathext,
                Some(Path::new("git").join("bash.EXE")),
            ),
            (
                "run",
                "tools",
                pathext,
                Some(Path::new("tools").join("run.BAT")),
            ),
            (
                "run",
                "tools",
                Some(".CMD;.BAT"),
                Some(Path::new("tools").join("run.CMD")),
            ),
            (
                "tool.exe",
                "tools",
                pathext,
                Some(Path::new("tools").join("tool.exe")),
            ),
            ("script.sh", "tools", pathext, None),
            ("bash", "git", None, Some(Path::new("git").join("bash.EXE"))),
            ("missing", "git;system32;tools", pathext, None),
            ("bash", "", pathext, None),
            ("git/bash.exe", "git", pathext, None),
            (r"git\bash", "git", pathext, None),
        ];
        for (name, path, pathext, expected) in cases {
            assert_eq!(
                find_in_path(name, path, pathext, exists),
                expected,
                "{name} in {path}"
            );
        }
    }
}
