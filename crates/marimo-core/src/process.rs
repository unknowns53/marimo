//! フックを起動したプロセスを記録し、そのプロセスが今も動いているかを調べる。

use crate::state::HostProcess;

/// このプロセスの親。親が先に終わっていて別のプロセスにすり替わっているときや、調べられないときは None を返す。
pub fn parent() -> Option<HostProcess> {
    let me = imp::describe(std::process::id())?;
    let parent = imp::describe(imp::parent_pid()?)?;
    // 親の ID が使い回されていれば、親は自分より後に作られたプロセスになる。
    (parent.created <= me.created).then_some(parent)
}

/// 記録したプロセスが終わったと確かめられたときだけ true を返す。調べる権限が無いなど、確かめられない
/// ときは動いているものとみなす。
pub fn has_exited(p: &HostProcess) -> bool {
    imp::has_exited(p)
}

#[cfg(windows)]
mod imp {
    pub use crate::winfocus::{
        describe_process as describe, parent_pid, process_exited as has_exited,
    };
}

#[cfg(target_os = "macos")]
mod imp {
    use std::mem::{MaybeUninit, size_of};

    use crate::state::HostProcess;

    // 親が終わると launchd（ID 1）の子に付け替えられる。
    pub fn parent_pid() -> Option<u32> {
        Some(std::os::unix::process::parent_id()).filter(|&pid| pid > 1)
    }

    pub fn describe(pid: u32) -> Option<HostProcess> {
        let info = bsdinfo(pid).ok()?;
        Some(HostProcess {
            pid,
            created: started(&info),
        })
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
    use crate::state::HostProcess;

    pub fn parent_pid() -> Option<u32> {
        None
    }

    pub fn describe(_: u32) -> Option<HostProcess> {
        None
    }

    pub fn has_exited(_: &HostProcess) -> bool {
        false
    }
}
