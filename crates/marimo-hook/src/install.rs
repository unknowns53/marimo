use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use marimo_core::codex::codex_home;
use marimo_core::paths::user_home;
use marimo_core::time::now_ms;
use marimo_core::{MarimoHome, store};
use serde_json::Value;

use crate::settings_edit::{self, Marimo, StatusLineChange};

pub enum Mode {
    Install,
    Uninstall,
}

struct Options {
    settings: PathBuf,
    dry_run: bool,
}

pub fn run(mode: Mode, args: &[OsString]) -> Result<(), String> {
    let opts = parse_args(args)?;
    let home =
        MarimoHome::resolve().ok_or("marimo のフォルダを決められません（HOME が未設定です）")?;
    let exe = home.hook_executable();
    let marimo = marimo_for(&exe);
    match mode {
        Mode::Install => {
            install(&opts, &exe, &marimo)?;
            codex(&mode, &opts, &exe)
        }
        Mode::Uninstall => {
            uninstall(&opts, &marimo)?;
            codex(&mode, &opts, &exe)?;
            print(&format!(
                "実行ファイルとデータは残しています。不要なら次を消してください。\n  {}\n  {}\n",
                exe.display(),
                home.root().display()
            ));
            Ok(())
        }
    }
}

const CODEX_TRUST_NOTE: &str = "Codex は、利用者が Codex CLI の /hooks でフックを確かめて信頼するまで、追加したフックを実行しません。marimo-hook の場所が変わったときは、もう一度 /hooks で信頼してください。\n";

// Codex のフォルダが無ければ Codex は入っていないものとして、何も表示しない。
// 信頼の記録は config.toml にあるが、利用者が /hooks で確かめる前提を崩さないよう触らない。
fn codex(mode: &Mode, opts: &Options, exe: &Path) -> Result<(), String> {
    let Some(dir) = codex_home().filter(|d| d.is_dir()) else {
        return Ok(());
    };
    let path = dir.join("hooks.json");
    let mut out = String::from("\nCodex\n");
    let Some(command) = settings_edit::codex_hook_command(&exe.to_string_lossy(), cfg!(windows))
    else {
        out += "marimo-hook のパスに空白などが含まれ、Codex のシェルに引用なしで渡せないため、Codex のフックは扱いませんでした。\n";
        print(&out);
        return Ok(());
    };
    if opts.dry_run {
        out += "dry-run のため、ファイルは書き換えていません。\n";
    }
    out += &format!("設定ファイル: {}\n", path.display());
    match mode {
        Mode::Install => {
            let original = read_settings(&path, true)?;
            let mut file = original
                .as_ref()
                .map_or_else(|| Value::Object(Default::default()), |s| s.value.clone());
            let report = settings_edit::install_codex(&mut file, &command)?;
            out += &format!("追加するフック: {}\n", list_or_none(&report.added));
            if !report.already.is_empty() {
                out += &format!("登録済みのフック: {}\n", report.already.join(", "));
            }
            if report.added.is_empty() {
                out += "変更はありません（すでにインストール済みです）。\n";
            } else {
                out += &commit(
                    &path,
                    opts.dry_run,
                    original.as_ref().map(|s| s.bytes.as_slice()),
                    &file,
                )?;
                out += CODEX_TRUST_NOTE;
            }
        }
        Mode::Uninstall => match read_settings(&path, false)? {
            None => out += "hooks.json がないので、取り除くものはありません。\n",
            Some(original) => {
                let mut file = original.value.clone();
                let removed = settings_edit::uninstall_codex(&mut file)?;
                out += &format!("取り除くフック: {}\n", list_or_none(&removed));
                if removed.is_empty() {
                    out += "変更はありません（marimo は登録されていません）。\n";
                } else {
                    out += &commit(&path, opts.dry_run, Some(&original.bytes), &file)?;
                }
            }
        },
    }
    print(&out);
    Ok(())
}

fn parse_args(args: &[OsString]) -> Result<Options, String> {
    let mut settings = None;
    let mut dry_run = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.to_str() {
            Some("--dry-run") => dry_run = true,
            Some("--settings") => {
                let path = it
                    .next()
                    .ok_or("--settings の後に settings.json のパスが必要です")?;
                settings = Some(PathBuf::from(path));
            }
            _ => return Err(format!("不明な引数です: {}", arg.to_string_lossy())),
        }
    }
    let settings = match settings {
        Some(p) => p,
        None => default_settings_path()?,
    };
    Ok(Options { settings, dry_run })
}

// CLAUDE_CONFIG_DIR は設定を置くフォルダを ~/.claude から差し替える。
// https://code.claude.com/docs/en/env-vars と https://code.claude.com/docs/en/settings に記載がある。
fn default_settings_path() -> Result<PathBuf, String> {
    if let Some(dir) = env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("settings.json"));
    }
    user_home()
        .map(|h| h.join(".claude").join("settings.json"))
        .ok_or_else(|| "ホームフォルダが分からないので、--settings で指定してください".to_owned())
}

fn marimo_for(exe: &Path) -> Marimo {
    let exe = exe.to_string_lossy();
    if cfg!(windows) {
        let home = user_home().map(|h| h.to_string_lossy().into_owned());
        Marimo::windows(&exe, home.as_deref())
    } else {
        Marimo::new(&exe)
    }
}

fn install(opts: &Options, exe: &Path, marimo: &Marimo) -> Result<(), String> {
    let original = read_settings(&opts.settings, true)?;
    let mut settings = original
        .as_ref()
        .map_or_else(|| Value::Object(Default::default()), |s| s.value.clone());
    let report = settings_edit::install(&mut settings, marimo)?;
    let mut out = String::new();

    if opts.dry_run {
        out += "dry-run のため、ファイルは書き換えていません。\n";
        let note = if running_from(exe) {
            "すでにこの場所から実行しているのでコピーしません"
        } else {
            "コピーします"
        };
        out += &format!("実行ファイル: {}（{note}）\n", exe.display());
    } else {
        let copied = copy_self(exe)?;
        out += &format!(
            "実行ファイル: {}（{}）\n",
            exe.display(),
            if copied {
                "コピーしました"
            } else {
                "すでにこの場所から実行しているのでコピーしません"
            }
        );
    }
    out += &format!("設定ファイル: {}\n", opts.settings.display());
    out += &format!("追加するフック: {}\n", list_or_none(&report.added));
    if !report.migrated.is_empty() {
        out += &format!(
            "シェルを通さない exec form へ書き換えるフック: {}\n",
            report.migrated.join(", ")
        );
    }
    if !report.already.is_empty() {
        out += &format!("登録済みのフック: {}\n", report.already.join(", "));
    }
    out += &describe_status_line(&report.status_line);

    if report.changed() {
        out += &commit(
            &opts.settings,
            opts.dry_run,
            original.as_ref().map(|s| s.bytes.as_slice()),
            &settings,
        )?;
    } else {
        out += "変更はありません（すでにインストール済みです）。\n";
    }
    print(&out);
    Ok(())
}

fn uninstall(opts: &Options, marimo: &Marimo) -> Result<(), String> {
    let Some(original) = read_settings(&opts.settings, false)? else {
        print(&format!(
            "{} がないので、取り除くものはありません。\n",
            opts.settings.display()
        ));
        return Ok(());
    };
    let mut settings = original.value.clone();
    let report = settings_edit::uninstall(&mut settings, marimo)?;
    let mut out = String::new();
    if opts.dry_run {
        out += "dry-run のため、ファイルは書き換えていません。\n";
    }
    out += &format!("設定ファイル: {}\n", opts.settings.display());
    out += &format!("取り除くフック: {}\n", list_or_none(&report.removed));
    out += &describe_status_line(&report.status_line);
    if report.changed() {
        out += &commit(
            &opts.settings,
            opts.dry_run,
            Some(&original.bytes),
            &settings,
        )?;
    } else {
        out += "変更はありません（marimo は登録されていません）。\n";
    }
    print(&out);
    Ok(())
}

// original は読み込んだときのファイルの中身で、ファイルがなかったときは None になる。
fn commit(
    path: &Path,
    dry_run: bool,
    original: Option<&[u8]>,
    settings: &Value,
) -> Result<String, String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    if dry_run {
        return Ok(format!("変更後の {name}:\n{text}"));
    }
    let target = resolve_symlink(path);
    ensure_unchanged(&target, original)?;
    let mut out = String::new();
    if original.is_some() {
        let backup = backup(&target)?;
        out += &format!("バックアップ: {}\n", backup.display());
    }
    write_replacing(&target, text.as_bytes())?;
    let reread = fs::read(&target).map_err(|e| format!("書き込んだ設定を読み直せません: {e}"))?;
    serde_json::from_slice::<Value>(&reread).map_err(|e| {
        format!("書き込んだ設定が JSON として読めません（バックアップから戻してください）: {e}")
    })?;
    out += &format!("{name} を書き換えました。\n");
    Ok(out)
}

struct Loaded {
    bytes: Vec<u8>,
    value: Value,
}

fn read_settings(path: &Path, allow_missing: bool) -> Result<Option<Loaded>, String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound && allow_missing => {
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
            if parent.is_some_and(|p| !p.is_dir()) {
                return Err(format!("{} のフォルダがありません", path.display()));
            }
            return Ok(None);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{} を読めません: {e}", path.display())),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "{} が JSON として読めないので、何も変更していません: {e}",
            path.display()
        )
    })?;
    if !value.is_object() {
        return Err(format!(
            "{} の最上位が JSON のオブジェクトではないので、何も変更していません",
            path.display()
        ));
    }
    Ok(Some(Loaded { bytes, value }))
}

// 読み込んでから書き戻すまでの間に Claude Code やエディタが settings.json を書き換えていたら、
// その変更を上書きで失わないよう、何も書かずに止めてやり直してもらう。
fn ensure_unchanged(path: &Path, original: Option<&[u8]>) -> Result<(), String> {
    let current = match fs::read(path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(format!(
                "{} を読み直せないので、何も変更していません: {e}",
                path.display()
            ));
        }
    };
    if current.as_deref() == original {
        Ok(())
    } else {
        Err(format!(
            "{} が処理の途中で書き換えられたので、何も変更していません。もう一度実行してください",
            path.display()
        ))
    }
}

// dotfiles の管理で settings.json がシンボリックリンクになっている場合、rename で
// リンクそのものを普通のファイルに置き換えないよう、リンク先を書き換える。
fn resolve_symlink(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

fn backup(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("settings.json");
    let stamp = utc_stamp(now_ms());
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut candidate = dir.join(format!("{name}.marimo-backup-{stamp}"));
    let mut n = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{name}.marimo-backup-{stamp}-{n}"));
        n += 1;
    }
    fs::copy(path, &candidate)
        .map_err(|e| format!("バックアップを作れないので、何も変更していません: {e}"))?;
    Ok(candidate)
}

// 日時の計算のためだけに依存を増やさないよう、UTC で名前を付ける。
fn utc_stamp(ms: u64) -> String {
    let iso = marimo_core::time::rfc3339_utc(ms);
    format!(
        "{}{}{}-{}{}{}",
        &iso[0..4],
        &iso[5..7],
        &iso[8..10],
        &iso[11..13],
        &iso[14..16],
        &iso[17..19]
    )
}

fn write_replacing(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("settings.json");
    let tmp = dir.join(format!(".{name}.marimo-{}.tmp", std::process::id()));
    // 前に失敗した実行の一時ファイルが残っていると create_new が失敗するので、先に消す。
    let _ = fs::remove_file(&tmp);
    let original = fs::metadata(path).ok().map(|m| m.permissions());
    let result = (|| -> io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        // 書き込む前から元のファイルの権限で作り、0600 の設定が umask の既定の権限で
        // 一瞬でも他人に読める状態にならないようにする。元のファイルがなければ本人だけにする。
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(original.as_ref().map_or(0o600, |p| p.mode() & 0o777));
        }
        let mut f = options.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        // umask で落ちたビットも含めて、元のファイルの権限を引き継ぐ。
        if let Some(permissions) = original {
            fs::set_permissions(&tmp, permissions)?;
        }
        fs::rename(&tmp, path)
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{} を書き込めません: {e}", path.display())
    })
}

fn running_from(dest: &Path) -> bool {
    let current = env::current_exe().and_then(fs::canonicalize);
    matches!((current, fs::canonicalize(dest)), (Ok(a), Ok(b)) if a == b)
}

fn copy_self(dest: &Path) -> Result<bool, String> {
    let current =
        env::current_exe().map_err(|e| format!("自分の実行ファイルの場所が分かりません: {e}"))?;
    if running_from(dest) {
        return Ok(false);
    }
    let dir = dest.parent().ok_or("実行ファイルの置き場所が不正です")?;
    store::create_private_dir_all(dir)
        .map_err(|e| format!("{} を作れません: {e}", dir.display()))?;
    // 実行中の古いバイナリを上書きせず、別名で書いてから rename して差し替える。
    // macOS は同じ inode への上書きで署名の検証に失敗し、実行中のプロセスを落とすことがある。
    let tmp = dir.join(format!(".marimo-hook.{}.tmp", std::process::id()));
    fs::copy(&current, &tmp)
        .and_then(|_| fs::rename(&tmp, dest))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("{} へコピーできません: {e}", dest.display())
        })?;
    Ok(true)
}

fn describe_status_line(change: &StatusLineChange) -> String {
    match change {
        StatusLineChange::Added { after } => {
            format!("statusLine: 未設定だったので登録します\n  変更後: {after}\n")
        }
        StatusLineChange::Wrapped { before, after } => {
            format!("statusLine: 元のコマンドを包みます\n  変更前: {before}\n  変更後: {after}\n")
        }
        StatusLineChange::AlreadyMarimo { current } => {
            format!("statusLine: すでに marimo を通っています\n  現在: {current}\n")
        }
        StatusLineChange::Removed { before } => {
            format!("statusLine: marimo だけだったので取り除きます\n  変更前: {before}\n")
        }
        StatusLineChange::Restored { before, after } => {
            format!("statusLine: 元のコマンドへ戻します\n  変更前: {before}\n  変更後: {after}\n")
        }
        StatusLineChange::Untouched { reason } => format!("statusLine: {reason}\n"),
    }
}

fn list_or_none<S: AsRef<str>>(items: &[S]) -> String {
    if items.is_empty() {
        "なし".to_owned()
    } else {
        items
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn print(text: &str) {
    let _ = io::stdout().lock().write_all(text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_writes_nothing_when_settings_changed_after_reading() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let edited = serde_json::json!({"hooks": {}});

        fs::write(&settings, "{\"model\": \"sonnet\"}").unwrap();
        assert!(commit(&settings, false, Some(b"{}"), &edited).is_err());
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "{\"model\": \"sonnet\"}"
        );

        // 読んだときはなかったファイルが、書く前に作られていた場合も止める。
        let fresh = dir.path().join("fresh.json");
        fs::write(&fresh, "{}").unwrap();
        assert!(commit(&fresh, false, None, &edited).is_err());
        assert_eq!(fs::read_to_string(&fresh).unwrap(), "{}");

        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "no backup or temp file: {names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn replacing_keeps_the_original_mode_and_new_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for original in [0o600, 0o644, 0o664] {
            let path = dir.path().join(format!("settings-{original:o}.json"));
            fs::write(&path, "{}").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(original)).unwrap();
            write_replacing(&path, b"{\"a\": 1}\n").unwrap();
            assert_eq!(mode(&path), original);
            assert_eq!(fs::read(&path).unwrap(), b"{\"a\": 1}\n");
        }

        let fresh = dir.path().join("fresh.json");
        // 前の実行が残した一時ファイルがあっても書き込める。
        let stale = dir
            .path()
            .join(format!(".fresh.json.marimo-{}.tmp", std::process::id()));
        fs::write(&stale, "stale").unwrap();
        write_replacing(&fresh, b"{}\n").unwrap();
        assert_eq!(mode(&fresh), 0o600);
        assert!(!stale.exists());
    }
}
