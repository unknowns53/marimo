use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const EVENTS: [&str; 14] = [
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

struct Env {
    _dir: tempfile::TempDir,
    home: PathBuf,
    settings: PathBuf,
}

impl Env {
    fn new(marimo_dir_name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(marimo_dir_name);
        let claude = dir.path().join("claude");
        fs::create_dir_all(&claude).unwrap();
        Self {
            home,
            settings: claude.join("settings.json"),
            _dir: dir,
        }
    }

    // 実行ファイルのパスに空白が入る場合の引用も、同時に確かめる。
    fn with_settings(text: &str) -> Self {
        let env = Self::new("marimo home");
        fs::write(&env.settings, text).unwrap();
        env
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_marimo-hook"))
            .args(args)
            .arg("--settings")
            .arg(&self.settings)
            .env("MARIMO_HOME", &self.home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn installed_exe(&self) -> PathBuf {
        self.home
            .join("bin")
            .join(format!("marimo-hook{}", std::env::consts::EXE_SUFFIX))
    }

    fn text(&self) -> String {
        fs::read_to_string(&self.settings).unwrap()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.text()).unwrap()
    }

    fn backups(&self) -> Vec<PathBuf> {
        let mut v: Vec<_> = fs::read_dir(self.settings.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("settings.json.marimo-backup-")
            })
            .collect();
        v.sort();
        v
    }
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

// 利用者の実際の設定と同じ形の設定。既存のフックが PreToolUse に 2 グループ、
// SessionStart に 1 グループあり、statusLine は bash の一行で、ほかの項目もいくつか持つ。
fn realistic() -> String {
    r#"{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "permissions": {
    "allow": [
      "Bash(git status:*)",
      "WebFetch(domain:code.claude.com)"
    ],
    "deny": [],
    "defaultMode": "auto"
  },
  "model": "opus",
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup|resume",
        "hooks": [
          {
            "type": "command",
            "command": "other-tool hook session-start"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "~/.claude/hooks/guard.sh"
          }
        ]
      },
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "echo 'edit' >> /tmp/edits.log",
            "timeout": 10
          }
        ]
      }
    ]
  },
  "statusLine": {
    "type": "command",
    "command": "bash ~/.claude/statusline.sh",
    "padding": 0
  },
  "skillOverrides": {
    "example-marketplace": {
      "enabled": true
    }
  },
  "enabledPlugins": {
    "example-plugin@example-marketplace": true
  },
  "alwaysThinkingEnabled": true
}
"#
    .to_owned()
}

#[test]
fn install_then_uninstall_restores_realistic_settings() {
    let original = realistic();
    let env = Env::with_settings(&original);

    let out = env.run(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let installed = env.json();
    let exe = env.installed_exe();
    assert!(exe.is_file());
    let hook_cmd = format!("'{}' hook", exe.display());
    for event in EVENTS {
        let groups = installed["hooks"][event].as_array().unwrap();
        let marimo: Vec<_> = groups
            .iter()
            .filter(|g| g["hooks"][0]["command"] == hook_cmd.as_str())
            .collect();
        assert_eq!(marimo.len(), 1, "{event}");
        assert_eq!(marimo[0]["hooks"][0]["timeout"], 5);
        assert!(marimo[0].get("matcher").is_none());
    }
    // 既存のグループは前に残り、marimo のグループは後ろに足される。
    assert_eq!(
        installed["hooks"]["PreToolUse"].as_array().unwrap().len(),
        3
    );
    assert_eq!(installed["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(
        installed["statusLine"]["command"],
        format!(
            "'{}' statusline -- sh -c 'bash ~/.claude/statusline.sh'",
            exe.display()
        )
    );
    assert_eq!(installed["statusLine"]["padding"], 0);
    let keys: Vec<_> = installed.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        [
            "$schema",
            "permissions",
            "model",
            "hooks",
            "statusLine",
            "skillOverrides",
            "enabledPlugins",
            "alwaysThinkingEnabled"
        ]
    );
    let backups = env.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), original);
    let text = stdout(&out);
    assert!(text.contains("バックアップ"), "{text}");
    assert!(
        text.contains("変更前: bash ~/.claude/statusline.sh"),
        "{text}"
    );

    let out = env.run(&["uninstall"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(env.text(), original);
    assert_eq!(
        env.json(),
        serde_json::from_str::<Value>(&original).unwrap()
    );
    assert!(stdout(&out).contains("元のコマンドへ戻します"));
    assert!(exe.is_file(), "uninstall must not delete the executable");
}

#[test]
fn install_is_idempotent() {
    let env = Env::with_settings(&realistic());
    assert!(env.run(&["install"]).status.success());
    let once = env.text();
    let out = env.run(&["install"]);
    assert!(out.status.success());
    assert_eq!(env.text(), once);
    assert_eq!(env.backups().len(), 1, "no backup when nothing changes");
    assert!(stdout(&out).contains("変更はありません"));
}

#[test]
fn minimal_settings_round_trip() {
    let cases = [
        json!({}),
        json!({"model": "sonnet"}),
        json!({"model": "sonnet", "statusLine": {"type": "command", "command": "echo hi"}}),
        json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}),
    ];
    for original in cases {
        let text = pretty(&original);
        let env = Env::with_settings(&text);
        let out = env.run(&["install"]);
        assert!(
            out.status.success(),
            "{original}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let installed = env.json();
        for event in EVENTS {
            assert!(installed["hooks"][event].is_array(), "{original} {event}");
        }
        let status = installed["statusLine"]["command"].as_str().unwrap();
        if original.get("statusLine").is_none() {
            assert!(status.ends_with(" statusline"), "{status}");
        } else {
            assert!(
                status.ends_with(" statusline -- sh -c 'echo hi'"),
                "{status}"
            );
        }
        let out = env.run(&["uninstall"]);
        assert!(out.status.success());
        assert_eq!(env.text(), text, "round trip of {original}");
    }
}

// install の前から空だった hooks は、marimo の分を取り除いて空になったものと区別できない。
// 空の hooks は無いのと同じ意味なので、消えることを仕様として確かめておく。
#[test]
fn empty_hooks_object_does_not_survive_round_trip() {
    let env = Env::with_settings(&pretty(&json!({"hooks": {}, "model": "x"})));
    assert!(env.run(&["install"]).status.success());
    assert!(env.run(&["uninstall"]).status.success());
    assert_eq!(env.json(), json!({"model": "x"}));
}

#[test]
fn missing_settings_file_is_created_without_backup() {
    let env = Env::new("m");
    let out = env.run(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(env.json()["hooks"]["Stop"].is_array());
    assert!(env.backups().is_empty());
}

#[test]
fn invalid_settings_are_left_untouched() {
    let cases: [&[u8]; 5] = [
        b"{\"model\": \"opus\",",
        b"[1, 2]",
        b"\"text\"",
        br#"{"hooks": []}"#,
        br#"{"hooks": {"Stop": {"hooks": []}}}"#,
    ];
    for content in cases {
        for sub in ["install", "uninstall"] {
            let env = Env::new("m");
            fs::write(&env.settings, content).unwrap();
            let out = env.run(&[sub]);
            assert!(
                !out.status.success(),
                "{sub} {:?}",
                String::from_utf8_lossy(content)
            );
            assert!(!out.stderr.is_empty());
            assert_eq!(fs::read(&env.settings).unwrap(), content);
            assert!(env.backups().is_empty());
            assert!(
                !env.installed_exe().exists(),
                "nothing is copied on failure"
            );
        }
    }
}

#[test]
fn unreadable_settings_fail() {
    let env = Env::new("m");
    // ディレクトリはファイルとして読めないので、読み込みの失敗を確実に起こせる。
    fs::create_dir_all(&env.settings).unwrap();
    for sub in ["install", "uninstall"] {
        let out = env.run(&[sub]);
        assert!(!out.status.success(), "{sub}");
        assert!(!out.stderr.is_empty());
        assert!(env.settings.is_dir());
    }
}

#[test]
fn dry_run_changes_nothing() {
    let original = realistic();
    let env = Env::with_settings(&original);
    let out = env.run(&["install", "--dry-run"]);
    assert!(out.status.success());
    assert_eq!(env.text(), original);
    assert!(env.backups().is_empty());
    assert!(!env.installed_exe().exists());
    let text = stdout(&out);
    assert!(text.contains("dry-run"));
    assert!(text.contains("\"MessageDisplay\""));

    assert!(env.run(&["install"]).status.success());
    let installed = env.text();
    let out = env.run(&["uninstall", "--dry-run"]);
    assert!(out.status.success());
    assert_eq!(env.text(), installed);
    assert!(stdout(&out).contains("bash ~/.claude/statusline.sh"));
}

#[test]
fn unknown_arguments_fail() {
    let env = Env::with_settings("{}");
    let out = env.run(&["install", "--force"]);
    assert!(!out.status.success());
    assert_eq!(env.text(), "{}");
}

#[test]
fn claude_config_dir_sets_the_default_settings_path() {
    let env = Env::with_settings("{}");
    let out = Command::new(env!("CARGO_BIN_EXE_marimo-hook"))
        .args(["install", "--dry-run"])
        .env("MARIMO_HOME", &env.home)
        .env("CLAUDE_CONFIG_DIR", env.settings.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout(&out).contains(&env.settings.display().to_string()));
}

#[test]
fn uninstall_keeps_user_hooks_sharing_an_event() {
    let original = pretty(&json!({
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}
    }));
    let env = Env::with_settings(&original);
    assert!(env.run(&["install"]).status.success());
    assert_eq!(env.json()["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert!(env.run(&["uninstall"]).status.success());
    assert_eq!(env.text(), original);
}

// 実行ファイルと名前の先頭だけが同じ他人のコマンドを、marimo のものと取り違えない。
#[test]
fn commands_sharing_the_executable_prefix_are_left_alone() {
    let env = Env::new("m");
    let bin = env.home.join("bin");
    let foreign_hook = format!("{}/marimo-hook-backup hook", bin.display());
    let foreign_status = format!("{}/marimo-hooky statusline", bin.display());
    let original = pretty(&json!({
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": foreign_hook}]}]},
        "statusLine": {"type": "command", "command": foreign_status}
    }));
    fs::write(&env.settings, &original).unwrap();

    let out = env.run(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let installed = env.json();
    assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(
        installed["hooks"]["Stop"][0]["hooks"][0]["command"],
        foreign_hook
    );
    assert_eq!(
        installed["statusLine"]["command"],
        format!(
            "{} statusline -- sh -c '{foreign_status}'",
            env.installed_exe().display()
        )
    );

    let once = env.text();
    let out = env.run(&["install"]);
    assert!(out.status.success());
    assert_eq!(env.text(), once);
    assert!(stdout(&out).contains("変更はありません"));

    assert!(env.run(&["uninstall"]).status.success());
    assert_eq!(env.text(), original);
    let out = env.run(&["uninstall"]);
    assert!(out.status.success());
    assert_eq!(env.text(), original);
}

#[cfg(unix)]
fn sh(command: &str, stdin: &str, home: &std::path::Path) -> String {
    use std::io::Write;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .env("HOME", home)
        .env("MARIMO_HOME", home.join("marimo-data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
}

#[cfg(unix)]
#[test]
fn wrapped_status_line_behaves_like_the_original() {
    let originals = [
        // 引用符、パイプ、~ の展開、$() を一通り含める。
        r#"input=$(cat); printf '%s|' "$(printf '%s' "$input" | wc -c | tr -d ' ')"; echo ~/x | sed "s|$HOME|HOME|"; echo 'it'"'"'s'"#,
        r#"jq -r '"[\(.model.display_name)] \(.context_window.used_percentage // 0)% context"' 2>/dev/null || cat | tr 'a-z' 'A-Z'"#,
        "printf \"a'b\\n\"; cat >/dev/null",
    ];
    let input = r#"{"session_id":"s1","model":{"display_name":"Opus"},"context_window":{"used_percentage":8}}"#;
    for original in originals {
        let env = Env::with_settings(&pretty(
            &json!({"statusLine": {"type": "command", "command": original}}),
        ));
        assert!(env.run(&["install"]).status.success());
        let wrapped = env.json()["statusLine"]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(wrapped, original);
        let fake_home = env.home.parent().unwrap();
        let expected = sh(original, input, fake_home);
        assert!(!expected.is_empty());
        assert_eq!(
            sh(&wrapped, input, fake_home),
            expected,
            "wrapped: {wrapped}"
        );
        assert!(env.run(&["uninstall"]).status.success());
        assert_eq!(env.json()["statusLine"]["command"], original);
    }
}
