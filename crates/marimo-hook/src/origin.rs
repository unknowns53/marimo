use std::env;

use marimo_core::Origin;

// どのアプリから起動したかを示す公式の環境変数は Claude Code のドキュメントに無いので、
// OS と端末が設定するものだけを使う。Terminal.app では TERM_PROGRAM=Apple_Terminal と
// __CFBundleIdentifier=com.apple.Terminal、デスクトップアプリの Code タブでは
// __CFBundleIdentifier=com.anthropic.claudefordesktop が、フックの環境に入ることを確かめてある。
pub fn detect() -> Origin {
    Origin {
        bundle_id: var("__CFBundleIdentifier"),
        term_program: var("TERM_PROGRAM"),
        tty: ancestor_tty(),
        entrypoint: None,
    }
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
    use std::mem::{MaybeUninit, size_of};

    unsafe extern "C" {
        fn devname(dev: libc::dev_t, kind: libc::mode_t) -> *mut libc::c_char;
    }

    const NODEV: u32 = u32::MAX;
    let mut pid = std::os::unix::process::parent_id() as libc::c_int;
    for _ in 0..16 {
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
        if info.e_tdev != NODEV {
            // SAFETY: devname は静的な領域への NUL 終端文字列か NULL を返す。
            let name = unsafe { devname(info.e_tdev as libc::dev_t, libc::S_IFCHR) };
            if name.is_null() {
                return None;
            }
            let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
            return (!name.is_empty() && name != "??").then(|| format!("/dev/{name}"));
        }
        pid = info.pbi_ppid as libc::c_int;
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn ancestor_tty() -> Option<String> {
    None
}
