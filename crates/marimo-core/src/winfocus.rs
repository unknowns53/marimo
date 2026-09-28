use crate::state::ProcessRef;

#[cfg(windows)]
pub use imp::{console_window, focus_process, focus_window, lineage, open_folder_with};

const MAX_ANCESTORS: usize = 8;

// 親のプロセス ID は、親が先に終わると別のプロセスに使い回される。子より後に作られたプロセスを
// 親として拾わないよう作成時刻を比べ、使い回しで親子の関係が輪になった場合にも止まるようにする。
// ID 0 は System Idle Process、4 は System で、その先に利用者のアプリは無い。
#[cfg(any(windows, test))]
fn walk_ancestors(
    start: u32,
    start_created: Option<u64>,
    parent_of: impl Fn(u32) -> Option<u32>,
    mut describe: impl FnMut(u32) -> Option<(u64, String)>,
) -> Vec<ProcessRef> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::from([start]);
    let (mut child, mut child_created) = (start, start_created);
    while out.len() < MAX_ANCESTORS {
        let Some(pid) = parent_of(child) else { break };
        if pid == 0 || pid == 4 || !seen.insert(pid) {
            break;
        }
        let Some((created, exe)) = describe(pid) else {
            break;
        };
        if child_created.is_some_and(|c| created > c) {
            break;
        }
        out.push(ProcessRef { pid, created, exe });
        (child, child_created) = (pid, Some(created));
    }
    out
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::iter;
    use std::mem::size_of;
    use std::path::Path;
    use std::process::Command;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, FILETIME, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, FreeConsole, GetConsoleWindow, GetStdHandle,
        STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, EnumWindows, GA_ROOTOWNER, GW_OWNER, GWL_EXSTYLE,
        GetAncestor, GetWindow, GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow, WS_EX_TOOLWINDOW,
    };
    use windows_sys::core::BOOL;

    use super::walk_ancestors;
    use crate::state::{ProcessRef, WindowRef};

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: Handle は開くのに成功したハンドルだけを持ち、閉じるのはここの一度だけである。
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open_process(pid: u32) -> Option<Handle> {
        // SAFETY: 引数は値だけで、失敗すると NULL が返る。
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        (!h.is_null()).then_some(Handle(h))
    }

    fn created_at(process: &Handle) -> Option<u64> {
        let [mut created, mut exited, mut kernel, mut user] = [FILETIME::default(); 4];
        // SAFETY: ハンドルは開いたままで、四つの FILETIME はどれもこの関数の間だけ有効な可変参照である。
        let ok = unsafe {
            GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user)
        };
        (ok != 0)
            .then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    fn image_path(process: &Handle) -> Option<String> {
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: len には buf の要素数を渡し、関数は書いた文字数を len に返す。
        let ok = unsafe {
            QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        };
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    fn process_created(pid: u32) -> Option<u64> {
        created_at(&open_process(pid)?)
    }

    fn parent_map() -> HashMap<u32, u32> {
        let mut map = HashMap::new();
        // SAFETY: 引数は値だけで、失敗すると INVALID_HANDLE_VALUE が返る。
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE {
            return map;
        }
        let snap = Handle(snap);
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: entry の dwSize に構造体の大きさを入れてあり、スナップショットは開いたままである。
        let mut ok = unsafe { Process32FirstW(snap.0, &mut entry) };
        while ok != 0 {
            map.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            // SAFETY: 上と同じ。
            ok = unsafe { Process32NextW(snap.0, &mut entry) };
        }
        map
    }

    /// このプロセスの祖先を、近い順に最大 8 個返す。
    pub fn lineage() -> Vec<ProcessRef> {
        let parents = parent_map();
        let me = std::process::id();
        walk_ancestors(
            me,
            process_created(me),
            |pid| parents.get(&pid).copied(),
            |pid| {
                let process = open_process(pid)?;
                let created = created_at(&process)?;
                Some((created, image_path(&process).unwrap_or_default()))
            },
        )
    }

    fn window_pid(hwnd: HWND) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: 無効なハンドルなら 0 を返すだけで、pid はこの関数の間だけ有効な可変参照である。
        let tid = unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        (tid != 0 && pid != 0).then_some(pid)
    }

    /// 親か祖先のコンソールを表示しているウィンドウを返す。呼んだプロセスは自分のコンソールから離れたままになるので、
    /// 標準入力を読み終えたフックからだけ呼ぶ。見つからなければ None を返す。
    // ConPTY の中では GetConsoleWindow は表示されないウィンドウを返す
    // (https://learn.microsoft.com/en-us/windows/console/getconsolewindow)。
    // Windows Terminal はそのウィンドウを自分のウィンドウの持ち物として作るので、GA_ROOTOWNER で
    // Windows Terminal のウィンドウに届き、conhost ではコンソールのウィンドウ自身が返る
    // (microsoft/terminal PR #13118)。VS Code の端末では持ち主が無く、見えないウィンドウのままなので捨てる。
    pub fn console_window(ancestors: &[ProcessRef]) -> Option<WindowRef> {
        // フックは Claude Code が隠したコンソールで動いていることがあるので、自分のコンソールは使わない。
        // 一つのプロセスがつながれるコンソールは一つだけなので、先に離れてから親や祖先のものへつなぐ
        // (https://learn.microsoft.com/en-us/windows/console/attachconsole)。
        // 親がシェルで、それも隠れたコンソールにいる場合に備えて、祖先を近い順に試す。
        // FreeConsole はコンソールのハンドルだけを閉じるが、コンソールをつなぎ直すと標準ハンドルが
        // 書き換えられうる。Claude Code とつながったパイプのハンドルを最後に戻しておく。
        let std_handles = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(saved);
        let mut candidates =
            iter::once(ATTACH_PARENT_PROCESS).chain(ancestors.iter().skip(1).map(|p| p.pid));
        // SAFETY: 引数を取らず、コンソールにつながっていなければ失敗を返すだけである。
        unsafe { FreeConsole() };
        let found = candidates.find_map(|pid| {
            let window = attached_window(pid);
            // SAFETY: 上と同じ。
            unsafe { FreeConsole() };
            window
        });
        for (kind, handle) in std_handles {
            // SAFETY: 起動時に受け取った標準ハンドルの値をそのまま戻すだけで、ハンドル自体は閉じていない。
            unsafe { SetStdHandle(kind, handle) };
        }
        found
    }

    fn saved(kind: STD_HANDLE) -> (STD_HANDLE, HANDLE) {
        // SAFETY: 引数は値だけで、ハンドルの値を返すだけである。
        (kind, unsafe { GetStdHandle(kind) })
    }

    fn attached_window(pid: u32) -> Option<WindowRef> {
        // SAFETY: 引数は値だけで、つなげなければ 0 を返す。
        if unsafe { AttachConsole(pid) } == 0 {
            return None;
        }
        // SAFETY: 引数を取らず、ウィンドウが無ければ NULL を返す。
        let console = unsafe { GetConsoleWindow() };
        if console.is_null() {
            return None;
        }
        // SAFETY: console は GetConsoleWindow が返した NULL でないハンドルである。
        let root = unsafe { GetAncestor(console, GA_ROOTOWNER) };
        // SAFETY: 無効なハンドルなら 0 を返すだけである。
        if root.is_null() || unsafe { IsWindowVisible(root) } == 0 {
            return None;
        }
        let pid = window_pid(root)?;
        Some(WindowRef {
            hwnd: root as usize as u64,
            pid,
            created: process_created(pid)?,
        })
    }

    // 利用者が行を押した直後に呼ぶので、「呼んだプロセスが最後の入力を受け取った」という条件で
    // SetForegroundWindow が許されることを期待する。拒まれるとタスクバーのボタンが点滅するだけになる
    // (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow)。
    fn bring_forward(hwnd: HWND) -> bool {
        // SAFETY: どの関数も、ハンドルが既に無効なら失敗を返すだけである。
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd) != 0
        }
    }

    /// 記録したウィンドウがまだ同じプロセスのものなら前面に出す。前面に出せたかを返す。
    pub fn focus_window(w: &WindowRef) -> bool {
        let hwnd = w.hwnd as usize as HWND;
        // SAFETY: 無効なハンドルなら 0 を返すだけである。
        let shown = unsafe { IsWindow(hwnd) != 0 && IsWindowVisible(hwnd) != 0 };
        shown
            && window_pid(hwnd) == Some(w.pid)
            && process_created(w.pid) == Some(w.created)
            && bring_forward(hwnd)
    }

    struct Search {
        pid: u32,
        found: HWND,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam は focus_process が EnumWindows の間だけ渡す Search への可変参照である。
        let search = unsafe { &mut *(lparam as *mut Search) };
        if window_pid(hwnd) == Some(search.pid) && is_app_window(hwnd) {
            search.found = hwnd;
            return 0;
        }
        1
    }

    // タスクバーに並ぶ種類のウィンドウだけを選ぶ。持ち主のあるウィンドウはダイアログなどで、
    // ツールウィンドウは浮かんでいるパレットなどである。
    fn is_app_window(hwnd: HWND) -> bool {
        // SAFETY: どの関数も、ハンドルが無効なら 0 か NULL を返すだけである。
        unsafe {
            IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
                && (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW) == 0
        }
    }

    /// 記録したプロセスがまだ動いていれば、そのウィンドウのうち EnumWindows が最初に返すものを前面に出す。
    pub fn focus_process(p: &ProcessRef) -> bool {
        if process_created(p.pid) != Some(p.created) {
            return false;
        }
        let mut search = Search {
            pid: p.pid,
            found: ptr::null_mut(),
        };
        // SAFETY: search は EnumWindows が戻るまで生きており、visit はその間だけ参照する。
        unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
        !search.found.is_null() && bring_forward(search.found)
    }

    /// エディタの実行ファイルに作業フォルダを一つだけ渡して起動する。シェルは通さない。
    /// フォルダが絶対パスの既存のディレクトリでなければ何もしない。起動できたかを返す。
    pub fn open_folder_with(exe: &str, folder: &str) -> bool {
        let dir = Path::new(folder);
        if !dir.is_absolute() || !dir.is_dir() || !Path::new(exe).is_file() {
            return false;
        }
        // 起動したエディタは、既に動いている本体にフォルダを渡して終わることがある。その本体は
        // marimo の子ではないので、どのプロセスにも前面に出ることを許しておく
        // (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow)。
        // SAFETY: 引数は値だけである。
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        Command::new(exe).arg(folder).spawn().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn walk(parents: &[(u32, u32)], created: &[(u32, u64)]) -> Vec<u32> {
        let parents: HashMap<u32, u32> = parents.iter().copied().collect();
        let created: HashMap<u32, u64> = created.iter().copied().collect();
        walk_ancestors(
            10,
            Some(1_000),
            |pid| parents.get(&pid).copied(),
            |pid| {
                created
                    .get(&pid)
                    .map(|c| (*c, format!(r"C:\bin\p{pid}.exe")))
            },
        )
        .into_iter()
        .map(|p| p.pid)
        .collect()
    }

    #[test]
    fn walk_ancestors_stop_conditions() {
        let chain: Vec<(u32, u32)> = (10..30).map(|p| (p, p + 1)).collect();
        let chain_created: Vec<(u32, u64)> = (10..31).map(|p| (p, 1_000 - u64::from(p))).collect();
        let cases = vec![
            (
                "nearest first, stops at System",
                vec![(10, 20), (20, 30), (30, 4)],
                vec![(20, 900), (30, 800)],
                vec![20, 30],
            ),
            (
                "stops at System Idle Process",
                vec![(10, 20), (20, 0)],
                vec![(20, 900)],
                vec![20],
            ),
            ("no parent", vec![], vec![], vec![]),
            // 親の ID が、子より後に作られた別のプロセスに使い回されている。
            (
                "reused pid",
                vec![(10, 20), (20, 30)],
                vec![(20, 900), (30, 950)],
                vec![20],
            ),
            (
                "cycle",
                vec![(10, 20), (20, 30), (30, 20)],
                vec![(20, 900), (30, 800)],
                vec![20, 30],
            ),
            (
                "unreadable process",
                vec![(10, 20), (20, 30)],
                vec![(20, 900)],
                vec![20],
            ),
            ("at most eight", chain, chain_created, (11..19).collect()),
        ];
        for (label, parents, created, expected) in cases {
            assert_eq!(walk(&parents, &created), expected, "{label}");
        }
    }
}
