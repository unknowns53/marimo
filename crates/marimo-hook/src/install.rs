use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use marimo_core::codex::codex_home;
use marimo_core::paths::user_home;
use marimo_core::time::now_ms;
use marimo_core::{Lang, MarimoHome, store};
use serde_json::Value;

use crate::opencode_plugin;
use crate::settings_edit::{self, Marimo, StatusLineChange};

pub enum Mode {
    Install,
    Uninstall,
}

struct Options {
    settings: PathBuf,
    /// --settings で settings.json を指定したか。指定していなければ、Claude Code の設定フォルダが無いときは
    /// Claude Code が入っていないものとして飛ばす。
    explicit_settings: bool,
    dry_run: bool,
}

/// home は MARIMO_HOME か ~/.marimo で、決められなければ None。lang は呼び出し側がその home の display.json と
/// OS の言語から決めたもので、--settings で別の settings.json を指しても変わらない。
pub fn run(
    mode: Mode,
    args: &[OsString],
    home: Option<MarimoHome>,
    lang: Lang,
) -> Result<(), String> {
    let opts = parse_args(args, lang)?;
    let home = home.ok_or(lang.pick(
        "marimo のフォルダを決められません（HOME が未設定です）",
        "Cannot determine the marimo folder (HOME is not set)",
    ))?;
    let exe = home.hook_executable();
    let marimo = marimo_for(&exe);
    // ツールごとの設定ファイルは別々に読み書きするので、一つが失敗してもほかのツールは続けて処理し、
    // 最後に失敗したツールをまとめて伝える。
    let mut failed = Vec::new();
    match mode {
        Mode::Install => {
            if opts.dry_run {
                print(dry_run_note(lang));
            }
            match copy_executable(&opts, &exe, lang) {
                Ok(line) => print(&line),
                // 前の版の実行ファイルが残っていれば、各ツールの登録はそれを指したまま使える。
                Err(e) if exe.is_file() => record_failure(&mut failed, "marimo-hook", Err(e)),
                Err(e) => return Err(e),
            }
            record_failure(&mut failed, "Claude Code", install(&opts, &marimo, lang));
            record_failure(&mut failed, "Codex", codex(&mode, &opts, &exe, lang));
            record_failure(&mut failed, "OpenCode", opencode(&mode, &opts, &exe, lang));
            record_failure(
                &mut failed,
                "Antigravity",
                antigravity(&mode, &opts, &exe, lang),
            );
            failures(&failed, lang)
        }
        Mode::Uninstall => {
            record_failure(&mut failed, "Claude Code", uninstall(&opts, &marimo, lang));
            record_failure(&mut failed, "Codex", codex(&mode, &opts, &exe, lang));
            record_failure(&mut failed, "OpenCode", opencode(&mode, &opts, &exe, lang));
            record_failure(
                &mut failed,
                "Antigravity",
                antigravity(&mode, &opts, &exe, lang),
            );
            let (exe, root) = (exe.display(), home.root().display());
            print(&match lang {
                Lang::Ja => format!(
                    "実行ファイルとデータは残しています。不要なら次を消してください。\n  {exe}\n  {root}\n"
                ),
                Lang::En => format!(
                    "The executable and data were kept. Delete the following if you no longer need them.\n  {exe}\n  {root}\n"
                ),
            });
            failures(&failed, lang)
        }
    }
}

fn record_failure(failed: &mut Vec<&'static str>, tool: &'static str, result: Result<(), String>) {
    if let Err(e) = result {
        let _ = writeln!(io::stderr(), "marimo-hook: {tool}: {e}");
        failed.push(tool);
    }
}

fn failures(failed: &[&str], lang: Lang) -> Result<(), String> {
    if failed.is_empty() {
        return Ok(());
    }
    let list = failed.join(", ");
    Err(match lang {
        Lang::Ja => format!(
            "{list} は処理できませんでした。上のメッセージを確認してください。ほかのツールは上の表示のとおり処理しました"
        ),
        Lang::En => format!(
            "Could not process {list}; check the messages above. The other tools were processed as shown above"
        ),
    })
}

fn codex_trust_note(lang: Lang) -> &'static str {
    lang.pick(
        "Codex は、利用者が Codex CLI の /hooks でフックを確認後信頼するまで、追加したフックを実行しません。marimo-hook の場所が変わったときは、もう一度 /hooks で信頼してください。\n",
        "Codex does not run the added hooks until you review and trust them with /hooks in the Codex CLI. If marimo-hook moves, trust them again with /hooks.\n",
    )
}

fn dry_run_note(lang: Lang) -> &'static str {
    lang.pick(
        "dry-run のため、ファイルは書き換えていません。\n",
        "dry-run: no files were changed.\n",
    )
}

fn settings_file_line(path: &Path, lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("設定ファイル: {}\n", path.display()),
        Lang::En => format!("Settings file: {}\n", path.display()),
    }
}

fn hooks_line(label: Label, items: &[impl AsRef<str>], lang: Lang) -> String {
    let (ja, en) = match label {
        Label::Add => ("追加するフック", "Hooks to add"),
        Label::Already => ("登録済みのフック", "Hooks already registered"),
        Label::Migrate => (
            "シェルを通さない exec form へ書き換えるフック",
            "Hooks to rewrite in exec form (no shell)",
        ),
        Label::Remove => ("取り除くフック", "Hooks to remove"),
    };
    format!("{}: {}\n", lang.pick(ja, en), list_or_none(items, lang))
}

enum Label {
    Add,
    Already,
    Migrate,
    Remove,
}

fn already_installed(lang: Lang) -> &'static str {
    lang.pick(
        "変更はありません（すでにインストール済みです）。\n",
        "No changes (already installed).\n",
    )
}

fn not_registered(lang: Lang) -> &'static str {
    lang.pick(
        "変更はありません（marimo は登録されていません）。\n",
        "No changes (marimo is not registered).\n",
    )
}

fn nothing_to_remove(name: &str, lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("{name} がないので、取り除くものはありません。\n"),
        Lang::En => format!("{name} does not exist, so there is nothing to remove.\n"),
    }
}

// tool はシェルの持ち主の名前、hooks は扱わなかったフックの持ち主の名前。
fn unquotable_path(tool: &str, hooks: &str, lang: Lang) -> String {
    match lang {
        Lang::Ja => format!(
            "marimo-hook のパスに空白などが含まれ、{tool} のシェルに引用なしで渡せないため、{hooks} のフックは扱いませんでした。\n"
        ),
        Lang::En => format!(
            "Skipped the {hooks} hooks because the marimo-hook path contains spaces or other characters that cannot be passed unquoted to the {tool} shell.\n"
        ),
    }
}

// Codex のフォルダが無ければ Codex は入っていないものとして、何も表示しない。
// 信頼の記録は config.toml にあるが、利用者が /hooks で確認する前提を崩さないよう触らない。
fn codex(mode: &Mode, opts: &Options, exe: &Path, lang: Lang) -> Result<(), String> {
    let Some(dir) = codex_home().filter(|d| d.is_dir()) else {
        return Ok(());
    };
    let path = dir.join("hooks.json");
    let mut out = String::from("\nCodex\n");
    let Some(command) = settings_edit::codex_hook_command(&exe.to_string_lossy(), cfg!(windows))
    else {
        out += &unquotable_path("Codex", "Codex", lang);
        print(&out);
        return Ok(());
    };
    if opts.dry_run {
        out += dry_run_note(lang);
    }
    out += &settings_file_line(&path, lang);
    match mode {
        Mode::Install => {
            let original = read_settings(&path, true, lang)?;
            let mut file = original
                .as_ref()
                .map_or_else(|| Value::Object(Default::default()), |s| s.value.clone());
            let report = settings_edit::install_codex(&mut file, &command, lang)?;
            out += &hooks_line(Label::Add, &report.added, lang);
            if !report.already.is_empty() {
                out += &hooks_line(Label::Already, &report.already, lang);
            }
            if report.added.is_empty() {
                out += already_installed(lang);
            } else {
                out += &commit(
                    &path,
                    opts.dry_run,
                    original.as_ref().map(|s| s.bytes.as_slice()),
                    &file,
                    lang,
                )?;
            }
            // 信頼していないフックを Codex は実行しないので、入れ直して確かめる利用者にも毎回伝える。
            out += codex_trust_note(lang);
        }
        Mode::Uninstall => match read_settings(&path, false, lang)? {
            None => out += &nothing_to_remove("hooks.json", lang),
            Some(original) => {
                let mut file = original.value.clone();
                let removed = settings_edit::uninstall_codex(&mut file, lang)?;
                out += &hooks_line(Label::Remove, &removed, lang);
                if removed.is_empty() {
                    out += not_registered(lang);
                } else {
                    out += &commit(&path, opts.dry_run, Some(&original.bytes), &file, lang)?;
                }
            }
        },
    }
    print(&out);
    Ok(())
}

fn opencode_reload_note(lang: Lang) -> &'static str {
    lang.pick(
        "OpenCode は起動したときにプラグインを読み込みます。動いている OpenCode は、バックグラウンドのサービスも含めて起動し直してください（サービスは opencode service restart で起動し直せます）。\n",
        "OpenCode loads plugins when it starts. Restart any running OpenCode, including the background service (restart the service with opencode service restart).\n",
    )
}

// OpenCode の設定のフォルダが無ければ OpenCode は入っていないものとして、何も表示しない。
// 設定ファイル（opencode.jsonc）は書き換えず、OpenCode が自動で読み込む plugins/ にプラグインを一つ置く。
// 同じ名前のファイルが marimo の書いたものでなければ、書き換えも削除もしない。
fn opencode(mode: &Mode, opts: &Options, exe: &Path, lang: Lang) -> Result<(), String> {
    let Some(dir) = opencode_plugin::config_dir().filter(|d| d.is_dir()) else {
        return Ok(());
    };
    let path = dir.join("plugins").join(opencode_plugin::FILE_NAME);
    let mut out = String::from("\nOpenCode\n");
    if opts.dry_run {
        out += dry_run_note(lang);
    }
    out += &match lang {
        Lang::Ja => format!("プラグイン: {}\n", path.display()),
        Lang::En => format!("Plugin: {}\n", path.display()),
    };
    // dotfiles の管理でリンクになっていれば、settings.json と同じくリンク先を読み書きする。
    let target = resolve_symlink(&path);
    let existing = match fs::read(&target) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(cannot_read(&target, &e, lang)),
    };
    let foreign = existing
        .as_deref()
        .is_some_and(|b| !std::str::from_utf8(b).is_ok_and(opencode_plugin::is_ours));
    if foreign {
        out += lang.pick(
            "marimo が書いたものではない同じ名前のファイルがあるので、触れていません。\n",
            "Left untouched: a file with the same name exists that marimo did not write.\n",
        );
        print(&out);
        return Ok(());
    }
    match mode {
        Mode::Install => {
            let text = opencode_plugin::render(&exe.to_string_lossy());
            if existing.as_deref() == Some(text.as_bytes()) {
                out += already_installed(lang);
            } else if opts.dry_run {
                out += lang.pick(
                    "プラグインを書き込みます。\n",
                    "The plugin will be written.\n",
                );
            } else {
                let plugins = target.parent().unwrap_or(Path::new("."));
                fs::create_dir_all(plugins).map_err(|e| cannot_create(plugins, &e, lang))?;
                write_replacing(&target, text.as_bytes(), lang)?;
                let reread = fs::read(&target).map_err(|e| match lang {
                    Lang::Ja => format!("書き込んだプラグインを読み直せません: {e}"),
                    Lang::En => format!("Cannot read back the written plugin: {e}"),
                })?;
                if reread != text.as_bytes() {
                    return Err(match lang {
                        Lang::Ja => {
                            format!("{} の内容が書き込んだものと違います", target.display())
                        }
                        Lang::En => format!(
                            "The contents of {} differ from what was written",
                            target.display()
                        ),
                    });
                }
                out += lang.pick("プラグインを書き込みました。\n", "Wrote the plugin.\n");
                out += opencode_reload_note(lang);
            }
        }
        Mode::Uninstall => {
            if existing.is_none() {
                out += lang.pick(
                    "プラグインがないので、取り除くものはありません。\n",
                    "The plugin does not exist, so there is nothing to remove.\n",
                );
            } else if opts.dry_run {
                out += lang.pick(
                    "プラグインを取り除きます。\n",
                    "The plugin will be removed.\n",
                );
            } else {
                fs::remove_file(&path).map_err(|e| match lang {
                    Lang::Ja => format!("{} を消せません: {e}", path.display()),
                    Lang::En => format!("Cannot delete {}: {e}", path.display()),
                })?;
                out += lang.pick("プラグインを取り除きました。\n", "Removed the plugin.\n");
                out += opencode_reload_note(lang);
            }
        }
    }
    print(&out);
    Ok(())
}

fn antigravity_reload_note(lang: Lang) -> &'static str {
    lang.pick(
        "フックを読み込ませるため、動いている agy の CLI と、Gemini のデスクトップアプリを起動し直してください。\n",
        "Restart any running agy CLI and the Gemini desktop app so they load the hooks.\n",
    )
}

// agy の共通の設定のフォルダ（~/.gemini/config）が無ければ Antigravity は入っていないものとして、何も表示しない。
// 利用者ごとの hooks.json がこのフォルダにあることは、agy の変更履歴と、agy の実行ファイルに書かれた既定の
// パス（~/.gemini/config/hooks.json）による。
fn antigravity(mode: &Mode, opts: &Options, exe: &Path, lang: Lang) -> Result<(), String> {
    let Some(dir) = user_home()
        .map(|h| h.join(".gemini").join("config"))
        .filter(|d| d.is_dir())
    else {
        return Ok(());
    };
    let path = dir.join("hooks.json");
    let mut out = String::from("\nAntigravity\n");
    let Some(command) =
        settings_edit::antigravity_hook_command(&exe.to_string_lossy(), cfg!(windows))
    else {
        out += &unquotable_path("agy", "Antigravity", lang);
        print(&out);
        return Ok(());
    };
    if opts.dry_run {
        out += dry_run_note(lang);
    }
    out += &settings_file_line(&path, lang);
    match mode {
        Mode::Install => {
            let original = read_settings(&path, true, lang)?;
            let mut file = original
                .as_ref()
                .map_or_else(|| Value::Object(Default::default()), |s| s.value.clone());
            if settings_edit::install_antigravity(&mut file, &command, lang)? {
                out += &hooks_line(Label::Add, &[settings_edit::ANTIGRAVITY_HOOK_NAME], lang);
                out += &commit(
                    &path,
                    opts.dry_run,
                    original.as_ref().map(|s| s.bytes.as_slice()),
                    &file,
                    lang,
                )?;
                out += antigravity_reload_note(lang);
            } else {
                out += already_installed(lang);
            }
        }
        Mode::Uninstall => match read_settings(&path, false, lang)? {
            None => out += &nothing_to_remove("hooks.json", lang),
            Some(original) => {
                let mut file = original.value.clone();
                let removed = settings_edit::uninstall_antigravity(&mut file, lang)?;
                out += &hooks_line(Label::Remove, &removed, lang);
                if removed.is_empty() {
                    out += not_registered(lang);
                } else {
                    out += &commit(&path, opts.dry_run, Some(&original.bytes), &file, lang)?;
                }
            }
        },
    }
    print(&out);
    Ok(())
}

fn parse_args(args: &[OsString], lang: Lang) -> Result<Options, String> {
    let mut settings: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.to_str() {
            Some("--dry-run") => dry_run = true,
            Some("--settings") => {
                let path = it.next().ok_or(lang.pick(
                    "--settings の後に settings.json のパスが必要です",
                    "--settings needs the path to settings.json",
                ))?;
                settings = Some(PathBuf::from(path));
            }
            _ => {
                let arg = arg.to_string_lossy();
                return Err(match lang {
                    Lang::Ja => format!("不明な引数です: {arg}"),
                    Lang::En => format!("Unknown argument: {arg}"),
                });
            }
        }
    }
    let explicit_settings = settings.is_some();
    let settings = match settings {
        Some(p) => p,
        None => default_settings_path(lang)?,
    };
    Ok(Options {
        settings,
        explicit_settings,
        dry_run,
    })
}

// CLAUDE_CONFIG_DIR は設定を置くフォルダを ~/.claude から差し替える。
// https://code.claude.com/docs/en/env-vars と https://code.claude.com/docs/en/settings に記載がある。
fn default_settings_path(lang: Lang) -> Result<PathBuf, String> {
    if let Some(dir) = env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("settings.json"));
    }
    user_home()
        .map(|h| h.join(".claude").join("settings.json"))
        .ok_or_else(|| {
            lang.pick(
                "ホームフォルダが分からないので、--settings で指定してください",
                "Cannot find the home folder; specify the settings file with --settings",
            )
            .to_owned()
        })
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

// 実行ファイルを置き、表示する一行を返す。
fn copy_executable(opts: &Options, exe: &Path, lang: Lang) -> Result<String, String> {
    let not_copied = lang.pick(
        "すでにこの場所から実行しているのでコピーしません",
        "not copied because it is already running from here",
    );
    let note = if opts.dry_run {
        if running_from(exe) {
            not_copied
        } else {
            lang.pick("コピーします", "will be copied")
        }
    } else if copy_self(exe, lang)? {
        lang.pick("コピーしました", "copied")
    } else {
        not_copied
    };
    Ok(match lang {
        Lang::Ja => format!("実行ファイル: {}（{note}）\n", exe.display()),
        Lang::En => format!("Executable: {} ({note})\n", exe.display()),
    })
}

fn install(opts: &Options, marimo: &Marimo, lang: Lang) -> Result<(), String> {
    // --settings で指定していないのに設定のフォルダが無ければ、Claude Code は入っていないものとして飛ばす。
    if !opts.explicit_settings
        && let Some(dir) = opts.settings.parent().filter(|d| !d.is_dir())
    {
        print(&match lang {
            Lang::Ja => format!(
                "Claude Code の設定フォルダ {} が無いので、Claude Code のフックは登録しませんでした。\n",
                dir.display()
            ),
            Lang::En => format!(
                "Skipped the Claude Code hooks because the Claude Code settings folder {} does not exist.\n",
                dir.display()
            ),
        });
        return Ok(());
    }
    let original = read_settings(&opts.settings, true, lang)?;
    let mut settings = original
        .as_ref()
        .map_or_else(|| Value::Object(Default::default()), |s| s.value.clone());
    let report = settings_edit::install(&mut settings, marimo, lang)?;
    let mut out = settings_file_line(&opts.settings, lang);
    if opts.dry_run && !report.added.is_empty() {
        // 設定ファイルの全体は env のトークンなどを含みうるので、marimo が足すハンドラだけを見せる。
        out += &match lang {
            Lang::Ja => format!("追加するハンドラ: {}\n", marimo.hook_handler()),
            Lang::En => format!("Handler to add: {}\n", marimo.hook_handler()),
        };
    }
    out += &hooks_line(Label::Add, &report.added, lang);
    if !report.migrated.is_empty() {
        out += &hooks_line(Label::Migrate, &report.migrated, lang);
    }
    if !report.already.is_empty() {
        out += &hooks_line(Label::Already, &report.already, lang);
    }
    out += &describe_status_line(&report.status_line, lang);

    if report.changed() {
        out += &commit(
            &opts.settings,
            opts.dry_run,
            original.as_ref().map(|s| s.bytes.as_slice()),
            &settings,
            lang,
        )?;
    } else {
        out += already_installed(lang);
    }
    print(&out);
    Ok(())
}

fn uninstall(opts: &Options, marimo: &Marimo, lang: Lang) -> Result<(), String> {
    let Some(original) = read_settings(&opts.settings, false, lang)? else {
        print(&nothing_to_remove(
            &opts.settings.display().to_string(),
            lang,
        ));
        return Ok(());
    };
    let mut settings = original.value.clone();
    let report = settings_edit::uninstall(&mut settings, marimo, lang)?;
    let mut out = String::new();
    if opts.dry_run {
        out += dry_run_note(lang);
    }
    out += &settings_file_line(&opts.settings, lang);
    out += &hooks_line(Label::Remove, &report.removed, lang);
    out += &describe_status_line(&report.status_line, lang);
    if report.changed() {
        out += &commit(
            &opts.settings,
            opts.dry_run,
            Some(&original.bytes),
            &settings,
            lang,
        )?;
    } else {
        out += not_registered(lang);
    }
    print(&out);
    Ok(())
}

// original は読み込んだときのファイルの内容で、ファイルがなかったときは None になる。
fn commit(
    path: &Path,
    dry_run: bool,
    original: Option<&[u8]>,
    settings: &Value,
    lang: Lang,
) -> Result<String, String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    // dry-run では内容を表示しない。設定ファイルは env のトークンなどを含みうるので、変わる箇所は
    // 呼び出し側がツールごとに表示する。
    if dry_run {
        return Ok(match lang {
            Lang::Ja => format!("{name} を書き換えます。\n"),
            Lang::En => format!("{name} will be updated.\n"),
        });
    }
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    let target = resolve_symlink(path);
    ensure_unchanged(&target, original, lang)?;
    let mut out = String::new();
    if original.is_some() {
        let backup = backup(&target, lang)?;
        out += &match lang {
            Lang::Ja => format!("バックアップ: {}\n", backup.display()),
            Lang::En => format!("Backup: {}\n", backup.display()),
        };
    }
    write_replacing(&target, text.as_bytes(), lang)?;
    let reread = fs::read(&target).map_err(|e| match lang {
        Lang::Ja => format!("書き込んだ設定を読み直せません: {e}"),
        Lang::En => format!("Cannot read back the written settings: {e}"),
    })?;
    serde_json::from_slice::<Value>(&reread).map_err(|e| match lang {
        Lang::Ja => {
            format!("書き込んだ設定が JSON として読めません（バックアップから戻してください）: {e}")
        }
        Lang::En => {
            format!("The written settings are not valid JSON (restore them from the backup): {e}")
        }
    })?;
    out += &match lang {
        Lang::Ja => format!("{name} を書き換えました。\n"),
        Lang::En => format!("Updated {name}.\n"),
    };
    Ok(out)
}

struct Loaded {
    bytes: Vec<u8>,
    value: Value,
}

fn read_settings(path: &Path, allow_missing: bool, lang: Lang) -> Result<Option<Loaded>, String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound && allow_missing => {
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
            if parent.is_some_and(|p| !p.is_dir()) {
                return Err(match lang {
                    Lang::Ja => format!("{} のフォルダがありません", path.display()),
                    Lang::En => format!("The folder for {} does not exist", path.display()),
                });
            }
            return Ok(None);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(cannot_read(path, &e, lang)),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| match lang {
        Lang::Ja => format!(
            "{} が JSON として読めないので、このファイルは変更していません: {e}",
            path.display()
        ),
        Lang::En => format!(
            "{} is not valid JSON, so this file was left unchanged: {e}",
            path.display()
        ),
    })?;
    if !value.is_object() {
        return Err(match lang {
            Lang::Ja => format!(
                "{} の最上位が JSON のオブジェクトではないので、このファイルは変更していません",
                path.display()
            ),
            Lang::En => format!(
                "The top level of {} is not a JSON object, so this file was left unchanged",
                path.display()
            ),
        });
    }
    Ok(Some(Loaded { bytes, value }))
}

fn cannot_read(path: &Path, e: &io::Error, lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("{} を読めません: {e}", path.display()),
        Lang::En => format!("Cannot read {}: {e}", path.display()),
    }
}

fn cannot_create(path: &Path, e: &io::Error, lang: Lang) -> String {
    match lang {
        Lang::Ja => format!("{} を作れません: {e}", path.display()),
        Lang::En => format!("Cannot create {}: {e}", path.display()),
    }
}

// 読み込んでから書き戻すまでの間に Claude Code やエディタが settings.json を書き換えていたら、
// その変更を上書きで失わないよう、何も書かずに止めてやり直してもらう。
fn ensure_unchanged(path: &Path, original: Option<&[u8]>, lang: Lang) -> Result<(), String> {
    let current = match fs::read(path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(match lang {
                Lang::Ja => format!(
                    "{} を読み直せないので、何も変更していません: {e}",
                    path.display()
                ),
                Lang::En => format!(
                    "Cannot read {} again, so nothing was changed: {e}",
                    path.display()
                ),
            });
        }
    };
    if current.as_deref() == original {
        Ok(())
    } else {
        Err(match lang {
            Lang::Ja => format!(
                "{} が処理の途中で書き換えられたので、何も変更していません。もう一度実行してください",
                path.display()
            ),
            Lang::En => format!(
                "{} was modified by another program in the meantime, so nothing was changed. Run the command again",
                path.display()
            ),
        })
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

fn backup(path: &Path, lang: Lang) -> Result<PathBuf, String> {
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
    fs::copy(path, &candidate).map_err(|e| match lang {
        Lang::Ja => format!("バックアップを作れないので、何も変更していません: {e}"),
        Lang::En => format!("Cannot create a backup, so nothing was changed: {e}"),
    })?;
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

fn write_replacing(path: &Path, bytes: &[u8], lang: Lang) -> Result<(), String> {
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
        match lang {
            Lang::Ja => format!("{} を書き込めません: {e}", path.display()),
            Lang::En => format!("Cannot write {}: {e}", path.display()),
        }
    })
}

fn running_from(dest: &Path) -> bool {
    let current = env::current_exe().and_then(fs::canonicalize);
    matches!((current, fs::canonicalize(dest)), (Ok(a), Ok(b)) if a == b)
}

fn copy_self(dest: &Path, lang: Lang) -> Result<bool, String> {
    let current = env::current_exe().map_err(|e| match lang {
        Lang::Ja => format!("自分の実行ファイルの場所が分かりません: {e}"),
        Lang::En => format!("Cannot find the location of this executable: {e}"),
    })?;
    if running_from(dest) {
        return Ok(false);
    }
    let dir = dest.parent().ok_or(lang.pick(
        "実行ファイルの置き場所が不正です",
        "The location for the executable is invalid",
    ))?;
    store::create_private_dir_all(dir).map_err(|e| cannot_create(dir, &e, lang))?;
    // 実行中の古いバイナリを上書きせず、別名で書いてから rename して差し替える。
    // macOS は同じ inode への上書きで署名の検証に失敗し、実行中のプロセスを落とすことがある。
    remove_set_aside(dir);
    let tmp = dir.join(format!(".marimo-hook.{}.tmp", std::process::id()));
    fs::copy(&current, &tmp)
        .and_then(|_| replace_executable(&tmp, dest))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            match lang {
                Lang::Ja => format!(
                    "{} へコピーできません: {e}。少し待ってから、もう一度実行してください",
                    dest.display()
                ),
                Lang::En => format!(
                    "Cannot copy to {}: {e}. Wait a moment and run the command again",
                    dest.display()
                ),
            }
        })?;
    Ok(true)
}

const SET_ASIDE_PREFIX: &str = ".marimo-hook.old.";

// Windows では、動いている実行ファイルを置き換え先にした rename は失敗するが、動いている実行ファイルを別名へ
// 移すことはできる。statusLine を通す marimo-hook は Claude Code のセッションが開いている間ほぼ常に動いて
// いるので、先に今のものを退避してから新しいものを置く。新しいものを置けなければ退避したものを戻す。
#[cfg(windows)]
fn replace_executable(new: &Path, dest: &Path) -> io::Result<()> {
    if !dest.exists() {
        return fs::rename(new, dest);
    }
    let dir = dest.parent().unwrap_or(Path::new("."));
    let aside = dir.join(format!("{SET_ASIDE_PREFIX}{}.exe", std::process::id()));
    let _ = fs::remove_file(&aside);
    fs::rename(dest, &aside)?;
    if let Err(e) = fs::rename(new, dest) {
        let _ = fs::rename(&aside, dest);
        return Err(e);
    }
    // まだ動いていれば消せないので、次の install で消す。
    let _ = fs::remove_file(&aside);
    Ok(())
}

#[cfg(not(windows))]
fn replace_executable(new: &Path, dest: &Path) -> io::Result<()> {
    fs::rename(new, dest)
}

// 前の install が退避した実行ファイルのうち、もう動いていないものを消す。動いているものは消せないので残す。
fn remove_set_aside(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(SET_ASIDE_PREFIX))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn describe_status_line(change: &StatusLineChange, lang: Lang) -> String {
    let (before, after, current) =
        lang.pick(("変更前", "変更後", "現在"), ("Before", "After", "Current"));
    match change {
        StatusLineChange::Added { after: a } => {
            let head = lang.pick("未設定だったので登録します", "Not set, so it will be added");
            format!("statusLine: {head}\n  {after}: {a}\n")
        }
        StatusLineChange::Wrapped {
            before: b,
            after: a,
        } => {
            let head = lang.pick(
                "元のコマンドをラップします",
                "The original command will be wrapped",
            );
            format!("statusLine: {head}\n  {before}: {b}\n  {after}: {a}\n")
        }
        StatusLineChange::Replaced {
            before: b,
            after: a,
        } => {
            let head = lang.pick(
                "別の場所の marimo-hook や二重のラップを、今の marimo-hook の一重のラップに書き換えます",
                "The wrapping by a marimo-hook at another location, or a double wrapping, will be replaced with a single wrapping by this marimo-hook",
            );
            format!("statusLine: {head}\n  {before}: {b}\n  {after}: {a}\n")
        }
        StatusLineChange::AlreadyMarimo { current: c } => {
            let head = lang.pick(
                "すでに marimo を通っています",
                "Already runs through marimo",
            );
            format!("statusLine: {head}\n  {current}: {c}\n")
        }
        StatusLineChange::Removed { before: b } => {
            let head = lang.pick(
                "marimo だけだったので取り除きます",
                "Only marimo was set, so it will be removed",
            );
            format!("statusLine: {head}\n  {before}: {b}\n")
        }
        StatusLineChange::Restored {
            before: b,
            after: a,
        } => {
            let head = lang.pick(
                "元のコマンドへ戻します",
                "The original command will be restored",
            );
            format!("statusLine: {head}\n  {before}: {b}\n  {after}: {a}\n")
        }
        StatusLineChange::Untouched { reason } => format!("statusLine: {reason}\n"),
    }
}

fn list_or_none<S: AsRef<str>>(items: &[S], lang: Lang) -> String {
    if items.is_empty() {
        lang.pick("なし", "none").to_owned()
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
        assert!(commit(&settings, false, Some(b"{}"), &edited, Lang::Ja).is_err());
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "{\"model\": \"sonnet\"}"
        );

        // 読んだときはなかったファイルが、書く前に作られていた場合も止める。
        let fresh = dir.path().join("fresh.json");
        fs::write(&fresh, "{}").unwrap();
        assert!(commit(&fresh, false, None, &edited, Lang::Ja).is_err());
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
            write_replacing(&path, b"{\"a\": 1}\n", Lang::Ja).unwrap();
            assert_eq!(mode(&path), original);
            assert_eq!(fs::read(&path).unwrap(), b"{\"a\": 1}\n");
        }

        let fresh = dir.path().join("fresh.json");
        // 前の実行が残した一時ファイルがあっても書き込める。
        let stale = dir
            .path()
            .join(format!(".fresh.json.marimo-{}.tmp", std::process::id()));
        fs::write(&stale, "stale").unwrap();
        write_replacing(&fresh, b"{}\n", Lang::Ja).unwrap();
        assert_eq!(mode(&fresh), 0o600);
        assert!(!stale.exists());
    }
}
