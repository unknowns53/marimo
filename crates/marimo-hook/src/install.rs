use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use marimo_core::MarimoHome;
use marimo_core::paths::user_home;
use marimo_core::time::now_ms;
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
    let marimo = Marimo::new(&command_path(&exe));
    match mode {
        Mode::Install => install(&opts, &exe, &marimo),
        Mode::Uninstall => uninstall(&opts, &home, &exe, &marimo),
    }
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

// Windows のフックは Git Bash で動くことがあり、Git Bash は引用していない \ を
// エスケープとして消してしまう。statusline のドキュメントの Windows configuration の
// 節にあるとおり、区切りを / にしておく。
fn command_path(exe: &Path) -> String {
    let s = exe.to_string_lossy();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

fn install(opts: &Options, exe: &Path, marimo: &Marimo) -> Result<(), String> {
    let original = read_settings(&opts.settings, true)?;
    let mut settings = original
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let report = settings_edit::install(&mut settings, marimo, !cfg!(windows))?;
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
    if !report.already.is_empty() {
        out += &format!("登録済みのフック: {}\n", report.already.join(", "));
    }
    out += &describe_status_line(&report.status_line);

    if report.changed() {
        out += &commit(opts, original.is_some(), &settings)?;
    } else {
        out += "変更はありません（すでにインストール済みです）。\n";
    }
    if cfg!(windows) {
        out += "Windows では statusLine を書き換えていないため、利用制限は表示されません。\n";
    }
    print(&out);
    Ok(())
}

fn uninstall(opts: &Options, home: &MarimoHome, exe: &Path, marimo: &Marimo) -> Result<(), String> {
    let Some(original) = read_settings(&opts.settings, false)? else {
        print(&format!(
            "{} がないので、取り除くものはありません。\n",
            opts.settings.display()
        ));
        return Ok(());
    };
    let mut settings = original.clone();
    let report = settings_edit::uninstall(&mut settings, marimo)?;
    let mut out = String::new();
    if opts.dry_run {
        out += "dry-run のため、ファイルは書き換えていません。\n";
    }
    out += &format!("設定ファイル: {}\n", opts.settings.display());
    out += &format!("取り除くフック: {}\n", list_or_none(&report.removed));
    out += &describe_status_line(&report.status_line);
    if report.changed() {
        out += &commit(opts, true, &settings)?;
    } else {
        out += "変更はありません（marimo は登録されていません）。\n";
    }
    out += &format!(
        "実行ファイルとデータは残しています。不要なら次を消してください。\n  {}\n  {}\n",
        exe.display(),
        home.root().display()
    );
    print(&out);
    Ok(())
}

fn commit(opts: &Options, existed: bool, settings: &Value) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    if opts.dry_run {
        return Ok(format!("変更後の settings.json:\n{text}"));
    }
    let target = resolve_symlink(&opts.settings);
    let mut out = String::new();
    if existed {
        let backup = backup(&target)?;
        out += &format!("バックアップ: {}\n", backup.display());
    }
    write_replacing(&target, text.as_bytes())?;
    let reread = fs::read(&target).map_err(|e| format!("書き込んだ設定を読み直せません: {e}"))?;
    serde_json::from_slice::<Value>(&reread).map_err(|e| {
        format!("書き込んだ設定が JSON として読めません（バックアップから戻してください）: {e}")
    })?;
    out += "settings.json を書き換えました。\n";
    Ok(out)
}

fn read_settings(path: &Path, allow_missing: bool) -> Result<Option<Value>, String> {
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
    Ok(Some(value))
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
    let result = (|| -> io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        // 元のファイルの権限（0600 など）を新しいファイルにも引き継ぐ。
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&tmp, meta.permissions())?;
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
    fs::create_dir_all(dir).map_err(|e| format!("{} を作れません: {e}", dir.display()))?;
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
    fn stamp_format() {
        assert_eq!(utc_stamp(1_790_509_325_123), "20260927-114205");
    }
}
