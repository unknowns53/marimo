//! フックを起動したプロセスを記録し、そのプロセスが今も動いているかを調べる。

use crate::state::HostProcess;

struct Process {
    host: HostProcess,
    /// 実行ファイルの名前。パスは含まない。
    name: String,
    parent: Option<u32>,
}

// Windows の Claude Code は Git Bash を通してフックを起動し、Git Bash の bash.exe は exec しても
// 親として残る。シェルはフックと一緒に終わるので、記録するとすぐに終わったと判定してしまう。
const SHELLS: [&str; 9] = [
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "fish",
    "cmd",
    "powershell",
    "pwsh",
];

const MAX_HOPS: usize = 4;

/// フックを起動したプロセス。シェルを通して起動されたときは、シェルを飛ばした先の祖先を返す。
/// 祖先が先に終わっていて別のプロセスにすり替わっているときや、調べられないときは None を返す。
pub fn host() -> Option<HostProcess> {
    // Codex は SHELL（Windows では COMSPEC）のシェルでフックを起動するので、一覧に無いシェルを
    // 設定していても飛ばせるよう、それも加える。
    let configured: Vec<String> = ["SHELL", "COMSPEC"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter_map(|v| Some(stem(std::path::Path::new(&v).file_name()?.to_str()?)))
        .collect();
    let is_shell = |name: &str| {
        let name = stem(name);
        SHELLS.contains(&name.as_str()) || configured.contains(&name)
    };
    find_host(std::process::id(), imp::inspector(), is_shell)
}

fn find_host(
    me: u32,
    inspect: impl Fn(u32) -> Option<Process>,
    is_shell: impl Fn(&str) -> bool,
) -> Option<HostProcess> {
    let mut child = inspect(me)?;
    for _ in 0..MAX_HOPS {
        let process = inspect(child.parent?)?;
        // 親の ID が使い回されていれば、親は子より後に作られたプロセスになる。
        if process.host.created > child.host.created {
            return None;
        }
        if !is_shell(&process.name) {
            return Some(process.host);
        }
        child = process;
    }
    None
}

// ログインシェルは名前の頭に - を付けて起動される。
fn stem(name: &str) -> String {
    let name = name.trim_start_matches('-').to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_owned()
}

/// 記録したプロセスが終わったと確かめられたときだけ true を返す。調べる権限が無いなど、確かめられない
/// ときは動いているものとみなす。
pub fn has_exited(p: &HostProcess) -> bool {
    imp::has_exited(p)
}

#[cfg(windows)]
mod imp {
    use super::Process;
    use crate::winfocus::{describe_process, process_table};

    pub use crate::winfocus::process_exited as has_exited;

    pub fn inspector() -> impl Fn(u32) -> Option<Process> {
        let table = process_table();
        move |pid| {
            let (parent, name) = table.get(&pid)?;
            Some(Process {
                host: describe_process(pid)?,
                name: name.clone(),
                parent: Some(*parent),
            })
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::mem::{MaybeUninit, size_of};

    use std::ffi::CStr;

    use super::Process;
    use crate::state::HostProcess;

    // 親が終わると launchd（ID 1）の子に付け替えられるので、そこから先はたどらない。
    pub fn inspector() -> impl Fn(u32) -> Option<Process> {
        |pid| {
            let info = bsdinfo(pid).ok()?;
            let comm = info.pbi_comm.map(|c| c as u8);
            let name = CStr::from_bytes_until_nul(&comm).ok()?;
            Some(Process {
                host: HostProcess {
                    pid,
                    created: started(&info),
                },
                name: name.to_string_lossy().into_owned(),
                parent: Some(info.pbi_ppid).filter(|&p| p > 1),
            })
        }
    }

    // 終わっても親が回収するまではゾンビとして残り、情報も読めるので、状態も見る。
    pub fn has_exited(p: &HostProcess) -> bool {
        match bsdinfo(p.pid) {
            Ok(info) => info.pbi_status == libc::SZOMB || started(&info) != p.created,
            Err(errno) => errno == libc::ESRCH,
        }
    }

    fn started(info: &libc::proc_bsdinfo) -> u64 {
        info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec
    }

    fn bsdinfo(pid: u32) -> Result<libc::proc_bsdinfo, i32> {
        let pid = libc::c_int::try_from(pid).map_err(|_| libc::ESRCH)?;
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
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        // SAFETY: proc_pidinfo が構造体の大きさぶんを書いたことを n で確かめた。
        Ok(unsafe { info.assume_init() })
    }
}

// marimo が動くのは macOS と Windows だけで、ほかの OS ではプロセスを記録しない。
#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::Process;
    use crate::state::HostProcess;

    pub fn inspector() -> impl Fn(u32) -> Option<Process> {
        |_| None
    }

    pub fn has_exited(_: &HostProcess) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_is_the_nearest_ancestor_that_is_not_a_shell() {
        // (pid, 親, 作成時刻, 名前)
        type Row = (u32, u32, u64, &'static str);
        let run = |procs: &[Row]| {
            find_host(
                10,
                |pid| {
                    let &(_, parent, created, name) = procs.iter().find(|p| p.0 == pid)?;
                    Some(Process {
                        host: HostProcess { pid, created },
                        name: name.to_owned(),
                        parent: Some(parent),
                    })
                },
                |name| SHELLS.contains(&stem(name).as_str()) || stem(name) == "nu",
            )
            .map(|h| h.pid)
        };
        let hook = (10, 9, 50, "marimo-hook");
        let cases: [(&str, Vec<Row>, Option<u32>); 5] = [
            ("direct", vec![hook, (9, 1, 40, "claude")], Some(9)),
            (
                "through git bash",
                vec![hook, (9, 8, 45, "bash.exe"), (8, 1, 40, "claude.exe")],
                Some(8),
            ),
            (
                "configured shell",
                vec![hook, (9, 8, 45, "nu"), (8, 1, 40, "codex")],
                Some(8),
            ),
            (
                "reused parent id",
                vec![hook, (9, 8, 45, "-zsh"), (8, 1, 60, "claude")],
                None,
            ),
            (
                "only shells",
                vec![
                    hook,
                    (9, 8, 45, "sh"),
                    (8, 7, 44, "cmd.exe"),
                    (7, 6, 43, "pwsh.exe"),
                    (6, 5, 42, "bash"),
                    (5, 1, 41, "claude"),
                ],
                None,
            ),
        ];
        for (label, procs, expected) in cases {
            assert_eq!(run(&procs), expected, "{label}");
        }
    }
}
