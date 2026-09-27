use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn run(home: &Path, args: &[&str], stdin: &[u8]) -> Output {
    run_with_env(home, args, stdin, &[])
}

fn run_with_env(home: &Path, args: &[&str], stdin: &[u8], envs: &[(&str, &str)]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_marimo-hook"))
        .args(args)
        .env("MARIMO_HOME", home)
        .env_remove("TERM_PROGRAM")
        .env_remove("__CFBundleIdentifier")
        .envs(envs.iter().copied())
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
    assert_eq!(s["activity"]["summary"], "cargo build");
    assert_eq!(s["activity"]["detail"], "cargo build");

    let end = json!({"session_id":"s1","hook_event_name":"SessionEnd","reason":"other"});
    let out = run(home, &["hook"], end.to_string().as_bytes());
    assert!(out.status.success() && out.stdout.is_empty());
    assert!(session(home, "s1").is_none());
}

// displayContent を返さなければ元の文章がそのまま表示される（hooks のドキュメントの
// MessageDisplay output の節）ので、marimo は何も出力してはいけない。
#[test]
fn message_display_prints_nothing_and_updates_line() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let start = json!({"session_id":"s1","hook_event_name":"UserPromptSubmit","cwd":"/w/p"});
    run(home, &["hook"], start.to_string().as_bytes());
    let md = json!({
        "session_id": "s1", "hook_event_name": "MessageDisplay", "cwd": "/w/p",
        "turn_id": "t", "message_id": "m", "index": 0, "final": false,
        "delta": "Here is the plan:\n1. Read the docs\n"
    });
    let out = run(home, &["hook"], md.to_string().as_bytes());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let s = session(home, "s1").unwrap();
    assert_eq!(s["status"], "working");
    assert_eq!(s["activity"]["summary"], "1. Read the docs");
}

#[test]
fn hook_records_origin_from_environment() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let event =
        json!({"session_id":"s1","hook_event_name":"SessionStart","source":"startup","cwd":"/w/p"});
    let terminal = [
        ("TERM_PROGRAM", "Apple_Terminal"),
        ("__CFBundleIdentifier", "com.apple.Terminal"),
    ];
    run_with_env(home, &["hook"], event.to_string().as_bytes(), &terminal);
    let s = session(home, "s1").unwrap();
    assert_eq!(s["origin"]["term_program"], "Apple_Terminal");
    assert_eq!(s["origin"]["bundle_id"], "com.apple.Terminal");

    // --resume で別のアプリへ移ったら、起動元を取り直す。
    let desktop = [("__CFBundleIdentifier", "com.anthropic.claudefordesktop")];
    run_with_env(home, &["hook"], event.to_string().as_bytes(), &desktop);
    let s = session(home, "s1").unwrap();
    assert_eq!(s["origin"]["bundle_id"], "com.anthropic.claudefordesktop");
    assert!(s["origin"].get("term_program").is_none());
}

#[test]
fn post_tool_use_reads_context_tokens_from_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let transcript = dir.path().join("t.jsonl");
    let line = json!({
        "type": "assistant", "isSidechain": false, "entrypoint": "cli",
        "message": {"role": "assistant", "content": [], "usage": {
            "input_tokens": 2, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 27038, "output_tokens": 4
        }}
    });
    std::fs::write(&transcript, format!("{line}\n")).unwrap();
    let event = json!({
        "session_id": "s1", "hook_event_name": "PostToolUse", "cwd": "/w/p",
        "transcript_path": transcript, "tool_name": "Bash", "tool_input": {"command": "ls"}
    });
    let out = run(&home, &["hook"], event.to_string().as_bytes());
    assert!(out.status.success() && out.stdout.is_empty());
    let s = session(&home, "s1").unwrap();
    assert_eq!(s["context"]["total_input_tokens"], 27140);
    assert_eq!(s["context"]["source"], "transcript");
    assert!(s["context"]["used_percentage"].is_null());
    assert_eq!(s["origin"]["entrypoint"], "cli");

    // 会話ログが読めなくても、状態の更新は続ける。
    let missing = json!({
        "session_id": "s2", "hook_event_name": "Stop", "transcript_path": "/nonexistent.jsonl",
        "last_assistant_message": "done"
    });
    let out = run(&home, &["hook"], missing.to_string().as_bytes());
    assert!(out.status.success() && out.stdout.is_empty());
    assert_eq!(session(&home, "s2").unwrap()["status"], "done");
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
    // 記録は UTC の日付ごとのファイルに分かれるので、日付をまたいだ場合も含めてすべて読む。
    let mut files: Vec<_> = std::fs::read_dir(dir.path().join("logs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(files.iter().all(|f| {
        let name = f.file_name().unwrap().to_string_lossy();
        name.starts_with("record-") && name.ends_with(".jsonl")
    }));
    let text: String = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
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
