//! アプリの中から、最新の正式なリリースへ更新する。手順は tools/update.sh と tools/update.ps1 が
//! 引数なしで行うものを写しているので、片方の手順を変えるときはもう片方も揃える。
//!
//! 動いているアプリは自分自身を差し替えられないので、取ってきて照合し展開するところまでをアプリで行い、
//! 差し替えと起動し直しは書き出した補助のスクリプトに任せてアプリは終了する。

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use marimo_core::{MarimoHome, store};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

// GitHub Actions がビルドのときに入れるので、フォークの CI でビルドしたものはフォークのリリースを探す。
const REPO: &str = match option_env!("GITHUB_REPOSITORY") {
    Some(repo) => repo,
    None => "unknowns53/marimo",
};
const PROGRESS_EVENT: &str = "update-progress";
const LOG_FILE: &str = "update.log";
const MAC_ARCHIVE: &str = "marimo-macos-arm64.tar.gz";
const WINDOWS_ARCHIVE: &str = "marimo-windows-x64.zip";

static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInfo {
    current: String,
    latest: String,
    newer: bool,
    url: String,
}

#[derive(Clone, Serialize)]
struct Progress {
    stage: &'static str,
    message: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Release {
    tag: String,
    url: String,
}

#[tauri::command]
pub async fn check_update() -> Result<UpdateInfo, String> {
    let release = blocking(latest_release).await?;
    let current = env!("CARGO_PKG_VERSION");
    let latest = release.tag.strip_prefix('v').unwrap_or(&release.tag);
    Ok(UpdateInfo {
        current: current.to_string(),
        latest: latest.to_string(),
        newer: is_newer(latest, current),
        url: release.url,
    })
}

/// 補助のスクリプトを起動できたらアプリを終了する。それより前に失敗したときは、
/// 入っているアプリにも ~/.marimo にも触れずに Err を返す。
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if in_source_tree() {
        return Err("開発中の実行ファイルからは更新できません。".into());
    }
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("更新はすでに進めています。".into());
    }
    let handle = app.clone();
    match blocking(move || prepare(&handle)).await {
        Ok(()) => {
            app.exit(0);
            Ok(())
        }
        Err(e) => {
            INSTALLING.store(false, Ordering::SeqCst);
            Err(e)
        }
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("更新の処理が途中で止まりました（{e}）。"))?
}

/// main.rs の follow_autostart_path と同じ判定で、ソースツリーのビルド用フォルダから起動したかを返す。
fn in_source_tree() -> bool {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    match (std::env::current_exe(), source_root.canonicalize()) {
        (Ok(exe), Ok(root)) => exe.canonicalize().is_ok_and(|exe| exe.starts_with(root)),
        _ => false,
    }
}

enum Target {
    Mac { app_dest: PathBuf },
    Windows,
}

impl Target {
    fn current() -> Result<Self, String> {
        // CI がビルドするのは arm64 の Mac 向けと x64 の Windows 向けだけである。
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            let exe = std::env::current_exe()
                .and_then(|exe| exe.canonicalize())
                .map_err(|e| format!("実行ファイルの場所を調べられませんでした（{e}）。"))?;
            let app_dest = app_bundle_of(&exe)
                .ok_or("marimo.app の中から起動していないので、差し替える先が分かりません。")?;
            Ok(Self::Mac { app_dest })
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Ok(Self::Windows)
        } else {
            Err("リリースにはこの環境向けのビルドがありません。".into())
        }
    }

    fn archive(&self) -> &'static str {
        match self {
            Self::Mac { .. } => MAC_ARCHIVE,
            Self::Windows => WINDOWS_ARCHIVE,
        }
    }
}

/// 実行ファイルが X.app/Contents/MacOS/ の中にあれば X.app を返す。
fn app_bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let shaped = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    shaped.then(|| bundle.to_path_buf())
}

fn progress(app: &AppHandle, stage: &'static str, message: impl Into<String>) {
    let payload = Progress {
        stage,
        message: message.into(),
    };
    if let Err(e) = app.emit(PROGRESS_EVENT, payload) {
        eprintln!("marimo: cannot emit update progress: {e}");
    }
}

/// 失敗したときに一時フォルダを残さないよう、keep を呼ぶまでは落とすときに消す。
struct Work {
    path: PathBuf,
    keep: bool,
}

impl Work {
    fn create() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path =
            std::env::temp_dir().join(format!("marimo-update-{}-{nanos}", std::process::id()));
        fs::create_dir(&path).map_err(|e| format!("一時フォルダを作れませんでした（{e}）。"))?;
        Ok(Self { path, keep: false })
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn prepare(app: &AppHandle) -> Result<(), String> {
    let target = Target::current()?;
    let home = MarimoHome::resolve().ok_or("marimo のフォルダの場所を決められませんでした。")?;
    let log = home.root().join(LOG_FILE);
    // 補助のスクリプトの失敗はここにしか残らないので、書けないときは始める前に止める。
    let log_file = store::create_private_dir_all(home.root())
        .and_then(|()| OpenOptions::new().create(true).append(true).open(&log))
        .map_err(|e| format!("{} に書き込めません（{e}）。", log.display()))?;

    progress(app, "download", "最新の正式なリリースを調べています");
    let release = latest_release()?;
    let mut work = Work::create()?;
    let archive = target.archive();
    progress(
        app,
        "download",
        format!("正式なリリース {} を取ってきています", release.tag),
    );
    download(app, &work.path, &release.tag, archive)?;

    progress(app, "verify", "チェックサムを確かめています");
    verify(&work.path, archive)?;

    progress(app, "extract", "取ってきたものを展開しています");
    let extract = work.path.join("extract");
    fs::create_dir(&extract).map_err(|e| format!("展開先のフォルダを作れませんでした（{e}）。"))?;
    let pid = std::process::id();
    let work_str = path_str(&work.path)?;
    let log_str = path_str(&log)?;
    let helper = match &target {
        Target::Mac { app_dest } => {
            extract_mac(&work.path.join(archive), &extract)?;
            let script = mac_script(&MacHelper {
                work: work_str,
                app_dest: path_str(app_dest)?,
                pid,
                tag: &release.tag,
                bundle_id: &app.config().identifier,
            });
            Helper::Mac { script }
        }
        Target::Windows => {
            let (installer, hook) = extract_windows(&work.path.join(archive), &extract)?;
            let script = windows_script(&WindowsHelper {
                work: work_str,
                log: log_str,
                pid,
                tag: &release.tag,
                installer: path_str(&installer)?,
                hook: path_str(&hook)?,
            });
            Helper::Windows { script }
        }
    };

    progress(app, "restart", "marimo を終了して差し替えます");
    spawn_helper(&work.path, helper, log_file)?;
    work.keep = true;
    Ok(())
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("{} の場所を扱えませんでした。", path.display()))
}

fn latest_release() -> Result<Release, String> {
    let api = latest_release_api(REPO);
    let (code, body) = fetch_text(&api)?;
    match code {
        200 => parse_release(&body, REPO)
            .ok_or_else(|| "GitHub の応答から最新のリリースのバージョンを読み取れませんでした。".into()),
        404 => Err("GitHub に正式なリリースがまだありません。".into()),
        _ => Err(format!(
            "GitHub で最新のリリースを調べられませんでした（HTTP {code}）。"
        )),
    }
}

fn download(app: &AppHandle, work: &Path, tag: &str, archive: &str) -> Result<(), String> {
    let base = download_base(REPO, tag);
    // リリースを公開した直後の数十秒は、タグが見えていてもダウンロードが 404 を返すことがあるので、1 分は確かめ直す。
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let mut code = fetch_file(&format!("{base}/SHA256SUMS"), &work.join("SHA256SUMS"))?;
        if code == 200 {
            code = fetch_file(&format!("{base}/{archive}"), &work.join(archive))?;
        }
        match code {
            200 => return Ok(()),
            404 if Instant::now() < deadline => {
                progress(
                    app,
                    "download",
                    "ビルドがまだ取れないので、10 秒後に確かめ直します（最大 1 分）",
                );
                std::thread::sleep(Duration::from_secs(10));
            }
            404 => {
                return Err(format!(
                    "リリース {tag} にこの環境向けのファイルがそろっていません。"
                ));
            }
            _ => {
                return Err(format!(
                    "リリース {tag} のファイルを取得できませんでした（HTTP {code}）。"
                ));
            }
        }
    }
}

fn verify(work: &Path, archive: &str) -> Result<(), String> {
    let sums = fs::read_to_string(work.join("SHA256SUMS"))
        .map_err(|e| format!("SHA256SUMS を読めませんでした（{e}）。"))?;
    let expected = expected_sum(&sums, archive)
        .ok_or_else(|| format!("SHA256SUMS に {archive} の行がありません。"))?;
    let actual = sha256_file(&work.join(archive))
        .map_err(|e| format!("{archive} を読めませんでした（{e}）。"))?;
    if actual != expected {
        return Err(format!(
            "{archive} のチェックサムが SHA256SUMS と合いません。"
        ));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn extract_mac(archive: &Path, extract: &Path) -> Result<(), String> {
    let mut tar = Command::new("/usr/bin/tar");
    tar.arg("-xzf").arg(archive).arg("-C").arg(extract);
    run(&mut tar, "取ってきたものを展開できませんでした。")?;
    // curl で取ったものには quarantine が付かないが、付いていると初回の起動で Gatekeeper に止められるので、念のため外す。
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(extract)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !extract.join("marimo.app").is_dir() {
        return Err("取ってきたものの中にアプリが見つかりません。".into());
    }
    if !is_executable(&extract.join("marimo-hook")) {
        return Err("取ってきたものの中に marimo-hook が見つかりません。".into());
    }
    Ok(())
}

/// 展開したフォルダの中のインストーラと marimo-hook.exe を返す。
fn extract_windows(archive: &Path, extract: &Path) -> Result<(PathBuf, PathBuf), String> {
    // インターネットから来た印（Zone.Identifier）が付いていると、Windows が実行の前に確認を出して /S の実行が止まるので、念のため外す。
    let command = format!(
        "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; \
         Expand-Archive -LiteralPath {archive} -DestinationPath {extract}; \
         Get-ChildItem -LiteralPath {extract} -File | Unblock-File",
        archive = ps_quote(path_str(archive)?),
        extract = ps_quote(path_str(extract)?),
    );
    let mut powershell = Command::new(powershell_program());
    powershell
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
        ])
        .arg("-EncodedCommand")
        .arg(encode_powershell(&command));
    run(&mut powershell, "取ってきたものを展開できませんでした。")?;

    let hook = extract.join("marimo-hook.exe");
    if !hook.is_file() {
        return Err("取ってきたものの中に marimo-hook.exe が見つかりません。".into());
    }
    let installer = fs::read_dir(extract)
        .map_err(|e| format!("展開したフォルダを読めませんでした（{e}）。"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("marimo_") && name.ends_with("_x64-setup.exe"))
        })
        .ok_or("取ってきたものの中にインストーラが見つかりません。")?;
    Ok((installer, hook))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

enum Helper {
    Mac { script: String },
    Windows { script: String },
}

fn spawn_helper(work: &Path, helper: Helper, log: File) -> Result<(), String> {
    let mut command = match helper {
        Helper::Mac { script } => {
            let path = work.join("update.sh");
            fs::write(&path, script)
                .map_err(|e| format!("差し替え用のスクリプトを書き出せませんでした（{e}）。"))?;
            let stderr = log
                .try_clone()
                .map_err(|e| format!("{LOG_FILE} を開けませんでした（{e}）。"))?;
            let mut command = Command::new("/bin/bash");
            command.arg(path).stdout(log).stderr(stderr);
            command
        }
        Helper::Windows { script } => {
            let path = work.join("update.ps1");
            // Windows PowerShell 5.1 は BOM の無いスクリプトを ANSI のコード ページで読んで日本語を壊す。
            fs::write(&path, format!("\u{feff}{script}"))
                .map_err(|e| format!("差し替え用のスクリプトを書き出せませんでした（{e}）。"))?;
            // スクリプトが同じ update.log へ追記するので、ここで開いたままにすると共有の違反で書けなくなる。
            drop(log);
            let mut command = Command::new(powershell_program());
            command
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                ])
                .args(["-WindowStyle", "Hidden", "-File"])
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        }
    };
    // 補助のスクリプトは一時フォルダごと自分を消すので、その中を作業フォルダにしない。
    command
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null());
    detach(&mut command);
    command
        .spawn()
        .map(drop)
        .map_err(|e| format!("差し替え用のスクリプトを起動できませんでした（{e}）。"))
}

// LaunchAgent から起動したアプリが終わると、launchd は同じプロセス グループに残ったプロセスも止めるので、
// 補助のスクリプトは別のグループで動かす。
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

// リリースビルドのアプリはコンソールを持たないので、子のコンソール アプリを呼ぶたびに窓が開くのを止める。
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

/// 終了コードが 0 でなければ failure を返す。子の stderr は利用者に見せる文に向かないので、端末にだけ書く。
fn run(command: &mut Command, failure: &str) -> Result<Vec<u8>, String> {
    command.stdin(Stdio::null());
    hide_console(command);
    let output = command.output().map_err(|e| format!("{failure}（{e}）"))?;
    if !output.status.success() {
        eprintln!(
            "marimo: {:?} failed: {}",
            command.get_program(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Err(failure.to_string());
    }
    Ok(output.stdout)
}

// PATH の先に置かれた別のものを拾わないよう、OS に同梱されているものを場所で指す。
fn curl_program() -> PathBuf {
    if cfg!(windows) {
        system32(&["curl.exe"]).unwrap_or_else(|| "curl.exe".into())
    } else {
        "/usr/bin/curl".into()
    }
}

fn powershell_program() -> PathBuf {
    system32(&["WindowsPowerShell", "v1.0", "powershell.exe"])
        .unwrap_or_else(|| "powershell.exe".into())
}

fn system32(parts: &[&str]) -> Option<PathBuf> {
    let mut path = PathBuf::from(std::env::var_os("SystemRoot")?).join("System32");
    path.extend(parts);
    path.is_file().then_some(path)
}

const NETWORK_ERROR: &str =
    "GitHub に接続できませんでした。ネットワークにつながっているかを確かめてください。";

fn curl(url: &str) -> Command {
    let mut command = Command::new(curl_program());
    command
        .args(["-sSL", "--retry", "3", "--connect-timeout", "20"])
        .arg("--url")
        .arg(url);
    command
}

/// 本文と HTTP の状態コードを返す。
fn fetch_text(url: &str) -> Result<(u16, Vec<u8>), String> {
    let mut command = curl(url);
    command.args(["-w", "\n%{http_code}"]);
    let stdout = run(&mut command, NETWORK_ERROR)?;
    split_status(&stdout).ok_or_else(|| NETWORK_ERROR.into())
}

/// dest へ書き出し、HTTP の状態コードを返す。
fn fetch_file(url: &str, dest: &Path) -> Result<u16, String> {
    let mut command = curl(url);
    command.arg("-o").arg(dest).args(["-w", "%{http_code}"]);
    let stdout = run(&mut command, NETWORK_ERROR)?;
    String::from_utf8_lossy(&stdout)
        .trim()
        .parse()
        .map_err(|_| NETWORK_ERROR.into())
}

fn split_status(stdout: &[u8]) -> Option<(u16, Vec<u8>)> {
    let at = stdout.iter().rposition(|&b| b == b'\n')?;
    let code = std::str::from_utf8(&stdout[at + 1..])
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some((code, stdout[..at].to_vec()))
}

fn latest_release_api(repo: &str) -> String {
    // この API は下書きとプレリリースを除いた最新のリリースを返すので、CI が main のコミットごとに作る build- のプレリリースは選ばれない。
    format!("https://api.github.com/repos/{repo}/releases/latest")
}

fn download_base(repo: &str, tag: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}")
}

/// タグは URL とスクリプトに埋め込むので、版の名前に使う文字だけでできたものに限る。
fn parse_release(body: &[u8], repo: &str) -> Option<Release> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    let safe = !tag.is_empty()
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'));
    if !safe {
        return None;
    }
    let url = json.get("html_url").and_then(Value::as_str).map_or_else(
        || format!("https://github.com/{repo}/releases/tag/{tag}"),
        str::to_string,
    );
    Some(Release {
        tag: tag.to_string(),
        url,
    })
}

/// sha256sum の出力の、ファイル名の前にバイナリを示す * が付いた行も読む。
fn expected_sum(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name);
        (name == file).then(|| hash.to_ascii_lowercase())
    })
}

/// 版を数字の並びとして比べ、latest が current より新しいかを返す。数字が同じなら、
/// ハイフンの付いたプレリリースより付いていない正式な版を新しいとみなす。読めない版は新しくないとする。
fn is_newer(latest: &str, current: &str) -> bool {
    let (Some((latest, latest_pre)), Some((current, current_pre))) =
        (parse_version(latest), parse_version(current))
    else {
        return false;
    };
    let len = latest.len().max(current.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    let latest: Vec<u64> = (0..len).map(|i| at(&latest, i)).collect();
    let current: Vec<u64> = (0..len).map(|i| at(&current, i)).collect();
    latest > current || (latest == current && current_pre && !latest_pre)
}

fn parse_version(version: &str) -> Option<(Vec<u64>, bool)> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let (core, pre) = match version.find(['-', '+']) {
        Some(at) => (&version[..at], version[at..].starts_with('-')),
        None => (version, false),
    };
    let numbers = core
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    Some((numbers, pre))
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// PowerShell は ' のほかに ‘ ’ ‚ ‛ も単一引用符として扱うので、どれも重ねて閉じないようにする。
fn ps_quote(s: &str) -> String {
    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('\'');
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('\'');
    quoted
}

/// -EncodedCommand は UTF-16LE を Base64 にしたものを受け取る。
fn encode_powershell(command: &str) -> String {
    let bytes: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

struct MacHelper<'a> {
    work: &'a str,
    app_dest: &'a str,
    pid: u32,
    tag: &'a str,
    bundle_id: &'a str,
}

fn mac_script(h: &MacHelper) -> String {
    format!(
        "#!/bin/bash\nset -u\n\nwork={}\napp_dest={}\napp_pid={}\ntag={}\nbundle_id={}\n{MAC_BODY}",
        sh_quote(h.work),
        sh_quote(h.app_dest),
        h.pid,
        sh_quote(h.tag),
        sh_quote(h.bundle_id),
    )
}

struct WindowsHelper<'a> {
    work: &'a str,
    log: &'a str,
    pid: u32,
    tag: &'a str,
    installer: &'a str,
    hook: &'a str,
}

fn windows_script(h: &WindowsHelper) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\r\n\r\n$work = {}\r\n$log = {}\r\n$appPid = {}\r\n$tag = {}\r\n$installer = {}\r\n$hook = {}\r\n{}",
        ps_quote(h.work),
        ps_quote(h.log),
        h.pid,
        ps_quote(h.tag),
        ps_quote(h.installer),
        ps_quote(h.hook),
        WINDOWS_BODY.replace('\n', "\r\n"),
    )
}

// 標準出力と標準エラーはアプリが update.log へつないで起動する。
const MAC_BODY: &str = r#"
bundle="$work/extract/marimo.app"
hook="$work/extract/marimo-hook"
app_new="$(dirname "$app_dest")/.$(basename "$app_dest").new"
app_old="$(dirname "$app_dest")/.$(basename "$app_dest").old"
running_pattern="$app_dest/Contents/MacOS/"
trap 'rm -rf "$work"' EXIT

say() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

fail() {
  say "エラー: $*"
  exit 1
}

say "正式なリリース $tag へ更新します（$app_dest）"

# アプリはこのスクリプトを起動した直後に終了するので、まずそれを待つ。
for _ in $(seq 1 60); do
  kill -0 "$app_pid" 2>/dev/null || break
  sleep 0.5
done

if pgrep -f "$running_pattern" >/dev/null; then
  say "起動中の marimo を終了しています"
  osascript -e "tell application id \"$bundle_id\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    pgrep -f "$running_pattern" >/dev/null || break
    sleep 0.5
  done
  if pgrep -f "$running_pattern" >/dev/null; then
    fail "marimo が終了しません。右クリックメニューの「終了」で終了してから、もう一度更新してください。アプリは差し替えていません。"
  fi
fi

# ここから先で止まるときは、アプリが終了したままにならないよう、残っている方のアプリを起動し直す。
say "$app_dest を差し替えています"
rm -rf "$app_new" "$app_old"
if ! ditto "$bundle" "$app_new"; then
  rm -rf "$app_new"
  open "$app_dest"
  fail "新しいアプリを $app_new へコピーできませんでした。アプリは差し替えずに起動し直しています。"
fi
if [[ -e "$app_dest" ]] && ! mv "$app_dest" "$app_old"; then
  rm -rf "$app_new"
  open "$app_dest"
  fail "$app_dest を退けられませんでした。アプリは差し替えずに起動し直しています。"
fi
if ! mv "$app_new" "$app_dest"; then
  [[ -e "$app_old" ]] && mv "$app_old" "$app_dest"
  open "$app_dest"
  fail "新しいアプリを $app_dest へ置けませんでした。元のアプリに戻して起動し直しています。"
fi
rm -rf "$app_old"

say "marimo-hook install を実行しています"
install_ok=1
"$hook" install || install_ok=0

say "marimo を起動しています"
open "$app_dest"

if [[ $install_ok -eq 0 ]]; then
  # 一時フォルダは終わると消えるので、install をやり直すときは更新ごとやり直してもらう。
  fail "marimo-hook install が失敗しました。アプリはすでに新しいものに差し替えて起動しています。上のメッセージを確かめてから、tools/update.sh を実行してください。"
fi
say "更新が終わりました"
"#;

// 標準出力はどこにもつながっていないので、残すものは Write-Log で update.log へ書く。
const WINDOWS_BODY: &str = r#"
$installDir = Join-Path $env:LOCALAPPDATA 'marimo'
$installedExe = Join-Path $installDir 'marimo.exe'
$utf8 = New-Object System.Text.UTF8Encoding $false

function Write-Log([string]$Message) {
    $line = '[' + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss') + '] ' + $Message + [Environment]::NewLine
    [IO.File]::AppendAllText($log, $line, $utf8)
}

function Fail([string]$Message) {
    Write-Log "エラー: $Message"
    exit 1
}

function Get-InstalledMarimo([string]$Dir) {
    $prefix = $Dir.TrimEnd('\') + '\'
    @(Get-Process -Name marimo -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    })
}

try {
    Write-Log "正式なリリース $tag へ更新します"

    # アプリはこのスクリプトを起動した直後に終了するので、まずそれを待つ。
    $app = Get-Process -Id $appPid -ErrorAction SilentlyContinue
    if ($app) { [void]$app.WaitForExit(30000) }

    $running = @(Get-InstalledMarimo $installDir)
    if ($running.Count -gt 0) {
        Write-Log '起動中の marimo を終了しています'
        foreach ($p in $running) {
            if ($p.HasExited) { continue }
            try {
                $p.Kill()
            } catch {
                Write-Log "marimo（PID $($p.Id)）を終了できませんでした: $($_.Exception.Message)"
            }
        }
        # 終了させたプロセスは、一覧を取り直すと片付く前の姿がまだ見えることがあるので、同じオブジェクトの終了を待つ。
        # それでも残るときは、動いているアプリを自分で閉じるインストーラに任せて先へ進む。
        $left = @($running | Where-Object { -not $_.WaitForExit(10000) })
        if ($left.Count -gt 0) {
            Write-Log 'marimo がまだ終了していないので、インストーラに終了を任せます'
        }
    }

    Write-Log "$(Split-Path -Leaf $installer) でアプリを入れ直しています"
    $setup = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
    if ($setup.ExitCode -ne 0) {
        # アプリが終了したままにならないよう、残っている方のアプリを起動し直す。
        if (Test-Path -LiteralPath $installedExe) { Start-Process -FilePath $installedExe }
        Fail "インストーラが失敗しました（終了コード $($setup.ExitCode)）。marimo を終了できなかった場合に起こるので、立ち絵を右クリックして「終了」を選んでから、tools\update.ps1 を実行してください。"
    }
    if (-not (Test-Path -LiteralPath $installedExe)) {
        Fail "インストーラは成功を返しましたが、$installedExe が見つかりません。"
    }

    Write-Log 'marimo-hook install を実行しています'
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & $hook install 2>&1 | Out-String
    } finally {
        $ErrorActionPreference = $saved
    }
    $installOk = ($LASTEXITCODE -eq 0)
    if ($output.Trim()) { Write-Log $output.TrimEnd() }

    Write-Log 'marimo を起動しています'
    Start-Process -FilePath $installedExe

    if (-not $installOk) {
        # 一時フォルダは終わると消えるので、install をやり直すときは更新ごとやり直してもらう。
        Fail "marimo-hook install が失敗しました。アプリはすでに新しいものに入れ直して起動しています。上のメッセージを確かめてから、tools\update.ps1 を実行してください。"
    }
    Write-Log '更新が終わりました'
} catch {
    Write-Log "エラー: $($_.Exception.Message)"
    exit 1
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_as_numbers() {
        for (latest, current, newer) in [
            ("0.2.0", "0.1.0", true),
            ("0.10.0", "0.9.9", true),
            ("1.0.0", "0.99.99", true),
            ("0.1.1", "0.1.0", true),
            ("v0.2.0", "0.1.0", true),
            ("0.1.0", "0.1.0", false),
            ("0.1.0", "0.2.0", false),
            ("0.1", "0.1.0", false),
            ("0.1.0.1", "0.1.0", true),
            ("0.2.0", "0.2.0-rc.1", true),
            ("0.2.0-rc.1", "0.2.0", false),
            ("0.2.0+build.5", "0.2.0", false),
            ("", "0.1.0", false),
            ("latest", "0.1.0", false),
            ("0..1", "0.1.0", false),
            ("0.-1.0", "0.1.0", false),
            ("+1.0.0", "0.1.0", false),
            ("99999999999999999999999.0.0", "0.1.0", false),
            ("0.2.0", "garbage", false),
        ] {
            assert_eq!(
                is_newer(latest, current),
                newer,
                "latest {latest:?}, current {current:?}"
            );
        }
    }

    #[test]
    fn reads_release_from_api_response() {
        let repo = "owner/repo";
        for (body, want) in [
            (
                r#"{"tag_name": "v0.2.0", "html_url": "https://github.com/owner/repo/releases/tag/v0.2.0"}"#,
                Some((
                    "v0.2.0",
                    "https://github.com/owner/repo/releases/tag/v0.2.0",
                )),
            ),
            (
                r#"{"tag_name": "v0.3.0"}"#,
                Some((
                    "v0.3.0",
                    "https://github.com/owner/repo/releases/tag/v0.3.0",
                )),
            ),
            (r#"{"tag_name": ""}"#, None),
            (r#"{"tag_name": "v1'; rm -rf ~"}"#, None),
            (r#"{"tag_name": 3}"#, None),
            (r#"{"message": "Not Found"}"#, None),
            ("not json", None),
        ] {
            let want = want.map(|(tag, url)| Release {
                tag: tag.into(),
                url: url.into(),
            });
            assert_eq!(parse_release(body.as_bytes(), repo), want, "body {body:?}");
        }
        assert_eq!(
            latest_release_api(repo),
            "https://api.github.com/repos/owner/repo/releases/latest"
        );
        assert_eq!(
            download_base(repo, "v0.2.0"),
            "https://github.com/owner/repo/releases/download/v0.2.0"
        );
    }

    #[test]
    fn finds_archive_line_in_sha256sums() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        for (sums, want) in [
            (
                format!("{a}  {MAC_ARCHIVE}\n{b}  {WINDOWS_ARCHIVE}\n"),
                Some(a.clone()),
            ),
            (
                format!("{b}  {WINDOWS_ARCHIVE}\r\n{a}  {MAC_ARCHIVE}\r\n"),
                Some(a.clone()),
            ),
            (
                format!("{b} *{MAC_ARCHIVE}\n"),
                Some(b.to_ascii_lowercase()),
            ),
            (format!("{a}  {MAC_ARCHIVE}.old\n"), None),
            (format!("{a}  {WINDOWS_ARCHIVE}\n"), None),
            (format!("{a}\n\n"), None),
            (String::new(), None),
        ] {
            assert_eq!(expected_sum(&sums, MAC_ARCHIVE), want, "sums {sums:?}");
        }
    }

    #[test]
    fn splits_body_from_status_code() {
        for (stdout, want) in [
            (&b"{\"a\":1}\n200"[..], Some((200, &b"{\"a\":1}"[..]))),
            (&b"line\nline\n404"[..], Some((404, &b"line\nline"[..]))),
            (&b"\n000"[..], Some((0, &b""[..]))),
            (&b"200"[..], None),
            (&b"body\n"[..], None),
        ] {
            assert_eq!(
                split_status(stdout),
                want.map(|(code, body)| (code, body.to_vec())),
                "stdout {stdout:?}"
            );
        }
    }

    #[test]
    fn finds_app_bundle_of_executable() {
        for (exe, want) in [
            (
                "/Applications/marimo.app/Contents/MacOS/marimo",
                Some("/Applications/marimo.app"),
            ),
            (
                "/Users/a/Apps/marimo copy.app/Contents/MacOS/marimo",
                Some("/Users/a/Apps/marimo copy.app"),
            ),
            ("/Users/a/marimo/target/release/marimo", None),
            ("/Applications/marimo.app/Contents/Resources/marimo", None),
            ("/marimo", None),
        ] {
            assert_eq!(
                app_bundle_of(Path::new(exe)),
                want.map(PathBuf::from),
                "exe {exe:?}"
            );
        }
    }

    #[test]
    fn quotes_values_for_scripts() {
        for (raw, sh, ps) in [
            ("/tmp/a b", "'/tmp/a b'", "'/tmp/a b'"),
            ("it's", r"'it'\''s'", "'it''s'"),
            ("$HOME `x` \"y\"", "'$HOME `x` \"y\"'", "'$HOME `x` \"y\"'"),
            ("鷹\u{2019}s", "'鷹\u{2019}s'", "'鷹\u{2019}\u{2019}s'"),
        ] {
            assert_eq!(
                (sh_quote(raw), ps_quote(raw)),
                (sh.into(), ps.into()),
                "raw {raw:?}"
            );
        }
        assert_eq!(encode_powershell("ab"), "YQBiAA==");
    }

    #[test]
    fn helper_scripts_embed_quoted_values() {
        let mac = mac_script(&MacHelper {
            work: "/tmp/marimo-update-1",
            app_dest: "/Users/a/My Apps/it's.app",
            pid: 42,
            tag: "v0.2.0",
            bundle_id: "com.marimo.desktop",
        });
        assert!(mac.starts_with("#!/bin/bash\n"));
        assert!(mac.contains("\napp_dest='/Users/a/My Apps/it'\\''s.app'\n"));
        assert!(mac.contains("\napp_pid=42\n"));

        let windows = windows_script(&WindowsHelper {
            work: r"C:\Users\a\AppData\Local\Temp\marimo-update-1",
            log: r"C:\Users\o'neil\.marimo\update.log",
            pid: 42,
            tag: "v0.2.0",
            installer: r"C:\x\marimo_0.2.0_x64-setup.exe",
            hook: r"C:\x\marimo-hook.exe",
        });
        assert!(windows.contains("\r\n$log = 'C:\\Users\\o''neil\\.marimo\\update.log'\r\n"));
        assert!(windows.contains("\r\n$appPid = 42\r\n"));
        assert!(!windows.replace("\r\n", "").contains('\n'));
    }

    #[cfg(unix)]
    #[test]
    fn mac_script_parses_as_bash() {
        let script = mac_script(&MacHelper {
            work: "/tmp/w",
            app_dest: "/Applications/marimo.app",
            pid: 1,
            tag: "v0.2.0",
            bundle_id: "com.marimo.desktop",
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.sh");
        fs::write(&path, script).unwrap();
        let status = Command::new("/bin/bash")
            .arg("-n")
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
    }
}
