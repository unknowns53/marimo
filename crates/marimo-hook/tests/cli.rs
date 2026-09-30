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
        // 利用者の本物の Codex と Hermes のフォルダを読まないよう、存在しない場所を指しておく。
        .env("CODEX_HOME", home.join("no-codex-home"))
        .env("HERMES_HOME", home.join("no-hermes-home"))
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

    // displayContent を返さなければ元の文章がそのまま表示される（hooks のドキュメントの
    // MessageDisplay output の節）ので、marimo は何も出力してはいけない。
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

    let end = json!({"session_id":"s1","hook_event_name":"SessionEnd","reason":"other"});
    let out = run(home, &["hook"], end.to_string().as_bytes());
    assert!(out.status.success() && out.stdout.is_empty());
    assert!(session(home, "s1").is_none());
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
fn codex_hook_tracks_state_context_title_and_rate_limits() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let codex_home = dir.path().join("codex");
    std::fs::create_dir_all(&codex_home).unwrap();
    let index = [
        json!({"id": "t1", "thread_name": "古い名前", "updated_at": "2026-09-27T00:00:00Z"}),
        json!({"id": "t1", "thread_name": "テストの会話", "updated_at": "2026-09-27T00:01:00Z"}),
    ]
    .map(|v| format!("{v}\n"))
    .concat();
    std::fs::write(codex_home.join("session_index.jsonl"), index).unwrap();
    let rollout = dir.path().join("rollout.jsonl");
    let token_count = json!({
        "timestamp": "2026-09-27T00:02:00.000Z", "type": "event_msg",
        "payload": {"type": "token_count",
            "info": {"last_token_usage": {"total_tokens": 142_000}, "total_token_usage": {"total_tokens": 900_000},
                     "model_context_window": 272_000},
            "rate_limits": {"primary": {"used_percent": 12.5, "window_minutes": 10080, "resets_at": 1_800_000_000},
                            "secondary": null, "plan_type": "plus"}}
    });
    std::fs::write(&rollout, format!("{token_count}\n")).unwrap();
    let envs = [("CODEX_HOME", codex_home.to_str().unwrap())];
    let send = |v: Value| {
        let out = run_with_env(&home, &["codex-hook"], v.to_string().as_bytes(), &envs);
        assert!(out.status.success() && out.stdout.is_empty(), "{v}");
    };
    let common =
        json!({"session_id": "t1", "cwd": "/w/p", "transcript_path": rollout, "model": "gpt-5.5"});
    let event = |name: &str, extra: Value| {
        let mut v = common.clone();
        v["hook_event_name"] = json!(name);
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    let codex_session = || session(&home, "codex-t1").unwrap();

    send(event(
        "SessionStart",
        json!({"source": "startup", "permission_mode": "default"}),
    ));
    let s = codex_session();
    assert_eq!(
        (&s["provider"], &s["status"]),
        (&json!("codex"), &json!("idle"))
    );
    assert_eq!(s["title"], "テストの会話");
    assert_eq!(s["context"]["used_percentage"], 50.0);
    assert_eq!(s["context"]["source"], "codex-rollout");
    assert!(session(&home, "t1").is_none());
    let limits: Value =
        serde_json::from_slice(&std::fs::read(home.join("codex_rate_limits.json")).unwrap())
            .unwrap();
    assert_eq!(
        limits["windows"],
        json!([{"window_minutes": 10080, "used_percentage": 12.5, "resets_at": 1_800_000_000}])
    );
    assert_eq!(limits["plan_type"], "plus");

    send(event(
        "UserPromptSubmit",
        json!({"turn_id": "u1", "prompt": "hi"}),
    ));
    send(event(
        "PreToolUse",
        json!({"turn_id": "u1", "tool_name": "apply_patch", "tool_use_id": "c1",
        "tool_input": {"command": "*** Begin Patch\n*** Update File: src/main.rs\n*** End Patch\n"}}),
    ));
    let s = codex_session();
    assert_eq!(
        (&s["status"], &s["activity"]["summary"]),
        (&json!("working"), &json!("編集: src/main.rs"))
    );
    send(event(
        "PermissionRequest",
        json!({"turn_id": "u1", "tool_name": "Bash",
        "tool_input": {"command": "rm -rf target"}}),
    ));
    assert_eq!(codex_session()["status"], "waiting");
    send(event("Interrupt", json!({"turn_id": "u1"})));
    assert_eq!(codex_session()["status"], "idle");
    send(
        json!({"session_id": "t1", "hook_event_name": "SessionEnd", "cwd": "/w/p",
        "transcript_path": null, "reason": "exit"}),
    );
    assert!(session(&home, "codex-t1").is_none());
}

#[test]
fn hermes_hook_tracks_a_discord_turn() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("marimo");
    let hermes = dir.path().join("hermes");
    std::fs::create_dir_all(hermes.join("sessions")).unwrap();
    std::fs::write(
        hermes.join("sessions").join("sessions.json"),
        json!({
            "agent:main:discord:group:222222222222222222:333333333333333333": {
                "session_id": "20260101_000000_abcd1234",
                "platform": "discord",
                "chat_type": "group",
                "origin": {
                    "platform": "discord", "chat_type": "group",
                    "chat_id": "222222222222222222", "chat_name": "Server / #general",
                    "guild_id": "444444444444444444"
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    let hermes_env = hermes.to_string_lossy().into_owned();
    let send = |event: Value| {
        let mut payload = json!({
            "session_id": "20260101_000000_abcd1234",
            "cwd": hermes_env,
            "tool_name": null,
            "tool_input": null,
            "profile": "default",
        });
        payload
            .as_object_mut()
            .unwrap()
            .extend(event.as_object().unwrap().clone());
        let out = run_with_env(
            &home,
            &["hermes-hook"],
            payload.to_string().as_bytes(),
            &[("HERMES_HOME", &hermes_env)],
        );
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stdout.is_empty());
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let state = || session(&home, "hermes-20260101_000000_abcd1234");

    send(json!({
        "hook_event_name": "pre_llm_call",
        "extra": { "turn_id": "t1", "platform": "discord", "user_message": "hi" }
    }));
    let s = state().unwrap();
    assert_eq!(s["provider"], "hermes");
    assert_eq!(s["status"], "working");
    assert_eq!(s["title"], "#general");
    assert_eq!(
        s["link"],
        "discord://-/channels/444444444444444444/222222222222222222"
    );
    assert!(s.get("cwd").is_none_or(Value::is_null), "{s}");
    assert!(s.get("origin").is_none_or(Value::is_null), "{s}");

    send(json!({
        "hook_event_name": "post_api_request",
        "extra": { "turn_id": "t1", "usage": { "prompt_tokens": 27200 }, "context_length": 272000 }
    }));
    assert_eq!(state().unwrap()["context"]["used_percentage"], 10.0);

    send(json!({
        "hook_event_name": "post_llm_call",
        "extra": { "turn_id": "t1", "assistant_response": "done" }
    }));
    send(json!({
        "hook_event_name": "pre_tool_call",
        "tool_name": "memory",
        "tool_input": {},
        "extra": { "turn_id": "background-review" }
    }));
    assert_eq!(state().unwrap()["status"], "done");

    send(json!({ "hook_event_name": "on_session_finalize", "extra": {} }));
    assert!(state().is_none());
    // Hermes のファイルは接頭辞で Claude Code のファイルと分けるので、接頭辞の無い名前では作らない。
    assert!(session(&home, "20260101_000000_abcd1234").is_none());
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
            &["codex-hook"][..],
            &["hermes-hook"][..],
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

    // 入力が壊れていても受け渡しは行う。
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

#[test]
fn statusline_exit_code_follows_the_original() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        let out = run(
            dir.path(),
            &["statusline", "--", "sh", "-c", "cat >/dev/null; exit 3"],
            b"{}",
        );
        assert_eq!(out.status.code(), Some(3));
    }
    let out = run(
        dir.path(),
        &["statusline", "--", "marimo-definitely-not-a-command"],
        STATUSLINE_INPUT.as_bytes(),
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}
