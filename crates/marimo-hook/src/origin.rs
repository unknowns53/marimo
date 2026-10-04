use std::env;

use marimo_core::Origin;

// どのアプリから起動したかを示す公式の環境変数は Claude Code のドキュメントに無いので、
// OS と端末が設定するものだけを使う。Terminal.app では TERM_PROGRAM=Apple_Terminal と
// __CFBundleIdentifier=com.apple.Terminal、デスクトップアプリの Code タブでは
// __CFBundleIdentifier=com.anthropic.claudefordesktop が、フックの環境に入ることを確かめてある。
pub fn detect(event: &str, codex_originator: Option<&str>) -> Origin {
    let mut origin = Origin {
        bundle_id: var("__CFBundleIdentifier"),
        term_program: var("TERM_PROGRAM"),
        tty: ancestor_tty(),
        ..Origin::default()
    };
    if origin.bundle_id.is_none() && may_have_moved(event) {
        origin.bundle_id = app_bundle_clue(codex_originator);
    }
    windows_clues(&mut origin, event);
    origin
}

// 祖先のプロセスやコンソールを調べるのは、環境変数を読むより時間がかかる。起動元が変わりうるのは
// セッションの開始と --resume の後だけなので、その後に必ず届くイベントでだけ調べ、ほかのイベントでは
// 保存済みの値を Origin::merge に残させる。
fn may_have_moved(event: &str) -> bool {
    matches!(event, "SessionStart" | "UserPromptSubmit")
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

// フックは制御端末を持たない別のセッションで動く（hooks のドキュメントの Hook input and output）。
// 端末は、フックを起動した claude プロセスの側が持っているので、祖先をたどって最初に見つかる
// 端末を使う。ps を起動すると数ミリ秒かかるので、カーネルに直接尋ねる。
#[cfg(target_os = "macos")]
fn ancestor_tty() -> Option<String> {
    use std::ffi::CStr;

    unsafe extern "C" {
        fn devname(dev: libc::dev_t, kind: libc::mode_t) -> *mut libc::c_char;
    }

    const NODEV: u32 = u32::MAX;
    let info = ancestors().find(|info| info.e_tdev != NODEV)?;
    // SAFETY: devname は静的な領域への NUL 終端文字列か NULL を返す。
    let name = unsafe { devname(info.e_tdev as libc::dev_t, libc::S_IFCHR) };
    if name.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    (!name.is_empty() && name != "??").then(|| format!("/dev/{name}"))
}

/// フックの親から launchd の手前まで、祖先の情報を近い順に最大 16 個返す。
#[cfg(target_os = "macos")]
fn ancestors() -> impl Iterator<Item = libc::proc_bsdinfo> {
    use std::mem::{MaybeUninit, size_of};

    let mut pid = std::os::unix::process::parent_id() as libc::c_int;
    std::iter::from_fn(move || {
        if pid <= 1 {
            return None;
        }
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: バッファは proc_bsdinfo の大きさで確保し、その大きさを渡している。
        let n = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if n != size {
            return None;
        }
        // SAFETY: proc_pidinfo が構造体の大きさぶんを書いたことを n で確かめた。
        let info = unsafe { info.assume_init() };
        pid = info.pbi_ppid as libc::c_int;
        Some(info)
    })
    .take(16)
}

#[cfg(not(target_os = "macos"))]
fn ancestor_tty() -> Option<String> {
    None
}

// Codex のデスクトップアプリ（/Applications/ChatGPT.app）から始めた会話のフックには、
// __CFBundleIdentifier も TERM_PROGRAM も入らない。そこで、祖先の実行ファイルを収めたアプリの
// Info.plist から bundle id を読む。アプリの外にある常駐のプロセスから動く会話は祖先にアプリを
// 持たないので、Codex のフックでは rollout の originator も見る。
#[cfg(target_os = "macos")]
fn app_bundle_clue(codex_originator: Option<&str>) -> Option<String> {
    const CODEX_DESKTOP: &str = "com.openai.codex";
    ancestor_bundle_id().or_else(|| {
        codex_originator
            .filter(|o| marimo_core::codex::started_by_desktop_app(o))
            .map(|_| CODEX_DESKTOP.to_owned())
    })
}

#[cfg(not(target_os = "macos"))]
fn app_bundle_clue(_: Option<&str>) -> Option<String> {
    None
}

// Claude Code の CLI は ~/Library/Application Support/Claude/claude-code の下の claude.app に入っていて、
// その Info.plist は LSBackgroundOnly を持つ。前面に出せるアプリではないので、その先の祖先を探す。
#[cfg(target_os = "macos")]
fn ancestor_bundle_id() -> Option<String> {
    ancestors().find_map(|info| {
        let exe = pid_path(info.pbi_pid as libc::c_int)?;
        let app = outermost_app(&exe)?;
        let plist = std::fs::read(std::path::Path::new(app).join("Contents/Info.plist")).ok()?;
        foreground_bundle_id(std::str::from_utf8(&plist).ok()?).map(str::to_owned)
    })
}

#[cfg(target_os = "macos")]
fn pid_path(pid: libc::c_int) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: バッファの大きさをそのまま渡している。
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    buf.truncate(usize::try_from(n).ok()?);
    String::from_utf8(buf).ok().filter(|p| !p.is_empty())
}

// アプリは補助のアプリを中に入れることがある（ChatGPT.app の Contents/Resources/codex-cli/CodexCLI.app、
// VS Code の Code Helper.app など）ので、利用者が開いたアプリである一番外側の .app を使う。
#[cfg(any(target_os = "macos", test))]
fn outermost_app(exe: &str) -> Option<&str> {
    if !exe.starts_with('/') {
        return None;
    }
    let mut end = 0;
    for part in exe.split('/') {
        end += part.len();
        if part.len() > ".app".len() && part.ends_with(".app") {
            return (end < exe.len()).then(|| &exe[..end]);
        }
        end += 1;
    }
    None
}

// 手元のアプリの Info.plist は XML なので、CFBundleIdentifier の直後の string だけを拾う。
// バイナリの plist や想定と違う形のものは読まずに None とする。
#[cfg(any(target_os = "macos", test))]
fn foreground_bundle_id(plist: &str) -> Option<&str> {
    fn value<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
        let at = plist.find(&format!("<key>{key}</key>"))?;
        Some(plist[at + key.len() + "<key></key>".len()..].trim_start())
    }
    if value(plist, "LSBackgroundOnly").is_some_and(|v| v.starts_with("<true/>")) {
        return None;
    }
    let rest = value(plist, "CFBundleIdentifier")?.strip_prefix("<string>")?;
    let id = rest[..rest.find("</string>")?].trim();
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    (!id.is_empty() && id.chars().all(plain)).then_some(id)
}

// 呼ばれるのは標準入力を読み終えた後で、hook は標準出力に何も書かない。
#[cfg(windows)]
fn windows_clues(origin: &mut Origin, event: &str) {
    if !may_have_moved(event) {
        return;
    }
    let ancestors = marimo_core::winfocus::lineage();
    origin.window = marimo_core::winfocus::console_window(&ancestors);
    origin.ancestors = ancestors;
}

#[cfg(not(windows))]
fn windows_clues(_: &mut Origin, _: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outermost_app_takes_the_bundle_the_user_opened() {
        let cases = [
            (
                "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
                Some("/Applications/ChatGPT.app"),
            ),
            (
                "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper",
                Some("/Applications/Visual Studio Code.app"),
            ),
            (
                "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal",
                Some("/System/Applications/Utilities/Terminal.app"),
            ),
            ("/bin/zsh", None),
            (
                "/Users/u/.codex/packages/app-server-daemon/releases/1.0.0/bin/codex",
                None,
            ),
            ("/Applications/.app/Contents/MacOS/x", None),
            (r"C:\Program Files\Foo.app\foo.exe", None),
        ];
        for (exe, want) in cases {
            assert_eq!(outermost_app(exe), want, "{exe}");
        }
    }

    #[test]
    fn foreground_bundle_id_reads_xml_info_plists_only() {
        let plist = |body: &str| {
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
            )
        };
        let cases = [
            (
                plist(
                    "\t<key>CFBundleExecutable</key>\n\t<string>ChatGPT</string>\n\t<key>CFBundleIdentifier</key>\n\t<string>com.openai.codex</string>\n",
                ),
                Some("com.openai.codex"),
            ),
            (
                plist("\t<key>CFBundleName</key>\n\t<string>Foo</string>\n"),
                None,
            ),
            (
                plist(
                    "\t<key>CFBundleIdentifier</key>\n\t<string>com.anthropic.claude-code</string>\n\t<key>LSBackgroundOnly</key>\n\t<true/>\n",
                ),
                None,
            ),
            (
                plist("\t<key>CFBundleIdentifier</key>\n\t<string>a&amp;b</string>\n"),
                None,
            ),
            (
                "bplist00\u{d1}\u{1}\u{2}_\u{10}\u{12}CFBundleIdentifier_\u{10}\u{f}com.example.app"
                    .to_owned(),
                None,
            ),
        ];
        for (text, want) in &cases {
            assert_eq!(foreground_bundle_id(text), *want, "{text}");
        }
    }
}
