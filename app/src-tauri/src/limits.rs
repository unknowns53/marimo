use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use marimo_core::paths::user_home;
use marimo_core::time::now_ms;
use marimo_core::{MarimoHome, antigravity, codex, store};
use serde_json::{Value, json};

use crate::scale;
use crate::update::hide_console;

const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
const EXIT_GRACE: Duration = Duration::from_secs(3);
const CHECK_INTERVAL: Duration = Duration::from_secs(30);
const PRINT_TIMEOUT: Duration = Duration::from_secs(60);

/// 設定のウィンドウで選んだ再取得の間隔（分）で、保存している値を観測した時刻がその間隔より古いかを見る。
/// フックや statusLine が新しい値を届けている間は起動しない。起動に失敗し続けても間隔より頻繁には
/// 起動しないよう、前回起動した時刻も見る。0 分は再取得しないことを表す。
fn refresh_due(minutes: u32, observed_ms: Option<u64>, last_refresh: Option<Instant>) -> bool {
    let interval = Duration::from_secs(u64::from(minutes) * 60);
    minutes > 0
        && observed_ms.is_none_or(|t| now_ms().saturating_sub(t) >= interval.as_millis() as u64)
        && last_refresh.is_none_or(|t| t.elapsed() >= interval)
}

/// 再取得の間隔を選んでいれば、Codex の利用制限を Codex 自身の `codex app-server` に問い合わせて書く。
/// rollout の token_count は Codex がターンを進めたときにしか書かれないので、使っていない間も値を
/// 新しく保てる。
pub fn spawn_codex(home: MarimoHome) {
    thread::spawn(move || {
        let mut last_refresh = None;
        let mut last_error = None;
        loop {
            // 設定のウィンドウで変えた間隔を次の周期から使うよう、毎回読む。
            let minutes = scale::load_usage_refresh_minutes(&home);
            let observed = store::read_codex_rate_limits(&home).map(|l| l.observed_at);
            if scale::load_usage_tools(&home).codex && refresh_due(minutes, observed, last_refresh)
            {
                last_refresh = Some(Instant::now());
                let names = program_names("codex");
                let read = user_home()
                    .and_then(|h| find_program(&names, &h))
                    .ok_or_else(|| "codex was not found".to_owned())
                    .and_then(|program| read_codex(&program));
                let written = read.and_then(|result| {
                    let limits =
                        codex::rate_limits_from_account(&result, now_ms()).ok_or_else(|| {
                            format!("unexpected account/rateLimits/read result: {result}")
                        })?;
                    store::write_codex_rate_limits(&home, &limits)
                        .map_err(|e| format!("cannot save Codex rate limits: {e}"))
                });
                // 同じ失敗を再取得のたびに書き続けないよう、変わったときだけ書く。
                let error = written.err();
                if let Some(e) = error.as_ref().filter(|e| last_error.as_ref() != Some(*e)) {
                    eprintln!("marimo: cannot read Codex rate limits: {e}");
                }
                last_error = error;
            }
            thread::sleep(CHECK_INTERVAL);
        }
    });
}

/// Claude の利用制限を、Claude Code が `~/.claude.json` の cachedUsageUtilization に残した値で補う。
/// statusLine はデスクトップアプリの Code タブでは呼ばれず、CLI でも応答が届いたときにしか新しい値を
/// 持たない。再取得の間隔を選んでいれば、Claude Code に `/usage` を実行させてこの値を再取得させる。
/// ファイルは Claude Code が頻繁に書き換えるので、更新時刻が変わったときだけ読む。
pub fn spawn_claude(home: MarimoHome) {
    thread::spawn(move || {
        let Some(user_home) = user_home() else {
            return;
        };
        let config = user_home.join(".claude.json");
        let mut seen = None;
        let mut last_refresh = None;
        let mut last_error = None;
        loop {
            let minutes = scale::load_usage_refresh_minutes(&home);
            let observed = store::read_rate_limits(&home).map(|l| l.updated_at);
            if scale::load_usage_tools(&home).claude && refresh_due(minutes, observed, last_refresh)
            {
                last_refresh = Some(Instant::now());
                let names = program_names("claude");
                let error = find_program(&names, &user_home)
                    .ok_or_else(|| "claude was not found".to_owned())
                    .and_then(|program| refresh_claude(&program, home.root()))
                    .err();
                if let Some(e) = error.as_ref().filter(|e| last_error.as_ref() != Some(*e)) {
                    eprintln!("marimo: cannot refresh Claude rate limits: {e}");
                }
                last_error = error;
            }
            let modified = std::fs::metadata(&config).and_then(|m| m.modified()).ok();
            if modified.is_some() && modified != seen {
                seen = modified;
                let limits = store::read_file(&config)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|config| store::rate_limits_from_claude_config(&config));
                if let Some(Err(e)) = limits.map(|l| store::write_rate_limits_if_newer(&home, &l)) {
                    eprintln!("marimo: cannot save Claude rate limits: {e}");
                }
            }
            thread::sleep(CHECK_INTERVAL);
        }
    });
}

/// 再取得の間隔を選んでいれば、Antigravity の利用制限を `agy -p /usage` で問い合わせて書く。
/// agy のフックの入力は利用制限を持たないので、ほかに値が届く経路は無い。
pub fn spawn_antigravity(home: MarimoHome) {
    thread::spawn(move || {
        let mut last_refresh = None;
        let mut last_error = None;
        loop {
            let minutes = scale::load_usage_refresh_minutes(&home);
            let observed = store::read_antigravity_rate_limits(&home).map(|l| l.updated_at);
            if scale::load_usage_tools(&home).antigravity
                && refresh_due(minutes, observed, last_refresh)
            {
                last_refresh = Some(Instant::now());
                let names = program_names("agy");
                let written = user_home()
                    .and_then(|h| find_program(&names, &h))
                    .ok_or_else(|| "agy was not found".to_owned())
                    .and_then(|program| read_antigravity(&program, &home))
                    .and_then(|output| {
                        let limits = antigravity::rate_limits_from_usage(&output, now_ms())
                            .ok_or_else(|| format!("unexpected agy -p /usage output: {output}"))?;
                        store::write_antigravity_rate_limits(&home, &limits)
                            .map_err(|e| format!("cannot save Antigravity rate limits: {e}"))
                    });
                let error = written.err();
                if let Some(e) = error.as_ref().filter(|e| last_error.as_ref() != Some(*e)) {
                    eprintln!("marimo: cannot read Antigravity rate limits: {e}");
                }
                last_error = error;
            }
            thread::sleep(CHECK_INTERVAL);
        }
    });
}

/// `claude -p "/usage"` はモデルへリクエストを送らずに、Claude Code 自身が利用状況を取得して
/// cachedUsageUtilization を書き直す。会話ログを残さないよう --no-session-persistence を付け、marimo の
/// フックがこの呼び出しを行として拾わないよう、フックをすべて止める。
fn refresh_claude(program: &Path, cwd: &Path) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(["-p", "/usage", "--no-session-persistence", "--settings"])
        .arg(r#"{"disableAllHooks": true}"#)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    prepare(&mut command, program);
    run_until_done(&mut command, "claude -p /usage").map(drop)
}

/// `agy -p /usage` は会話を作らず、モデルへのリクエストも送らずに、利用制限の残りを返す。
/// agy は起動のたびに自分のログのフォルダへ新しいファイルを作るので、上書きされる一つのファイルへ向ける。
fn read_antigravity(program: &Path, home: &MarimoHome) -> Result<Value, String> {
    let _ = std::fs::create_dir_all(home.logs_dir());
    let mut command = Command::new(program);
    command
        .args(["-p", "/usage", "--output-format", "json", "--log-file"])
        .arg(home.logs_dir().join("agy-usage.log"))
        .current_dir(home.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    prepare(&mut command, program);
    let stdout = run_until_done(&mut command, "agy -p /usage")?;
    serde_json::from_slice(&stdout).map_err(|e| format!("agy -p /usage did not print JSON: {e}"))
}

/// 終わるまで待ち、PRINT_TIMEOUT を過ぎたら止める。stdout を受けるときは、出力でパイプが詰まって
/// 終われなくならないよう、別のスレッドで読む。
fn run_until_done(command: &mut Command, what: &str) -> Result<Vec<u8>, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {what}: {e}"))?;
    let reader = child.stdout.take().map(|mut stdout| {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            let _ = tx.send(bytes);
        });
        rx
    });
    let deadline = Instant::now() + PRINT_TIMEOUT;
    loop {
        match child.try_wait() {
            // 子プロセスが起動したプロセスがパイプを開いたままでも待ち続けないよう、読み終わりを待つ時間に上限を付ける。
            Ok(Some(status)) if status.success() => {
                let bytes = reader.and_then(|rx| rx.recv_timeout(EXIT_GRACE).ok());
                return Ok(bytes.unwrap_or_default());
            }
            Ok(Some(status)) => return Err(format!("{what} exited with {status}")),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(200)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{what} did not finish"));
            }
            Err(e) => return Err(format!("cannot wait for {what}: {e}")),
        }
    }
}

fn read_codex(program: &Path) -> Result<Value, String> {
    let mut command = Command::new(program);
    command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    prepare(&mut command, program);
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
    let result = ask_rate_limits(&mut child);
    // app-server は stdin が閉じると自分で終わる。Windows の codex.cmd は cmd.exe から node と codex を
    // 起動するので、kill で止めると cmd.exe だけが終わって子が残る。先に終わるのを待ち、それでも
    // 残ったときだけ kill する。
    let deadline = Instant::now() + EXIT_GRACE;
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

/// stdin は関数を抜けるときに閉じる。
fn ask_rate_limits(child: &mut Child) -> Result<Value, String> {
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let messages = [
        json!({"id": 1, "method": "initialize", "params": {
            "clientInfo": {"name": "marimo", "version": env!("CARGO_PKG_VERSION")}}}),
        json!({"method": "initialized"}),
        json!({"id": 2, "method": "account/rateLimits/read"}),
    ];
    for message in messages {
        writeln!(stdin, "{message}")
            .map_err(|e| format!("cannot write to codex app-server: {e}"))?;
    }
    let deadline = Instant::now() + REPLY_TIMEOUT;
    loop {
        let line = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "codex app-server did not answer".to_owned())?;
        let Ok(reply) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = reply.get("id").and_then(Value::as_i64);
        if let (Some(1 | 2), Some(error)) = (id, reply.get("error")) {
            return Err(format!("codex app-server returned an error: {error}"));
        }
        if id == Some(2) {
            return reply
                .get("result")
                .cloned()
                .ok_or_else(|| "codex app-server sent no result".to_owned());
        }
    }
}

fn program_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.to_owned()]
    }
}

/// Finder や LaunchAgent から起動したアプリの PATH は /usr/bin などだけなので、PATH の次に、Claude Code の
/// ネイティブ版が入る `~/.local/bin`、Homebrew の置き場所、ChatGPT のデスクトップアプリが同梱する codex の
/// 置き場所も探す。
fn find_program(names: &[String], user_home: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.push(user_home.join(".local").join("bin"));
    if cfg!(target_os = "macos") {
        dirs.extend(
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin",
            ]
            .map(PathBuf::from),
        );
    }
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|p| p.is_file())
}

/// npm が入れる codex は `#!/usr/bin/env node` で始まる。GUI のアプリは端末の PATH を引き継がないので、
/// 隣にある node が見つかるよう、プログラムのフォルダを PATH の先頭に足す。
fn prepare(command: &mut Command, program: &Path) {
    let rest = env::var_os("PATH").map(|p| env::split_paths(&p).collect::<Vec<_>>());
    let dirs = program.parent().map(Path::to_path_buf).into_iter();
    if let Ok(path) = env::join_paths(dirs.chain(rest.into_iter().flatten())) {
        command.env("PATH", path);
    }
    hide_console(command);
}
