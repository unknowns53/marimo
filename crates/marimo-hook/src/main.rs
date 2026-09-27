use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;

use marimo_core::{HookInput, MarimoHome, store};

mod install;
mod settings_edit;

// hook、statusline、record は、marimo の内部で何が失敗しても終了コード 0 で抜ける。
// フックの終了コードと stdout は Claude Code に読まれ、終了コード 2 は操作を止め、
// UserPromptSubmit や SessionStart の stdout は Claude の文脈に加わるためである。
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
        Some("install" | "uninstall")
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
            report(hook());
            ExitCode::SUCCESS
        }
        "statusline" => statusline(rest),
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

fn hook() -> Result<(), String> {
    let input = read_stdin();
    let parsed: HookInput =
        serde_json::from_slice(&input).map_err(|e| format!("invalid hook input: {e}"))?;
    store::apply_hook(&home()?, &parsed).map_err(|e| format!("write failed: {e}"))
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
