use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use marimo_core::{Origin, ProcessRef, Provider, SessionState, WindowRef, hermes};

const TERMINAL: &str = "com.apple.Terminal";
const ITERM: &str = "com.googlecode.iterm2";
const VSCODE: &str = "com.microsoft.VSCode";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    TerminalTab {
        tty: String,
    },
    ItermSession {
        tty: String,
    },
    EditorFolder {
        bundle_id: String,
        folder: String,
    },
    App {
        bundle_id: String,
    },
    /// Windows で記録した手がかり。editor はエディタの実行ファイルと作業フォルダの組で、
    /// 試す順は editor、window、ancestors の近い順である。
    Windows {
        window: Option<WindowRef>,
        editor: Option<(String, String)>,
        ancestors: Vec<ProcessRef>,
    },
    /// Hermes の会話が行われているチャットを開く URL。
    Link {
        url: String,
    },
    Nothing,
}

// VS Code とその派生のうち、Windows で統合ターミナルが TERM_PROGRAM=vscode を名乗るもの。
const WINDOWS_EDITORS: [&str; 5] = [
    "Code.exe",
    "Code - Insiders.exe",
    "VSCodium.exe",
    "Cursor.exe",
    "Windsurf.exe",
];

// 祖先をたどってここまで来たら、それより上は利用者が Claude Code を動かしているアプリではない。
// 前面に出すとデスクトップやエクスプローラーのウィンドウが出てしまうので、ここで打ち切る。
const WINDOWS_SHELL: [&str; 7] = [
    "explorer.exe",
    "svchost.exe",
    "sihost.exe",
    "userinit.exe",
    "winlogon.exe",
    "services.exe",
    "wininit.exe",
];

// 端末の見分け方の根拠は次のとおり。Terminal.app の TERM_PROGRAM=Apple_Terminal と
// __CFBundleIdentifier は実機で確かめた。iTerm2 の TERM_PROGRAM=iTerm.app は iTerm2 自身の
// シェル統合スクリプトが同じ値で判定している。VS Code の TERM_PROGRAM=vscode は
// https://code.visualstudio.com/docs/terminal/shell-integration に記載がある。
pub fn plan(session: &SessionState) -> Target {
    // Hermes は gateway の裏で動き、会話はチャットアプリの中にあるので、そのチャンネルを開く。
    // 状態ファイルの値を OS に渡すので、marimo が組み立てる形の URL だけを通す。
    if session.provider == Provider::Hermes {
        return match session.link.clone().filter(|l| hermes::is_discord_link(l)) {
            Some(url) => Target::Link { url },
            None => Target::Nothing,
        };
    }
    let origin = session.origin.clone().unwrap_or_default();
    if origin.window.is_some() || !origin.ancestors.is_empty() {
        return windows_target(origin, session.cwd.as_deref());
    }
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
    if term == Some("vscode") {
        let bundle_id = bundle.unwrap_or(VSCODE).to_owned();
        // cwd はフックの入力をそのまま記録したものなので、- で始まる値を open が
        // オプションとして読まないよう、絶対パスのときだけフォルダとして渡す。
        return match session.cwd.clone().filter(|c| c.starts_with('/')) {
            Some(folder) => Target::EditorFolder { bundle_id, folder },
            None => Target::App { bundle_id },
        };
    }
    match bundle {
        Some(b) => Target::App {
            bundle_id: b.to_owned(),
        },
        None => Target::Nothing,
    }
}

// VS Code の統合ターミナルは持ち主のない ConPTY で動くので、コンソールからたどったウィンドウは
// 見えないものになり、記録されないと見込んで、エディタにフォルダを渡す経路を先に試す。この見込みは
// 実機で確かめる必要がある。祖先にエディタの実行ファイルがあれば、それが同じ派生のエディタである。
fn windows_target(origin: Origin, cwd: Option<&str>) -> Target {
    let ancestors: Vec<ProcessRef> = origin
        .ancestors
        .into_iter()
        .take_while(|p| !WINDOWS_SHELL.iter().any(|s| exe_name_is(&p.exe, s)))
        .collect();
    let editor = if origin.term_program.as_deref() == Some("vscode") {
        let folder = cwd.filter(|c| is_windows_absolute(c));
        let exe = ancestors
            .iter()
            .find(|p| WINDOWS_EDITORS.iter().any(|e| exe_name_is(&p.exe, e)));
        folder.zip(exe).map(|(f, p)| (p.exe.clone(), f.to_owned()))
    } else {
        None
    };
    Target::Windows {
        window: origin.window,
        editor,
        ancestors,
    }
}

fn exe_name_is(path: &str, name: &str) -> bool {
    path.rsplit(['\\', '/'])
        .next()
        .is_some_and(|n| n.eq_ignore_ascii_case(name))
}

// cwd は Windows の形のパスで、macOS でも試験できるよう std::path には頼らずに見分ける。
// `C:\` で始まるドライブの絶対パスと、`\\server\share` の UNC パスや `\\?\C:\` で始まる形だけを受け付け、
// `C:foo` や `\foo` のようにドライブか作業フォルダに依存するものは受け付けない。
fn is_windows_absolute(path: &str) -> bool {
    let sep = |b: u8| b == b'\\' || b == b'/';
    match path.as_bytes() {
        [drive, b':', s, ..] => drive.is_ascii_alphabetic() && sep(*s),
        [a, b, c, ..] => sep(*a) && sep(*b) && !sep(*c),
        _ => false,
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
        Target::EditorFolder { bundle_id, folder } => {
            open(&editor_args(&bundle_id, &folder, |p| p.is_dir()));
        }
        Target::App { bundle_id } => open(&["-b", &bundle_id]),
        Target::Windows {
            window,
            editor,
            ancestors,
        } => focus_on_windows(window, editor, ancestors),
        Target::Link { url } => open_link(&url),
        Target::Nothing => {}
    });
}

// Discord のリンクを開く Windows の実行ファイル。安定版のほかに、公開試験版と開発版がある。
#[cfg(windows)]
const DISCORD_EXES: [&str; 3] = ["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"];

// URL の scheme に結び付いたアプリ（Discord）が開く。シェルを通さないので、URL の文字が命令として
// 読まれることはない。Windows の Discord は、動いている本体へリンクを渡して画面を切り替えるだけで、
// 前面には出てこない（実機で確かめた）。行を押した直後の marimo は前面に出す権利を持っているので、
// 画面が切り替わるのを少し待ってから、Discord のウィンドウをこちらから前面に出す。
#[cfg(windows)]
fn open_link(url: &str) {
    use marimo_core::winfocus;
    if winfocus::open_url(url) {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = winfocus::focus_app(&DISCORD_EXES);
    }
}

// macOS の open は、URL を渡したアプリを前面に出す。
#[cfg(not(windows))]
fn open_link(url: &str) {
    open(&[url]);
}

#[cfg(windows)]
fn focus_on_windows(
    window: Option<WindowRef>,
    editor: Option<(String, String)>,
    ancestors: Vec<ProcessRef>,
) {
    use marimo_core::winfocus;
    if let Some((exe, folder)) = editor
        && winfocus::open_folder_with(&exe, &folder)
    {
        return;
    }
    if window.as_ref().is_some_and(winfocus::focus_window) {
        return;
    }
    let _ = ancestors.iter().any(winfocus::focus_process);
}

// Windows の手がかりは Windows のフックしか記録しないので、ほかの OS ではここに来ない。
#[cfg(not(windows))]
fn focus_on_windows(_: Option<WindowRef>, _: Option<(String, String)>, _: Vec<ProcessRef>) {}

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

// フォルダが消えていれば、アプリを前面に出すだけにする。
fn editor_args<'a>(
    bundle_id: &'a str,
    folder: &'a str,
    is_dir: impl Fn(&Path) -> bool,
) -> Vec<&'a str> {
    if is_dir(Path::new(folder)) {
        vec!["-b", bundle_id, folder]
    } else {
        vec!["-b", bundle_id]
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

    fn session(bundle: Option<&str>, term: Option<&str>, tty: Option<&str>) -> SessionState {
        session_in("/w/proj", bundle, term, tty)
    }

    fn session_in(
        cwd: &str,
        bundle: Option<&str>,
        term: Option<&str>,
        tty: Option<&str>,
    ) -> SessionState {
        SessionState {
            cwd: Some(cwd.into()),
            origin: Some(Origin {
                bundle_id: bundle.map(Into::into),
                term_program: term.map(Into::into),
                tty: tty.map(Into::into),
                ..Origin::default()
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

        // open がオプションとして読まないよう、絶対パスでない cwd はフォルダとして渡さない。
        for cwd in ["-a", "--args", "w/proj", "."] {
            assert_eq!(
                plan(&session_in(cwd, None, Some("vscode"), None)),
                Target::App {
                    bundle_id: VSCODE.into()
                },
                "{cwd}"
            );
        }
        let mut no_cwd = session(None, Some("vscode"), None);
        no_cwd.cwd = None;
        assert_eq!(
            plan(&no_cwd),
            Target::App {
                bundle_id: VSCODE.into()
            }
        );
    }

    fn process(pid: u32, exe: &str) -> ProcessRef {
        ProcessRef {
            pid,
            created: 133_700_000_000_000_000 + u64::from(pid),
            exe: exe.into(),
        }
    }

    fn windows_session(cwd: &str, term: Option<&str>, origin: Origin) -> SessionState {
        SessionState {
            cwd: Some(cwd.into()),
            origin: Some(Origin {
                term_program: term.map(Into::into),
                ..origin
            }),
            ..SessionState::new("s1")
        }
    }

    const CLAUDE: &str = r"C:\Users\user\.local\bin\claude.exe";

    #[test]
    fn windows_clues_pick_the_windows_route() {
        let window = WindowRef {
            hwnd: 0x1_0a2c,
            pid: 4120,
            created: 133_700_000_000_000_000,
        };
        let terminal = windows_session(
            r"C:\Users\user\proj",
            None,
            Origin {
                window: Some(window.clone()),
                ancestors: vec![
                    process(5008, CLAUDE),
                    process(4400, r"C:\Program Files\PowerShell\7\pwsh.exe"),
                    process(4120, r"C:\Program Files\WindowsApps\WindowsTerminal.exe"),
                    process(3000, r"C:\Windows\explorer.exe"),
                    process(900, r"C:\Windows\System32\winlogon.exe"),
                ],
                ..Origin::default()
            },
        );
        // エクスプローラーから上は前面に出す候補にしない。
        assert_eq!(
            plan(&terminal),
            Target::Windows {
                window: Some(window),
                editor: None,
                ancestors: vec![
                    process(5008, CLAUDE),
                    process(4400, r"C:\Program Files\PowerShell\7\pwsh.exe"),
                    process(4120, r"C:\Program Files\WindowsApps\WindowsTerminal.exe"),
                ],
            }
        );

        let only_ancestors = windows_session(
            r"C:\Users\user\proj",
            None,
            Origin {
                ancestors: vec![process(5008, CLAUDE)],
                ..Origin::default()
            },
        );
        assert_eq!(
            plan(&only_ancestors),
            Target::Windows {
                window: None,
                editor: None,
                ancestors: vec![process(5008, CLAUDE)],
            }
        );
    }

    #[test]
    fn windows_vscode_opens_the_folder_with_the_ancestor_editor() {
        let cursor = r"C:\Users\user\AppData\Local\Programs\cursor\CURSOR.EXE";
        let origin = Origin {
            ancestors: vec![
                process(5008, CLAUDE),
                process(
                    4400,
                    r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                ),
                process(4300, cursor),
                process(
                    4100,
                    r"C:\Users\user\AppData\Local\Programs\Microsoft VS Code\Code.exe",
                ),
            ],
            ..Origin::default()
        };
        let Target::Windows { editor, .. } = plan(&windows_session(
            r"C:\Users\user\proj",
            Some("vscode"),
            origin.clone(),
        )) else {
            panic!("expected the Windows route");
        };
        assert_eq!(
            editor,
            Some((cursor.to_owned(), r"C:\Users\user\proj".to_owned()))
        );

        // TERM_PROGRAM が vscode でなければ、祖先にエディタがあってもフォルダは開かない。
        let Target::Windows { editor, .. } = plan(&windows_session(
            r"C:\Users\user\proj",
            None,
            origin.clone(),
        )) else {
            panic!("expected the Windows route");
        };
        assert_eq!(editor, None);

        for cwd in [r"proj", r"C:proj", r"\proj", "-a", "/w/proj"] {
            let Target::Windows {
                editor, ancestors, ..
            } = plan(&windows_session(cwd, Some("vscode"), origin.clone()))
            else {
                panic!("expected the Windows route");
            };
            assert_eq!((editor, ancestors.len()), (None, 4), "{cwd}");
        }
    }

    #[test]
    fn windows_absolute_paths() {
        for path in [
            r"C:\Users\user\proj",
            r"c:\",
            "C:/Users/user/proj",
            r"\\server\share\proj",
            r"\\?\C:\Users\user\proj",
            "//server/share",
        ] {
            assert!(is_windows_absolute(path), "{path}");
        }
        for path in [
            "",
            "C:",
            "C:proj",
            r"\proj",
            "/w/proj",
            r"\\",
            r"\\\server",
            "proj",
            "-a",
            "1:\\x",
        ] {
            assert!(!is_windows_absolute(path), "{path}");
        }
    }

    #[test]
    fn hermes_opens_its_chat_and_ignores_the_launching_terminal() {
        let hermes = |link: Option<&str>| SessionState {
            provider: Provider::Hermes,
            link: link.map(Into::into),
            ..session(Some(TERMINAL), Some("Apple_Terminal"), Some("/dev/ttys001"))
        };
        assert_eq!(
            plan(&hermes(Some("discord://-/channels/1/2"))),
            Target::Link {
                url: "discord://-/channels/1/2".into()
            }
        );
        assert_eq!(plan(&hermes(None)), Target::Nothing);
        assert_eq!(
            plan(&hermes(Some("file:///C:/Windows/System32/calc.exe"))),
            Target::Nothing
        );
    }
}
