//! アプリの中から、最新の正式なリリースへ更新する。手順は tools/update.sh と tools/update.ps1 が
//! 引数なしで行うものと同じ手順にしているので、片方の手順を変えるときはもう片方も揃える。
//!
//! 動いているアプリは自分自身を差し替えられないので、取得して照合し展開するところまでをアプリで行い、
//! 差し替えと起動し直しは書き出した補助のスクリプトに任せてアプリは終了する。

use std::cmp::Ordering;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{self, AtomicBool};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use marimo_core::{Lang, MarimoHome, store};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

// GitHub Actions がビルドのときに入れるので、フォークの CI でビルドしたものはフォークのリリースを探す。
const REPO: &str = match option_env!("GITHUB_REPOSITORY") {
    Some(repo) => repo,
    None => "unknowns53/marimo",
};
const PROGRESS_EVENT: &str = "update-progress";
/// 確認するたびに、UpdateStatus の全体を送る。
const STATUS_EVENT: &str = "update-status";
const LOG_FILE: &str = "update.log";
const MAC_ARCHIVE: &str = "marimo-macos-arm64.tar.gz";
const WINDOWS_ARCHIVE: &str = "marimo-windows-x64.zip";
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(3);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

static INSTALLING: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<Option<Checked>> = Mutex::new(None);

/// 最後の確認の結果。設定のウィンドウはこれを出し、トレイのメニューは latest が新しいときだけ項目を出す。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateStatus {
    current: String,
    /// まだ確認していないときと、確認に失敗したときは None。
    latest: Option<String>,
    newer: bool,
    /// 確認した時刻（UNIX 時刻のミリ秒）。まだ確認していないときは None。
    checked_at: Option<u64>,
    error: Option<String>,
    /// 最新リリースのページ URL。確認済みのリリースがあるときだけ入れる。
    release_url: Option<String>,
}

#[derive(Debug, Clone)]
struct Checked {
    result: Result<String, String>,
    at: u64,
}

#[derive(Clone, Serialize)]
struct Progress {
    stage: &'static str,
    message: String,
}

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn release_page(repo: &str, version: &str) -> String {
    format!("https://github.com/{repo}/releases/tag/v{version}")
}

fn status_of(checked: Option<&Checked>) -> UpdateStatus {
    let current = current_version();
    let latest = checked.and_then(|c| c.result.as_ref().ok()).cloned();
    let release_url = latest.as_deref().map(|v| release_page(REPO, v));
    UpdateStatus {
        current: current.to_string(),
        newer: latest.as_deref().is_some_and(|l| is_newer(l, current)),
        latest,
        checked_at: checked.map(|c| c.at),
        error: checked.and_then(|c| c.result.as_ref().err()).cloned(),
        release_url,
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// GitHub に問い合わせて結果を覚え、トレイのメニューと設定のウィンドウへ知らせる。
fn check_now(app: &AppHandle) -> UpdateStatus {
    let result = latest_release(crate::lang(app))
        .map(|tag| tag.strip_prefix('v').unwrap_or(&tag).to_string());
    let checked = Checked {
        result,
        at: now_millis(),
    };
    let status = status_of(Some(&checked));
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(checked);
    if let Some(state) = app.try_state::<AppState>()
        && let Some(tray) = state.tray.get()
    {
        tray.set_update(status.latest.as_deref().filter(|_| status.newer));
    }
    if let Err(e) = app.emit(STATUS_EVENT, &status) {
        eprintln!("marimo: cannot emit update status: {e}");
    }
    status
}

/// 起動の少しあとと、そのあと一日ごとに、新しいバージョンがあるかを確認する。
pub fn spawn_checker(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                check_now(&app);
                std::thread::sleep(CHECK_INTERVAL);
            }
        });
    if let Err(e) = spawned {
        eprintln!("marimo: cannot start the update check: {e}");
    }
}

/// 設定のウィンドウを開いたときに、最後の確認の結果を出すために使う。
#[tauri::command]
pub fn update_status() -> UpdateStatus {
    status_of(STATUS.lock().unwrap_or_else(|e| e.into_inner()).as_ref())
}

/// 確認済みの最新リリースの変更内容ページを既定のブラウザで開く。
#[tauri::command]
pub fn open_release_notes() {
    let url = STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|c| c.result.as_ref().ok())
        .map_or_else(
            || format!("https://github.com/{REPO}/releases"),
            |v| release_page(REPO, v),
        );
    crate::focus::open_external_url(&url);
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateStatus, String> {
    let lang = crate::lang(&app);
    blocking(lang, move || Ok(check_now(&app))).await
}

/// 補助のスクリプトを起動できたらアプリを終了する。それより前に失敗したときは、
/// 入っているアプリにも ~/.marimo にも触れずに Err を返す。
/// version は確認のダイアログに出したバージョンで、その後に別のリリースが出ていたら入れずに Err を返す。
#[tauri::command]
pub async fn install_update(app: AppHandle, version: String) -> Result<(), String> {
    let lang = crate::lang(&app);
    if in_source_tree() {
        return Err(lang
            .pick(
                "開発中の実行ファイルからは更新できません。",
                "A development build cannot update itself.",
            )
            .into());
    }
    if INSTALLING.swap(true, atomic::Ordering::SeqCst) {
        return Err(lang
            .pick(
                "更新はすでに実行中です。",
                "An update is already in progress.",
            )
            .into());
    }
    let handle = app.clone();
    match blocking(lang, move || prepare(&handle, lang, &version)).await {
        Ok(()) => {
            app.exit(0);
            Ok(())
        }
        Err(e) => {
            INSTALLING.store(false, atomic::Ordering::SeqCst);
            Err(e)
        }
    }
}

async fn blocking<T: Send + 'static>(
    lang: Lang,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        lang.pick(
            format!("更新の処理が途中で止まりました（{e}）。"),
            format!("The update stopped unexpectedly ({e})."),
        )
    })?
}

/// ソースツリーのビルド用フォルダから起動したかを返す。
pub(crate) fn in_source_tree() -> bool {
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
    fn current(lang: Lang) -> Result<Self, String> {
        // CI がビルドするのは arm64 の Mac 向けと x64 の Windows 向けだけである。
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            let exe = std::env::current_exe()
                .and_then(|exe| exe.canonicalize())
                .map_err(|e| {
                    lang.pick(
                        format!("実行ファイルの場所を調べられませんでした（{e}）。"),
                        format!("Could not find the location of the executable ({e})."),
                    )
                })?;
            let app_dest = app_bundle_of(&exe).ok_or(lang.pick(
                "marimo.app の中から起動していないので、差し替える先が分かりません。",
                "marimo was not launched from marimo.app, so there is no app to replace.",
            ))?;
            Ok(Self::Mac { app_dest })
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Ok(Self::Windows)
        } else {
            Err(lang
                .pick(
                    "リリースにはこの環境向けのビルドがありません。",
                    "Releases do not include a build for this platform.",
                )
                .into())
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
    fn create(lang: Lang) -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path =
            std::env::temp_dir().join(format!("marimo-update-{}-{nanos}", std::process::id()));
        fs::create_dir(&path).map_err(|e| {
            lang.pick(
                format!("一時フォルダを作れませんでした（{e}）。"),
                format!("Could not create a temporary folder ({e})."),
            )
        })?;
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

fn prepare(app: &AppHandle, lang: Lang, version: &str) -> Result<(), String> {
    let target = Target::current(lang)?;
    let home = MarimoHome::resolve().ok_or(lang.pick(
        "marimo のフォルダの場所を特定できませんでした。",
        "Could not find the marimo folder.",
    ))?;
    let log = home.root().join(LOG_FILE);
    // 補助のスクリプトの失敗はここにしか残らないので、書けないときは始める前に止める。
    let log_file = store::create_private_dir_all(home.root())
        .and_then(|()| OpenOptions::new().create(true).append(true).open(&log))
        .map_err(|e| {
            lang.pick(
                format!("{} に書き込めません（{e}）。", log.display()),
                format!("Cannot write to {} ({e}).", log.display()),
            )
        })?;

    progress(
        app,
        "download",
        lang.pick(
            "最新の正式なリリースを調べています",
            "Checking for the latest stable release",
        ),
    );
    let tag = latest_release(lang)?;
    if tag.strip_prefix('v').unwrap_or(&tag) != version {
        return Err(lang.pick(
            format!("最新の正式なリリースが {tag} に変わりました。もう一度確認してください。"),
            format!("The latest stable release has changed to {tag}. Check for updates again."),
        ));
    }
    let mut work = Work::create(lang)?;
    let archive = target.archive();
    progress(
        app,
        "download",
        lang.pick(
            format!("正式なリリース {tag} をダウンロードしています"),
            format!("Downloading stable release {tag}"),
        ),
    );
    download(app, lang, &work.path, &tag, archive)?;

    progress(
        app,
        "verify",
        lang.pick("チェックサムを検証しています", "Verifying the checksum"),
    );
    verify(&work.path, archive, lang)?;

    progress(
        app,
        "extract",
        lang.pick(
            "ダウンロードしたファイルを展開しています",
            "Extracting the download",
        ),
    );
    let extract = work.path.join("extract");
    fs::create_dir(&extract).map_err(|e| {
        lang.pick(
            format!("展開先のフォルダを作れませんでした（{e}）。"),
            format!("Could not create the folder to extract into ({e})."),
        )
    })?;
    let pid = std::process::id();
    let work_str = path_str(&work.path, lang)?;
    let log_str = path_str(&log, lang)?;
    let helper = match &target {
        Target::Mac { app_dest } => {
            extract_mac(&work.path.join(archive), &extract, lang)?;
            let script = mac_script(&MacHelper {
                work: work_str,
                app_dest: path_str(app_dest, lang)?,
                pid,
                tag: &tag,
                bundle_id: &app.config().identifier,
                lang,
            });
            Helper::Mac { script }
        }
        Target::Windows => {
            let (installer, hook) = extract_windows(&work.path.join(archive), &extract, lang)?;
            let script = windows_script(&WindowsHelper {
                work: work_str,
                log: log_str,
                pid,
                tag: &tag,
                installer: path_str(&installer, lang)?,
                hook: path_str(&hook, lang)?,
                lang,
            });
            Helper::Windows { script }
        }
    };

    progress(
        app,
        "restart",
        lang.pick(
            "marimo を終了して差し替えます",
            "Quitting marimo to replace it",
        ),
    );
    spawn_helper(&work.path, helper, log_file, lang)?;
    work.keep = true;
    Ok(())
}

fn path_str(path: &Path, lang: Lang) -> Result<&str, String> {
    path.to_str().ok_or_else(|| {
        lang.pick(
            format!("{} の場所を扱えませんでした。", path.display()),
            format!("Cannot handle the path {}.", path.display()),
        )
    })
}

/// プレリリースを入れるのは tools/update.sh の --pre だけにし、アプリからの更新はどのバージョンからでも正式なバージョンだけを
/// 更新先にする。プレリリースを入れたアプリも、それより新しい正式なバージョンが出ればそちらへ移る。
fn latest_release(lang: Lang) -> Result<String, String> {
    let (code, body) = fetch_text(&releases_api(REPO), lang)?;
    match code {
        200 => newest_release(&body).ok_or_else(|| {
            lang.pick(
                "GitHub に更新先のリリースがまだありません。",
                "There is no stable release on GitHub to update to yet.",
            )
            .into()
        }),
        _ => Err(lang.pick(
            format!("GitHub で最新のリリースを調べられませんでした（HTTP {code}）。"),
            format!("Could not check the latest release on GitHub (HTTP {code})."),
        )),
    }
}

fn download(
    app: &AppHandle,
    lang: Lang,
    work: &Path,
    tag: &str,
    archive: &str,
) -> Result<(), String> {
    let base = download_base(REPO, tag);
    // リリースを公開した直後の数十秒は、タグが見えていてもダウンロードが 404 を返すことがあるので、1 分は確認し直す。
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let mut code = fetch_file(
            &format!("{base}/SHA256SUMS"),
            &work.join("SHA256SUMS"),
            lang,
        )?;
        if code == 200 {
            code = fetch_file(&format!("{base}/{archive}"), &work.join(archive), lang)?;
        }
        match code {
            200 => return Ok(()),
            404 if Instant::now() < deadline => {
                progress(
                    app,
                    "download",
                    lang.pick(
                        "ビルドがまだ公開されていないので、10 秒後にもう一度確認します（最大 1 分）",
                        "The build is not published yet; checking again in 10 seconds (up to 1 min)",
                    ),
                );
                std::thread::sleep(Duration::from_secs(10));
            }
            404 => {
                return Err(lang.pick(
                    format!("リリース {tag} にこの環境向けのファイルがそろっていません。"),
                    format!("Release {tag} is missing the files for this platform."),
                ));
            }
            _ => {
                return Err(lang.pick(
                    format!("リリース {tag} のファイルを取得できませんでした（HTTP {code}）。"),
                    format!("Could not download the files of release {tag} (HTTP {code})."),
                ));
            }
        }
    }
}

fn verify(work: &Path, archive: &str, lang: Lang) -> Result<(), String> {
    let sums = fs::read_to_string(work.join("SHA256SUMS")).map_err(|e| {
        lang.pick(
            format!("SHA256SUMS を読めませんでした（{e}）。"),
            format!("Could not read SHA256SUMS ({e})."),
        )
    })?;
    let expected = expected_sum(&sums, archive).ok_or_else(|| {
        lang.pick(
            format!("SHA256SUMS に {archive} の行がありません。"),
            format!("SHA256SUMS has no line for {archive}."),
        )
    })?;
    let actual = sha256_file(&work.join(archive)).map_err(|e| {
        lang.pick(
            format!("{archive} を読めませんでした（{e}）。"),
            format!("Could not read {archive} ({e})."),
        )
    })?;
    if actual != expected {
        return Err(lang.pick(
            format!("{archive} のチェックサムが SHA256SUMS と合いません。"),
            format!("The checksum of {archive} does not match SHA256SUMS."),
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

fn extract_mac(archive: &Path, extract: &Path, lang: Lang) -> Result<(), String> {
    let mut tar = Command::new("/usr/bin/tar");
    tar.arg("-xzf").arg(archive).arg("-C").arg(extract);
    run(&mut tar, extract_failed(lang), lang)?;
    // curl で取ったものには quarantine が付かないが、付いていると初回の起動で Gatekeeper に止められるので、念のため外す。
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(extract)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !extract.join("marimo.app").is_dir() {
        return Err(lang
            .pick(
                "ダウンロードしたファイルの中にアプリが見つかりません。",
                "The download does not contain the app.",
            )
            .into());
    }
    if !is_executable(&extract.join("marimo-hook")) {
        return Err(lang
            .pick(
                "ダウンロードしたファイルの中に marimo-hook が見つかりません。",
                "The download does not contain marimo-hook.",
            )
            .into());
    }
    Ok(())
}

/// 展開したフォルダの中のインストーラと marimo-hook.exe を返す。
fn extract_windows(
    archive: &Path,
    extract: &Path,
    lang: Lang,
) -> Result<(PathBuf, PathBuf), String> {
    // インターネットから来たことを示す Mark of the Web（Zone.Identifier）が付いていると、Windows が実行の前に確認を出して /S の実行が止まるので、念のため外す。
    let command = format!(
        "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; \
         Expand-Archive -LiteralPath {archive} -DestinationPath {extract}; \
         Get-ChildItem -LiteralPath {extract} -File | Unblock-File",
        archive = ps_quote(path_str(archive, lang)?),
        extract = ps_quote(path_str(extract, lang)?),
    );
    let mut powershell = windows_powershell();
    powershell
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
        ])
        .arg("-EncodedCommand")
        .arg(encode_powershell(&command));
    run(&mut powershell, extract_failed(lang), lang)?;

    let hook = extract.join("marimo-hook.exe");
    if !hook.is_file() {
        return Err(lang
            .pick(
                "ダウンロードしたファイルの中に marimo-hook.exe が見つかりません。",
                "The download does not contain marimo-hook.exe.",
            )
            .into());
    }
    let installer = fs::read_dir(extract)
        .map_err(|e| {
            lang.pick(
                format!("展開したフォルダを読めませんでした（{e}）。"),
                format!("Could not read the extracted folder ({e})."),
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("marimo_") && name.ends_with("_x64-setup.exe"))
        })
        .ok_or(lang.pick(
            "ダウンロードしたファイルの中にインストーラが見つかりません。",
            "The download does not contain the installer.",
        ))?;
    Ok((installer, hook))
}

fn extract_failed(lang: Lang) -> &'static str {
    lang.pick(
        "ダウンロードしたファイルを展開できませんでした。",
        "Could not extract the download.",
    )
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

fn spawn_helper(work: &Path, helper: Helper, log: File, lang: Lang) -> Result<(), String> {
    let write_failed = |e: io::Error| {
        lang.pick(
            format!("差し替え用のスクリプトを書き出せませんでした（{e}）。"),
            format!("Could not write the update script ({e})."),
        )
    };
    let mut command = match helper {
        Helper::Mac { script } => {
            let path = work.join("update.sh");
            fs::write(&path, script).map_err(write_failed)?;
            let stderr = log.try_clone().map_err(|e| {
                lang.pick(
                    format!("{LOG_FILE} を開けませんでした（{e}）。"),
                    format!("Could not open {LOG_FILE} ({e})."),
                )
            })?;
            let mut command = Command::new("/bin/bash");
            command.arg(path).stdout(log).stderr(stderr);
            command
        }
        Helper::Windows { script } => {
            let path = work.join("update.ps1");
            // Windows PowerShell 5.1 は BOM の無いスクリプトを ANSI のコード ページで読んで日本語を壊す。
            fs::write(&path, format!("\u{feff}{script}")).map_err(write_failed)?;
            // スクリプトが同じ update.log へ追記するので、ここで開いたままにすると共有の違反で書けなくなる。
            drop(log);
            let mut command = windows_powershell();
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
    command.spawn().map(drop).map_err(|e| {
        lang.pick(
            format!("差し替え用のスクリプトを起動できませんでした（{e}）。"),
            format!("Could not start the update script ({e})."),
        )
    })
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

// リリースビルドのアプリはコンソールを持たないので、コンソール アプリを子プロセスとして呼ぶたびにウィンドウが開くのを止める。
#[cfg(windows)]
pub(crate) fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(crate) fn hide_console(_: &mut Command) {}

/// 終了コードが 0 でなければ failure を返す。子プロセスの stderr は利用者に見せる文に向かないので、端末にだけ書く。
fn run(command: &mut Command, failure: &str, lang: Lang) -> Result<Vec<u8>, String> {
    command.stdin(Stdio::null());
    hide_console(command);
    let output = command
        .output()
        .map_err(|e| lang.pick(format!("{failure}（{e}）"), format!("{failure} ({e})")))?;
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

/// Windows PowerShell 5.1 を起動するコマンドを返す。PowerShell 7 から起動された marimo は、PowerShell 7 の
/// モジュールの場所を並べた PSModulePath を受け継いでいる。それを 5.1 に渡すと、5.1 が PowerShell 7 用の
/// Microsoft.PowerShell.Archive などを読み込み、Expand-Archive が失敗する。外して渡せば、5.1 が既定の場所で組み立て直す。
pub(crate) fn windows_powershell() -> Command {
    let program = system32(&["WindowsPowerShell", "v1.0", "powershell.exe"])
        .unwrap_or_else(|| "powershell.exe".into());
    let mut command = Command::new(program);
    command.env_remove("PSModulePath");
    command
}

fn system32(parts: &[&str]) -> Option<PathBuf> {
    let mut path = PathBuf::from(std::env::var_os("SystemRoot")?).join("System32");
    path.extend(parts);
    path.is_file().then_some(path)
}

fn network_error(lang: Lang) -> &'static str {
    lang.pick(
        "GitHub に接続できませんでした。ネットワークが有効であることを確認してください。",
        "Could not connect to GitHub. Check your network connection.",
    )
}

fn curl(url: &str) -> Command {
    let mut command = Command::new(curl_program());
    command
        .args(["-sSL", "--retry", "3", "--connect-timeout", "20"])
        .arg("--url")
        .arg(url);
    command
}

/// 本文と HTTP の状態コードを返す。
fn fetch_text(url: &str, lang: Lang) -> Result<(u16, Vec<u8>), String> {
    let mut command = curl(url);
    command.args(["-w", "\n%{http_code}"]);
    let stdout = run(&mut command, network_error(lang), lang)?;
    split_status(&stdout).ok_or_else(|| network_error(lang).into())
}

/// dest へ書き出し、HTTP の状態コードを返す。
fn fetch_file(url: &str, dest: &Path, lang: Lang) -> Result<u16, String> {
    let mut command = curl(url);
    command.arg("-o").arg(dest).args(["-w", "%{http_code}"]);
    let stdout = run(&mut command, network_error(lang), lang)?;
    String::from_utf8_lossy(&stdout)
        .trim()
        .parse()
        .map_err(|_| network_error(lang).into())
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

// 新しく作られた順に返る。CI が main のコミットごとに作る build- のプレリリースは 3 件しか残さないので、
// 1 ページで最近の v のリリースまで届く。
fn releases_api(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/releases?per_page=50")
}

fn download_base(repo: &str, tag: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}")
}

/// リリースの一覧から、v で始まる正式なバージョンのうち最も新しいもののタグを選ぶ。プレリリース、build- のタグ、下書きは選ばない。
fn newest_release(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    json.as_array()?
        .iter()
        .filter(|r| r.get("draft").and_then(Value::as_bool) == Some(false))
        .filter(|r| r.get("prerelease").and_then(Value::as_bool) == Some(false))
        .filter_map(release_tag)
        .filter(|tag| tag.starts_with('v'))
        .filter_map(|tag| parse_version(tag).map(|v| (v, tag)))
        .max_by(|(a, _), (b, _)| compare_versions(a, b))
        .map(|(_, tag)| tag.to_string())
}

/// タグは URL とスクリプトに埋め込むので、バージョンの名前に使う文字だけでできたものに限る。
fn release_tag(json: &Value) -> Option<&str> {
    let tag = json.get("tag_name")?.as_str()?;
    let safe = !tag.is_empty()
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'));
    safe.then_some(tag)
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

/// latest が current より新しいかを返す。読めないバージョンは新しくないとする。
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => compare_versions(&latest, &current) == Ordering::Greater,
        _ => false,
    }
}

struct Version {
    core: Vec<u64>,
    pre: Vec<PreIdent>,
}

// SemVer のプレリリースの識別子の順序。数字だけのものは数として比べ、英字を含むものより前に置く。
// 列挙の順に並ぶので、Num を先に書く。
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PreIdent {
    Num(u64),
    Alpha(String),
}

/// 数字の並びを 0 で埋めて比べ、同じなら SemVer の規則でプレリリースを比べる。+ 以降のビルド情報は比べない。
fn compare_versions(a: &Version, b: &Version) -> Ordering {
    let len = a.core.len().max(b.core.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..len)
        .map(|i| at(&a.core, i).cmp(&at(&b.core, i)))
        .find(|o| o.is_ne())
        .unwrap_or_else(|| match (a.pre.is_empty(), b.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => a.pre.cmp(&b.pre),
        })
}

fn parse_version(version: &str) -> Option<Version> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version.split_once('+').map_or(version, |(v, _)| v);
    let (core, pre) = version.split_once('-').unwrap_or((version, ""));
    let core = core
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    let pre = if pre.is_empty() {
        Vec::new()
    } else {
        pre.split('.')
            .map(|id| match id.parse() {
                Ok(n) => Some(PreIdent::Num(n)),
                _ if !id.is_empty()
                    && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !id.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    Some(PreIdent::Alpha(id.to_string()))
                }
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?
    };
    Some(Version { core, pre })
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
    lang: Lang,
}

fn mac_script(h: &MacHelper) -> String {
    format!(
        "#!/bin/bash\nset -u\n\nwork={}\napp_dest={}\napp_pid={}\ntag={}\nbundle_id={}\n{}",
        sh_quote(h.work),
        sh_quote(h.app_dest),
        h.pid,
        sh_quote(h.tag),
        sh_quote(h.bundle_id),
        fill_messages(MAC_BODY, h.lang),
    )
}

struct WindowsHelper<'a> {
    work: &'a str,
    log: &'a str,
    pid: u32,
    tag: &'a str,
    installer: &'a str,
    hook: &'a str,
    lang: Lang,
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
        fill_messages(WINDOWS_BODY, h.lang).replace('\n', "\r\n"),
    )
}

/// 本文の `{{name}}` を、書き出すときの言語の文で置き換える。
fn fill_messages(body: &str, lang: Lang) -> String {
    SCRIPT_MESSAGES
        .iter()
        .fold(body.to_owned(), |text, (name, ja, en)| {
            text.replace(name, lang.pick(ja, en))
        })
}

// 補助のスクリプトが update.log へ書く文。文はスクリプトの引用符の中へそのまま埋め込むので、
// 埋め込む先の引用符の中での書き方で書く。
const SCRIPT_MESSAGES: &[(&str, &str, &str)] = &[
    ("{{error}}", "エラー", "Error"),
    (
        "{{quitting}}",
        "起動中の marimo を終了しています",
        "Quitting the running marimo",
    ),
    (
        "{{running_install}}",
        "marimo-hook install を実行しています",
        "Running marimo-hook install",
    ),
    (
        "{{launching}}",
        "marimo を起動しています",
        "Starting marimo",
    ),
    ("{{done}}", "更新が終わりました", "Update complete"),
    (
        "{{mac_updating}}",
        "正式なリリース ${tag} へ更新します（${app_dest}）",
        "Updating to stable release ${tag} (${app_dest})",
    ),
    (
        "{{mac_not_quit}}",
        "marimo が終了しません。右クリックメニューの「終了」で終了してから、もう一度更新してください。アプリは差し替えていません。",
        r#"marimo did not quit. Choose \"Quit\" from the right-click menu, then update again. The app was not replaced."#,
    ),
    (
        "{{mac_replacing}}",
        "$app_dest を差し替えています",
        "Replacing $app_dest",
    ),
    (
        "{{mac_copy_failed}}",
        "新しいアプリを $app_new へコピーできませんでした。アプリは差し替えずに起動し直しています。",
        "Could not copy the new app to $app_new. Relaunching the app without replacing it.",
    ),
    (
        "{{mac_move_aside_failed}}",
        "$app_dest を退避できませんでした。アプリは差し替えずに起動し直しています。",
        "Could not move $app_dest aside. Relaunching the app without replacing it.",
    ),
    (
        "{{mac_place_failed}}",
        "新しいアプリを $app_dest へ置けませんでした。元のアプリに戻して起動し直しています。",
        "Could not move the new app to $app_dest. Restoring the original app and relaunching it.",
    ),
    (
        "{{mac_install_failed}}",
        "marimo-hook install が失敗しました。アプリは最新バージョンとして起動に成功しています。上記のメッセージを確認してから、tools/update.sh を実行してください。",
        "marimo-hook install failed. The app has already been replaced with the new version and started. Check the messages above, then run tools/update.sh.",
    ),
    (
        "{{win_updating}}",
        "正式なリリース $tag へ更新します",
        "Updating to stable release $tag",
    ),
    (
        "{{win_kill_failed}}",
        "marimo（PID $($p.Id)）を終了できませんでした: $($_.Exception.Message)",
        "Could not quit marimo (PID $($p.Id)): $($_.Exception.Message)",
    ),
    (
        "{{win_left_to_installer}}",
        "marimo がまだ終了していないので、インストーラに終了を任せます",
        "marimo has not quit yet; leaving it to the installer to close it",
    ),
    (
        "{{win_reinstalling}}",
        "$(Split-Path -Leaf $installer) でアプリを入れ直しています",
        "Reinstalling the app with $(Split-Path -Leaf $installer)",
    ),
    (
        "{{win_installer_failed}}",
        r"インストーラが失敗しました（終了コード $($setup.ExitCode)）。marimo を終了できなかった場合に起こるので、立ち絵を右クリックして「終了」を選んでから、tools\update.ps1 を実行してください。",
        r#"The installer failed (exit code $($setup.ExitCode)). This happens when marimo could not be closed. Right-click the character, choose ""Quit"", then run tools\update.ps1."#,
    ),
    (
        "{{win_missing_exe}}",
        "インストーラは成功を返しましたが、$installedExe が見つかりません。",
        "The installer reported success, but $installedExe was not found.",
    ),
    (
        "{{win_install_failed}}",
        r"marimo-hook install が失敗しました。アプリはすでに新しいものに入れ直して起動しています。上記のメッセージを確認してから、tools\update.ps1 を実行してください。",
        r"marimo-hook install failed. The new version of the app has already been installed and started. Check the messages above, then run tools\update.ps1.",
    ),
];

// 標準出力と標準エラーはアプリが update.log へつないで起動する。Finder や launchd から起動したアプリの
// 子は C ロケールで動き、bash 3.2 は変数名の直後の非 ASCII のバイトを名前の一部として読むので、
// SCRIPT_MESSAGES の文でも日本語の直前の変数は ${name} の形で書く。
const MAC_BODY: &str = r#"
bundle="$work/extract/marimo.app"
hook="$work/extract/marimo-hook"
app_new="$(dirname "$app_dest")/.$(basename "$app_dest").new"
app_old="$(dirname "$app_dest")/.$(basename "$app_dest").old"
running_pattern="$app_dest/Contents/MacOS/"
relaunched=0

launch() {
  open "$app_dest"
  relaunched=1
}

# 途中で想定外に止まっても、アプリが終了したままにならないよう、まだ起動し直していなければ起動する。
cleanup() {
  if [[ $relaunched -eq 0 ]] && ! pgrep -f "$running_pattern" >/dev/null; then
    open "$app_dest"
  fi
  rm -rf "$work"
}
trap cleanup EXIT

say() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

fail() {
  say "{{error}}: $*"
  exit 1
}

say "{{mac_updating}}"

# アプリはこのスクリプトを起動した直後に終了するので、まずそれを待つ。
for _ in $(seq 1 60); do
  kill -0 "$app_pid" 2>/dev/null || break
  sleep 0.5
done

if pgrep -f "$running_pattern" >/dev/null; then
  say "{{quitting}}"
  osascript -e "tell application id \"$bundle_id\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    pgrep -f "$running_pattern" >/dev/null || break
    sleep 0.5
  done
  if pgrep -f "$running_pattern" >/dev/null; then
    fail "{{mac_not_quit}}"
  fi
fi

# ここから先で止まるときは、アプリが終了したままにならないよう、残っている方のアプリを起動し直す。
say "{{mac_replacing}}"
rm -rf "$app_new" "$app_old"
if ! ditto "$bundle" "$app_new"; then
  rm -rf "$app_new"
  launch
  fail "{{mac_copy_failed}}"
fi
if [[ -e "$app_dest" ]] && ! mv "$app_dest" "$app_old"; then
  rm -rf "$app_new"
  launch
  fail "{{mac_move_aside_failed}}"
fi
if ! mv "$app_new" "$app_dest"; then
  [[ -e "$app_old" ]] && mv "$app_old" "$app_dest"
  launch
  fail "{{mac_place_failed}}"
fi
rm -rf "$app_old"

say "{{running_install}}"
install_ok=1
"$hook" install || install_ok=0

say "{{launching}}"
launch

if [[ $install_ok -eq 0 ]]; then
  # 一時フォルダは終わると消えるので、install をやり直すときは更新ごとやり直してもらう。
  fail "{{mac_install_failed}}"
fi
say "{{done}}"
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
    Write-Log "{{error}}: $Message"
    exit 1
}

function Get-InstalledMarimo([string]$Dir) {
    $prefix = $Dir.TrimEnd('\') + '\'
    @(Get-Process -Name marimo -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    })
}

try {
    Write-Log "{{win_updating}}"

    # アプリはこのスクリプトを起動した直後に終了するので、まずそれを待つ。
    $app = Get-Process -Id $appPid -ErrorAction SilentlyContinue
    if ($app) { [void]$app.WaitForExit(30000) }

    $running = @(Get-InstalledMarimo $installDir)
    if ($running.Count -gt 0) {
        Write-Log '{{quitting}}'
        foreach ($p in $running) {
            if ($p.HasExited) { continue }
            try {
                $p.Kill()
            } catch {
                Write-Log "{{win_kill_failed}}"
            }
        }
        # 終了させたプロセスは、一覧を取り直すと、終了処理が済む前のプロセスがまだ見えることがあるので、同じオブジェクトの終了を待つ。
        # それでも残るときは、動いているアプリを自分で閉じるインストーラに任せて先へ進む。
        $left = @($running | Where-Object { -not $_.WaitForExit(10000) })
        if ($left.Count -gt 0) {
            Write-Log '{{win_left_to_installer}}'
        }
    }

    Write-Log "{{win_reinstalling}}"
    $setup = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
    if ($setup.ExitCode -ne 0) {
        # アプリが終了したままにならないよう、残っている方のアプリを起動し直す。
        if (Test-Path -LiteralPath $installedExe) { Start-Process -FilePath $installedExe }
        Fail "{{win_installer_failed}}"
    }
    if (-not (Test-Path -LiteralPath $installedExe)) {
        Fail "{{win_missing_exe}}"
    }

    Write-Log '{{running_install}}'
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & $hook install 2>&1 | Out-String
    } finally {
        $ErrorActionPreference = $saved
    }
    $installOk = ($LASTEXITCODE -eq 0)
    if ($output.Trim()) { Write-Log $output.TrimEnd() }

    Write-Log '{{launching}}'
    Start-Process -FilePath $installedExe

    if (-not $installOk) {
        # 一時フォルダは終わると消えるので、install をやり直すときは更新ごとやり直してもらう。
        Fail "{{win_install_failed}}"
    }
    Write-Log '{{done}}'
} catch {
    Write-Log "{{error}}: $($_.Exception.Message)"
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
            ("0.1.2-beta.1", "0.1.1", true),
            ("0.1.2-beta.2", "0.1.2-beta.1", true),
            ("0.1.2-beta.10", "0.1.2-beta.2", true),
            ("0.1.2-rc.1", "0.1.2-beta.9", true),
            ("0.1.2-beta.1", "0.1.2-beta", true),
            ("0.1.2-beta.1", "0.1.2-beta.1", false),
            ("0.1.2-beta..1", "0.1.1", false),
            ("0.1.2-be_ta", "0.1.1", false),
        ] {
            assert_eq!(
                is_newer(latest, current),
                newer,
                "latest {latest:?}, current {current:?}"
            );
        }
    }

    #[test]
    fn picks_newest_formal_v_release() {
        let release = |tag: &str, prerelease: bool, draft: bool| {
            format!(r#"{{"tag_name": "{tag}", "prerelease": {prerelease}, "draft": {draft}}}"#)
        };
        let body = format!(
            "[{}]",
            [
                release("build-1c96f341edd2", true, false),
                release("v0.1.3-beta.1", true, false),
                release("v0.2.0", false, true),
                release("v0.1.2-beta.10", true, false),
                release("v0.1.2-beta.2", true, false),
                release("v0.1.1", false, false),
                release("v1'; rm -rf ~", false, false),
                release("v0.1.0", false, false),
            ]
            .join(",")
        );
        for (body, want) in [
            (body.as_str(), Some("v0.1.1")),
            ("[]", None),
            (r#"{"message": "Not Found"}"#, None),
            ("not json", None),
        ] {
            let got = newest_release(body.as_bytes());
            assert_eq!(got.as_deref(), want, "{body:?}");
        }
        assert_eq!(
            download_base("owner/repo", "v0.2.0"),
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
            ("日\u{2019}s", "'日\u{2019}s'", "'日\u{2019}\u{2019}s'"),
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
            lang: Lang::Ja,
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
            lang: Lang::Ja,
        });
        assert!(windows.contains("\r\n$log = 'C:\\Users\\o''neil\\.marimo\\update.log'\r\n"));
        assert!(windows.contains("\r\n$appPid = 42\r\n"));
        assert!(!windows.replace("\r\n", "").contains('\n'));
    }

    fn sample_mac_script(lang: Lang) -> String {
        mac_script(&MacHelper {
            work: "/tmp/w",
            app_dest: "/Applications/marimo.app",
            pid: 1,
            tag: "v0.2.0",
            bundle_id: "com.marimo.desktop",
            lang,
        })
    }

    fn sample_windows_script(lang: Lang) -> String {
        windows_script(&WindowsHelper {
            work: r"C:\w",
            log: r"C:\Users\a\.marimo\update.log",
            pid: 1,
            tag: "v0.2.0",
            installer: r"C:\w\marimo_0.2.0_x64-setup.exe",
            hook: r"C:\w\marimo-hook.exe",
            lang,
        })
    }

    #[cfg(unix)]
    #[test]
    fn mac_script_parses_as_bash() {
        for lang in [Lang::Ja, Lang::En] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("update.sh");
            fs::write(&path, sample_mac_script(lang)).unwrap();
            let status = Command::new("/bin/bash")
                .arg("-n")
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success(), "{lang:?}");
        }
    }

    // アプリが起動するのは Windows PowerShell 5.1 なので、それで読む。ファイルは install_update と同じく BOM を付けて書く。
    #[cfg(windows)]
    #[test]
    fn windows_script_parses_as_powershell() {
        for lang in [Lang::Ja, Lang::En] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("update.ps1");
            fs::write(&path, format!("\u{feff}{}", sample_windows_script(lang))).unwrap();
            let check = format!(
                "$e = $null; [void][System.Management.Automation.Language.Parser]::ParseFile({}, [ref]$null, [ref]$e); if ($e) {{ $e | Out-String | Write-Host; exit 1 }}",
                ps_quote(&path.to_string_lossy())
            );
            let status = windows_powershell()
                .args(["-NoProfile", "-NonInteractive", "-Command", check.as_str()])
                .status()
                .unwrap();
            assert!(status.success(), "{lang:?}");
        }
    }

    #[test]
    fn windows_powershell_drops_ps_module_path() {
        let command = windows_powershell();
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "PSModulePath" && value.is_none())
        );
    }

    #[test]
    fn mac_script_braces_variables_before_non_ascii() {
        for lang in [Lang::Ja, Lang::En] {
            let script = sample_mac_script(lang);
            let bytes = script.as_bytes();
            for (i, _) in script.match_indices('$') {
                let name_end = bytes[i + 1..]
                    .iter()
                    .position(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
                    .map_or(bytes.len(), |n| i + 1 + n);
                assert!(
                    name_end == i + 1 || bytes.get(name_end).is_none_or(u8::is_ascii),
                    "{}",
                    script[i..].lines().next().unwrap_or_default()
                );
            }
        }
    }

    #[test]
    fn helper_scripts_write_english_messages() {
        let mac = sample_mac_script(Lang::En);
        let windows = sample_windows_script(Lang::En);
        for script in [&mac, &windows] {
            assert!(!script.contains("{{"), "{script}");
            let code: String = script
                .lines()
                .filter(|line| !line.trim_start().starts_with('#'))
                .collect();
            assert!(code.is_ascii(), "{script}");
        }
        assert!(mac.contains("\n  say \"Error: $*\"\n"));
        assert!(
            mac.contains(r#"fail "marimo did not quit. Choose \"Quit\" from the right-click menu"#)
        );
        assert!(windows.contains("\r\n    Write-Log 'Update complete'\r\n"));
        assert!(windows.contains(r#"choose ""Quit"", then run tools\update.ps1.""#));
        assert!(!windows.replace("\r\n", "").contains('\n'));

        for lang in [Lang::Ja, Lang::En] {
            let script = sample_windows_script(lang);
            assert!(!script.contains("{{"), "{lang:?}");
        }
        assert!(sample_mac_script(Lang::Ja).contains("\n  say \"エラー: $*\"\n"));
    }
}
