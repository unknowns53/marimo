use std::io::Write;
use std::process::{Command, Stdio};

use marimo_core::SessionState;

const TERMINAL: &str = "com.apple.Terminal";
const ITERM: &str = "com.googlecode.iterm2";
const VSCODE: &str = "com.microsoft.VSCode";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    TerminalTab { tty: String },
    ItermSession { tty: String },
    EditorFolder { bundle_id: String, folder: String },
    App { bundle_id: String },
    Nothing,
}

// 端末の見分け方の根拠は次のとおり。Terminal.app の TERM_PROGRAM=Apple_Terminal と
// __CFBundleIdentifier は実機で確かめた。iTerm2 の TERM_PROGRAM=iTerm.app は iTerm2 自身の
// シェル統合スクリプトが同じ値で判定している。VS Code の TERM_PROGRAM=vscode は
// https://code.visualstudio.com/docs/terminal/shell-integration に記載がある。
pub fn plan(session: &SessionState) -> Target {
    let origin = session.origin.clone().unwrap_or_default();
    let term = origin.term_program.as_deref();
    let bundle = origin.bundle_id.as_deref();
    if let Some(tty) = origin.tty.clone() {
        if term == Some("Apple_Terminal") || bundle == Some(TERMINAL) {
            return Target::TerminalTab { tty };
        }
        if term == Some("iTerm.app") || bundle == Some(ITERM) {
            return Target::ItermSession { tty };
        }
    }
    // VS Code の派生（Cursor など）も TERM_PROGRAM=vscode を名乗るので、
    // 開くアプリは記録した bundle id を優先する。
    if term == Some("vscode")
        && let Some(folder) = session.cwd.clone()
    {
        let bundle_id = bundle.unwrap_or(VSCODE).to_owned();
        return Target::EditorFolder { bundle_id, folder };
    }
    match bundle {
        Some(b) => Target::App {
            bundle_id: b.to_owned(),
        },
        None => Target::Nothing,
    }
}

// Apple Events が拒否されたり、タブがすでに閉じられていたりしても、落ちずにアプリを
// 前面に出すところまでで止める。どの経路も待たずに戻れるよう、別スレッドで動かす。
pub fn run(target: Target) {
    std::thread::spawn(move || match target {
        Target::TerminalTab { tty } => {
            if !osascript(TERMINAL_SCRIPT, &tty) {
                open(&["-b", TERMINAL]);
            }
        }
        Target::ItermSession { tty } => {
            if !osascript(ITERM_SCRIPT, &tty) {
                open(&["-b", ITERM]);
            }
        }
        // フォルダを開いているウィンドウがあれば、open はそのウィンドウを前面に出す。
        Target::EditorFolder { bundle_id, folder } => open(&["-b", &bundle_id, &folder]),
        Target::App { bundle_id } => open(&["-b", &bundle_id]),
        Target::Nothing => {}
    });
}

// tty はスクリプトに埋め込まず引数で渡し、文字列の引用の崩れを起こさないようにする。
const TERMINAL_SCRIPT: &str = r#"
on run argv
  set target to item 1 of argv
  if application "Terminal" is not running then return "absent"
  tell application "Terminal"
    repeat with w in windows
      repeat with t in tabs of w
        if tty of t is target then
          set selected of t to true
          set index of w to 1
          activate
          return "ok"
        end if
      end repeat
    end repeat
  end tell
  return "missing"
end run
"#;

// iTerm2 のセッションの tty 属性は https://iterm2.com/documentation-scripting.html に記載がある。
const ITERM_SCRIPT: &str = r#"
on run argv
  set target to item 1 of argv
  if application "iTerm2" is not running then return "absent"
  tell application "iTerm2"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with s in sessions of t
          if tty of s is target then
            tell s to select
            tell t to select
            tell w to select
            activate
            return "ok"
          end if
        end repeat
      end repeat
    end repeat
  end tell
  return "missing"
end run
"#;

fn osascript(script: &str, arg: &str) -> bool {
    let child = Command::new("/usr/bin/osascript")
        .arg("-")
        .arg(arg)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    match child.wait_with_output() {
        Ok(out) => {
            let result = String::from_utf8_lossy(&out.stdout);
            if !out.status.success() {
                eprintln!(
                    "marimo: osascript failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            out.status.success() && result.trim() == "ok"
        }
        Err(_) => false,
    }
}

fn open(args: &[&str]) {
    if let Err(e) = Command::new("/usr/bin/open").args(args).status() {
        eprintln!("marimo: open failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use marimo_core::Origin;

    fn session(bundle: Option<&str>, term: Option<&str>, tty: Option<&str>) -> SessionState {
        SessionState {
            cwd: Some("/w/proj".into()),
            origin: Some(Origin {
                bundle_id: bundle.map(Into::into),
                term_program: term.map(Into::into),
                tty: tty.map(Into::into),
                entrypoint: None,
            }),
            ..SessionState::new("s1")
        }
    }

    #[test]
    fn picks_the_route_for_each_app() {
        let tty = Some("/dev/ttys002");
        assert_eq!(
            plan(&session(Some(TERMINAL), Some("Apple_Terminal"), tty)),
            Target::TerminalTab {
                tty: "/dev/ttys002".into()
            }
        );
        assert_eq!(
            plan(&session(None, Some("iTerm.app"), tty)),
            Target::ItermSession {
                tty: "/dev/ttys002".into()
            }
        );
        assert_eq!(
            plan(&session(
                Some("com.todesktop.230313mzl4w4u92"),
                Some("vscode"),
                tty
            )),
            Target::EditorFolder {
                bundle_id: "com.todesktop.230313mzl4w4u92".into(),
                folder: "/w/proj".into()
            }
        );
        assert_eq!(
            plan(&session(None, Some("vscode"), None)),
            Target::EditorFolder {
                bundle_id: VSCODE.into(),
                folder: "/w/proj".into()
            }
        );
        assert_eq!(
            plan(&session(Some("com.anthropic.claudefordesktop"), None, None)),
            Target::App {
                bundle_id: "com.anthropic.claudefordesktop".into()
            }
        );
        // tty が分からない Terminal.app は、アプリを前面に出すだけにする。
        assert_eq!(
            plan(&session(Some(TERMINAL), Some("Apple_Terminal"), None)),
            Target::App {
                bundle_id: TERMINAL.into()
            }
        );
        assert_eq!(plan(&session(None, None, None)), Target::Nothing);
        assert_eq!(plan(&SessionState::new("s1")), Target::Nothing);
    }
}
