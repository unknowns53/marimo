use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const EVENTS: [&str; 16] = [
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

struct Env {
    dir: tempfile::TempDir,
    home: PathBuf,
    settings: PathBuf,
    /// CODEX_HOME として渡す場所。with_codex で作るまでは存在しないので、Codex は入っていない扱いになる。
    codex: PathBuf,
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
            codex: dir.path().join("codex"),
            dir,
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
            // Windows の statusLine の書き方はホームフォルダの下かどうかで変わるので、
            // 一時フォルダをホームとして渡して結果を決まったものにする。
            .env("HOME", self.dir.path())
            .env("USERPROFILE", self.dir.path())
            .env("CODEX_HOME", &self.codex)
            .env_remove("CLAUDE_CONFIG_DIR")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn with_codex(hooks_json: Option<&str>) -> Self {
        let env = Self::with_settings("{}");
        fs::create_dir_all(&env.codex).unwrap();
        if let Some(text) = hooks_json {
            fs::write(env.codex_hooks(), text).unwrap();
        }
        env
    }

    fn codex_hooks(&self) -> PathBuf {
        self.codex.join("hooks.json")
    }

    fn installed_exe(&self) -> PathBuf {
        self.home
            .join("bin")
            .join(format!("marimo-hook{}", std::env::consts::EXE_SUFFIX))
    }

    // 以前の install が書いていた shell form のコマンド。Windows では区切りを / にしていた。
    fn legacy_hook_command(&self) -> String {
        let path = self.installed_exe().display().to_string();
        let path = if cfg!(windows) {
            path.replace('\\', "/")
        } else {
            path
        };
        assert!(
            path.contains(' '),
            "the legacy form is quoted only with a space"
        );
        format!("'{path}' hook")
    }

    fn exec_handler(&self) -> Value {
        json!({
            "type": "command",
            "command": self.installed_exe().display().to_string(),
            "args": ["hook"],
            "timeout": 5
        })
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
    for event in EVENTS {
        let groups = installed["hooks"][event].as_array().unwrap();
        let marimo: Vec<_> = groups
            .iter()
            .filter(|g| g["hooks"][0]["command"] == exe.display().to_string().as_str())
            .collect();
        assert_eq!(marimo.len(), 1, "{event}");
        // 実行ファイルのパスに空白があっても、exec form なので引用しない。
        assert_eq!(
            marimo[0],
            &json!({"hooks": [env.exec_handler()]}),
            "{event}"
        );
    }
    // 既存のグループは前に残り、marimo のグループは後ろに足される。
    assert_eq!(
        installed["hooks"]["PreToolUse"].as_array().unwrap().len(),
        3
    );
    assert_eq!(installed["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    #[cfg(unix)]
    assert_eq!(
        installed["statusLine"]["command"],
        format!(
            "'{}' statusline -- sh -c 'bash ~/.claude/statusline.sh'",
            exe.display()
        )
    );
    #[cfg(windows)]
    assert_eq!(
        installed["statusLine"]["command"],
        "bash ~/.claude/statusline.sh"
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
    #[cfg(unix)]
    assert!(
        text.contains("変更前: bash ~/.claude/statusline.sh"),
        "{text}"
    );
    #[cfg(windows)]
    assert!(text.contains("「利用制限を API から取得」"), "{text}");

    let once = env.text();
    let out = env.run(&["install"]);
    assert!(out.status.success());
    assert_eq!(env.text(), once);
    assert_eq!(env.backups().len(), 1, "no backup when nothing changes");
    assert!(stdout(&out).contains("変更はありません"));

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
    #[cfg(unix)]
    assert!(stdout(&out).contains("元のコマンドへ戻します"));
    assert!(exe.is_file(), "uninstall must not delete the executable");
}

#[test]
fn minimal_settings_round_trip() {
    // install の前から空だった hooks は、marimo の分を取り除いて空になったものと区別できない。
    // 空の hooks は無いのと同じ意味なので、消えることを仕様として確かめておく。
    let cases = [
        (json!({}), None),
        (json!({"model": "sonnet"}), None),
        (
            json!({"model": "sonnet", "statusLine": {"type": "command", "command": "echo hi"}}),
            None,
        ),
        (
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}),
            None,
        ),
        (
            json!({"hooks": {}, "model": "x"}),
            Some(json!({"model": "x"})),
        ),
    ];
    for (original, after_uninstall) in cases {
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
        #[cfg(unix)]
        {
            let status = installed["statusLine"]["command"].as_str().unwrap();
            if original.get("statusLine").is_none() {
                assert!(status.ends_with(" statusline"), "{status}");
            } else {
                assert!(
                    status.ends_with(" statusline -- sh -c 'echo hi'"),
                    "{status}"
                );
            }
        }
        // 実行ファイルのパスに空白があるので、既存の statusLine は包まず、新しく登録もしない。
        #[cfg(windows)]
        assert_eq!(installed.get("statusLine"), original.get("statusLine"));
        let out = env.run(&["uninstall"]);
        assert!(out.status.success());
        let expected = after_uninstall
            .as_ref()
            .map_or_else(|| text.clone(), pretty);
        assert_eq!(env.text(), expected, "round trip of {original}");
    }
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

    // ディレクトリはファイルとして読めないので、読み込みの失敗を確実に起こせる。
    let env = Env::new("m");
    fs::create_dir_all(&env.settings).unwrap();
    for sub in ["install", "uninstall"] {
        let out = env.run(&[sub]);
        assert!(!out.status.success(), "{sub}");
        assert!(!out.stderr.is_empty());
        assert!(env.settings.is_dir());
    }

    let env = Env::with_settings("{}");
    let out = env.run(&["install", "--force"]);
    assert!(!out.status.success());
    assert_eq!(env.text(), "{}");
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
fn claude_config_dir_sets_the_default_settings_path() {
    let env = Env::with_settings("{}");
    let out = Command::new(env!("CARGO_BIN_EXE_marimo-hook"))
        .args(["install", "--dry-run"])
        .env("MARIMO_HOME", &env.home)
        .env("CLAUDE_CONFIG_DIR", env.settings.parent().unwrap())
        .env("CODEX_HOME", &env.codex)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout(&out).contains(&env.settings.display().to_string()));
}

// 実行ファイルと名前の先頭だけが同じ他人のコマンドや、command が同じでも args が違うハンドラ、
// 名前の似た実行ファイルに args: ["hook"] を渡すハンドラを、marimo のものと取り違えない。
#[test]
fn look_alike_commands_are_left_alone() {
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
    #[cfg(unix)]
    assert_eq!(
        installed["statusLine"]["command"],
        format!(
            "{} statusline -- sh -c '{foreign_status}'",
            env.installed_exe().display()
        )
    );
    #[cfg(windows)]
    assert_eq!(installed["statusLine"]["command"], foreign_status);

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

    let env = Env::with_settings("{}");
    let exe = env.installed_exe().display().to_string();
    let look_alike = env
        .installed_exe()
        .with_file_name("marimo-hook-backup")
        .display()
        .to_string();
    let foreign = json!([
        {"type": "command", "command": exe, "args": ["record", "Stop"]},
        {"type": "command", "command": exe, "args": ["hook", "--extra"]},
        {"type": "command", "command": look_alike, "args": ["hook"]},
        {"type": "command", "command": env.legacy_hook_command(), "args": []}
    ]);
    let original = pretty(&json!({"hooks": {"Stop": [{"hooks": foreign}]}}));
    fs::write(&env.settings, &original).unwrap();

    assert!(env.run(&["install"]).status.success());
    let installed = env.json();
    let stop = installed["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert_eq!(stop[0]["hooks"], foreign);
    assert_eq!(stop[1], json!({"hooks": [env.exec_handler()]}));

    assert!(env.run(&["uninstall"]).status.success());
    assert_eq!(env.text(), original);
}

// 以前の shell form の登録は、同じグループの同じ位置で exec form に置き換わる。
#[test]
fn legacy_shell_form_migrates_in_place() {
    let env = Env::with_settings("{}");
    let legacy = env.legacy_hook_command();
    let original = json!({
        "hooks": {
            "Stop": [
                {"hooks": [{"type": "command", "command": "say done"}]},
                {"hooks": [
                    {"type": "command", "command": "echo before"},
                    {"type": "command", "command": legacy, "timeout": 5},
                    {"type": "command", "command": "echo after"}
                ]}
            ],
            "Notification": [{"matcher": "", "hooks": [
                {"type": "command", "command": legacy, "async": true, "timeout": 5}
            ]}]
        }
    });
    fs::write(&env.settings, pretty(&original)).unwrap();

    let out = env.run(&["install", "--dry-run"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(
        text.contains("exec form へ書き換えるフック: Notification, Stop"),
        "{text}"
    );
    assert!(text.contains("\"args\""), "{text}");
    assert_eq!(env.json(), original);

    let out = env.run(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let installed = env.json();
    assert_eq!(
        installed["hooks"]["Stop"],
        json!([
            {"hooks": [{"type": "command", "command": "say done"}]},
            {"hooks": [
                {"type": "command", "command": "echo before"},
                env.exec_handler(),
                {"type": "command", "command": "echo after"}
            ]}
        ])
    );
    // 利用者が足した項目とキーの順序は、書き換えた後も残る。
    let migrated = &installed["hooks"]["Notification"][0]["hooks"][0];
    let keys: Vec<_> = migrated.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["type", "command", "args", "async", "timeout"]);
    let mut expected = env.exec_handler();
    expected["async"] = json!(true);
    assert_eq!(
        installed["hooks"]["Notification"],
        json!([{"matcher": "", "hooks": [expected]}])
    );
    assert!(!env.text().contains(&legacy));

    let once = env.text();
    let out = env.run(&["install"]);
    assert!(out.status.success());
    assert_eq!(env.text(), once);
    assert_eq!(env.backups().len(), 1);
    assert!(stdout(&out).contains("変更はありません"));

    assert!(env.run(&["uninstall"]).status.success());
    let after = env.json();
    assert_eq!(
        after["hooks"],
        json!({"Stop": [
            {"hooks": [{"type": "command", "command": "say done"}]},
            {"hooks": [
                {"type": "command", "command": "echo before"},
                {"type": "command", "command": "echo after"}
            ]}
        ]})
    );
}

#[test]
fn uninstall_removes_legacy_and_exec_forms() {
    let env = Env::with_settings("{}");
    let original = json!({
        "hooks": {
            "Stop": [{"hooks": [
                {"type": "command", "command": env.legacy_hook_command(), "timeout": 5}
            ]}],
            "SessionStart": [{"hooks": [env.exec_handler()]}],
            "PreToolUse": [{"matcher": "Bash", "hooks": [
                {"type": "command", "command": "~/.claude/hooks/guard.sh"},
                env.exec_handler()
            ]}]
        },
        "model": "opus"
    });
    fs::write(&env.settings, pretty(&original)).unwrap();
    let out = env.run(&["uninstall"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        env.json(),
        json!({
            "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [
                {"type": "command", "command": "~/.claude/hooks/guard.sh"}
            ]}]},
            "model": "opus"
        })
    );
}

// ホームフォルダの下に置いた実行ファイルは、Git Bash と PowerShell のどちらでも読める ~/ の形で
// statusLine に登録する。
#[cfg(windows)]
#[test]
fn windows_status_line_uses_the_home_relative_path() {
    let env = Env::new("m");
    fs::write(&env.settings, "{}").unwrap();
    let out = env.run(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let installed = env.json();
    assert_eq!(
        installed["statusLine"],
        json!({"type": "command", "command": "~/m/bin/marimo-hook.exe statusline"})
    );
    assert_eq!(
        installed["hooks"]["Stop"],
        json!([{"hooks": [env.exec_handler()]}])
    );

    let once = env.text();
    let out = env.run(&["install"]);
    assert!(out.status.success());
    assert_eq!(env.text(), once);
    assert!(stdout(&out).contains("変更はありません"));

    assert!(env.run(&["uninstall"]).status.success());
    assert_eq!(env.json(), json!({}));
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

const CODEX_EVENTS: [&str; 10] = [
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

// 実行ファイルのパスに空白があるので、Unix では単一引用符で引用される。Windows では一語で書けないので登録しない。
#[test]
fn codex_hooks_round_trip_and_reinstall_changes_nothing() {
    let original = pretty(&json!({
        "description": "my hooks",
        "hooks": {
            "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "./guard.sh", "timeout": 30}]}],
            "PreCompact": [{"hooks": [{"type": "command", "command": "echo compact"}]}]
        }
    }));
    // 前に別の場所の marimo-hook で入れた登録は、入れ直しで置き換わり、uninstall でも消える。
    let mut stale: Value = serde_json::from_str(&original).unwrap();
    stale["hooks"]["Stop"] = json!([
        {"hooks": [{"type": "command", "command": "/old/marimo-hook codex-hook", "timeout": 5}]},
        {"hooks": [{"type": "command", "command": "'/old dir/marimo-hook' codex-hook", "timeout": 5}]}
    ]);
    let stale = pretty(&stale);
    let env = Env::with_codex(Some(&stale));
    let out = env.run(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    if cfg!(windows) {
        assert!(text.contains("Codex のフックは扱いませんでした"), "{text}");
        assert_eq!(fs::read_to_string(env.codex_hooks()).unwrap(), stale);
        return;
    }
    assert!(text.contains("/hooks で"), "{text}");
    let command = format!("'{}' codex-hook", env.installed_exe().display());
    let installed: Value =
        serde_json::from_str(&fs::read_to_string(env.codex_hooks()).unwrap()).unwrap();
    assert_eq!(installed["description"], "my hooks");
    for event in CODEX_EVENTS {
        let groups = installed["hooks"][event].as_array().unwrap();
        assert_eq!(
            groups.last().unwrap(),
            &json!({"hooks": [{"type": "command", "command": command, "timeout": 5}]}),
            "{event}"
        );
    }
    assert_eq!(
        installed["hooks"]["PreToolUse"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        installed["hooks"]["PreCompact"].as_array().unwrap().len(),
        1
    );
    assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 1);

    // 同じ実行ファイルで入れ直しても hooks.json を書き換えないので、Codex の信頼は保たれる。
    let before = fs::read(env.codex_hooks()).unwrap();
    let out = env.run(&["install"]);
    assert!(out.status.success());
    assert_eq!(fs::read(env.codex_hooks()).unwrap(), before);
    assert!(!stdout(&out).contains("/hooks で"));

    let out = env.run(&["uninstall"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(env.codex_hooks()).unwrap(), original);
}

#[test]
fn codex_is_handled_only_when_its_folder_exists() {
    let env = Env::with_settings("{}");
    for args in [&["install"][..], &["uninstall"][..]] {
        let out = env.run(args);
        assert!(out.status.success());
        assert!(!stdout(&out).contains("Codex"), "{args:?}");
    }
    assert!(!env.codex.exists());

    // hooks.json がなければ作る。
    let env = Env::with_codex(None);
    let out = env.run(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    if cfg!(unix) {
        let created: Value =
            serde_json::from_str(&fs::read_to_string(env.codex_hooks()).unwrap()).unwrap();
        assert_eq!(
            created["hooks"].as_object().unwrap().len(),
            CODEX_EVENTS.len()
        );
        assert!(env.run(&["uninstall"]).status.success());
        assert_eq!(fs::read_to_string(env.codex_hooks()).unwrap(), "{}\n");
    }
    assert!(!env.codex.join("config.toml").exists());
}
