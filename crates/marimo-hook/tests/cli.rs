use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn run(home: &Path, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_marimo-hook"))
        .args(args)
        .env("MARIMO_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

fn session(home: &Path, id: &str) -> Option<Value> {
    let bytes = std::fs::read(home.join("sessions").join(format!("{id}.json"))).ok()?;
    Some(serde_json::from_slice(&bytes).unwrap())
}

const STATUSLINE_INPUT: &str = r#"{"session_id":"s1","cwd":"/w/p","model":{"id":"claude-opus-5-5","display_name":"Opus"},"context_window":{"total_input_tokens":15500,"context_window_size":200000,"used_percentage":8},"rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1738425600}}}"#;

#[test]
fn hook_updates_state_and_prints_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let events = [
        json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","cwd":"/w/p","prompt":"hello"}),
        json!({"session_id":"s1","hook_event_name":"PreToolUse","cwd":"/w/p","tool_name":"Bash","tool_input":{"command":"cargo build"},"tool_use_id":"t1"}),
    ];
    for event in &events {
        let out = run(home, &["hook"], event.to_string().as_bytes());
        assert!(out.status.success());
        assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    }
    let s = session(home, "s1").unwrap();
    assert_eq!(s["status"], "working");
    assert_eq!(s["line"], "Bash: cargo build");

    let end = json!({"session_id":"s1","hook_event_name":"SessionEnd","reason":"other"});
    let out = run(home, &["hook"], end.to_string().as_bytes());
    assert!(out.status.success() && out.stdout.is_empty());
    assert!(session(home, "s1").is_none());
}

#[test]
fn broken_input_exits_zero_with_empty_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let inputs: [&[u8]; 5] = [
        b"",
        b"{not json",
        b"[1,2,3]",
        br#"{"hook_event_name":"Stop"}"#,
        br#"{"session_id":"../../etc","hook_event_name":"Stop"}"#,
    ];
    for input in inputs {
        for args in [
            &["hook"][..],
            &["statusline"][..],
            &["record", "x"][..],
            &["nope"][..],
            &[][..],
        ] {
            let out = run(dir.path(), args, input);
            assert_eq!(out.status.code(), Some(0), "args {args:?} input {input:?}");
            assert!(out.stdout.is_empty(), "args {args:?} input {input:?}");
        }
    }
    assert!(!dir.path().join("sessions").exists());
}

#[test]
fn unwritable_home_still_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    // MARIMO_HOME がディレクトリでなくファイルを指していると、書き込みは必ず失敗する。
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let event = json!({"session_id":"s1","hook_event_name":"UserPromptSubmit"});
    let out = run(&file, &["hook"], event.to_string().as_bytes());
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn statusline_saves_rate_limits() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &["statusline"], STATUSLINE_INPUT.as_bytes());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let rl: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("rate_limits.json")).unwrap())
            .unwrap();
    assert_eq!(rl["five_hour"]["used_percentage"], 23.5);
}

#[cfg(unix)]
#[test]
fn statusline_relays_stdin_and_stdout_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    // 元のコマンドが受け取った stdin をそのまま出力すれば、行きと帰りの両方を一度に確かめられる。
    let out = run(
        dir.path(),
        &["statusline", "--", "cat"],
        STATUSLINE_INPUT.as_bytes(),
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, STATUSLINE_INPUT.as_bytes());
    assert!(dir.path().join("rate_limits.json").exists());

    let script = r#"printf '\033[32mOpus\033[0m | '; wc -c | tr -d ' '; printf 'line2\n'"#;
    let out = run(
        dir.path(),
        &["statusline", "--", "sh", "-c", script],
        STATUSLINE_INPUT.as_bytes(),
    );
    let expected = format!("\x1b[32mOpus\x1b[0m | {}\nline2\n", STATUSLINE_INPUT.len());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
}

#[cfg(unix)]
#[test]
fn statusline_relays_even_when_input_is_broken() {
    let dir = tempfile::tempdir().unwrap();
    let broken = b"{\"session_id\": oops \xff";
    let out = run(dir.path(), &["statusline", "--", "cat"], broken);
    assert!(out.status.success());
    assert_eq!(out.stdout, broken);

    // MARIMO_HOME に書けないときも受け渡しは行う。
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let out = run(
        &file,
        &["statusline", "--", "cat"],
        STATUSLINE_INPUT.as_bytes(),
    );
    assert_eq!(out.stdout, STATUSLINE_INPUT.as_bytes());
}

#[cfg(unix)]
#[test]
fn statusline_propagates_original_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        dir.path(),
        &["statusline", "--", "sh", "-c", "cat >/dev/null; exit 3"],
        b"{}",
    );
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn statusline_with_missing_command_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        dir.path(),
        &["statusline", "--", "marimo-definitely-not-a-command"],
        STATUSLINE_INPUT.as_bytes(),
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}

#[test]
fn record_appends_without_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        dir.path(),
        &["record", "Notification"],
        br#"{"notification_type":"permission_prompt"}"#,
    );
    assert!(out.status.success() && out.stdout.is_empty());
    let out = run(dir.path(), &["record", "statusline"], b"raw text");
    assert!(out.status.success() && out.stdout.is_empty());
    let text = std::fs::read_to_string(dir.path().join("logs").join("record.jsonl")).unwrap();
    let lines: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["label"], "Notification");
    assert_eq!(
        lines[0]["payload"]["notification_type"],
        "permission_prompt"
    );
    assert_eq!(lines[1]["payload"]["raw"], "raw text");
}
