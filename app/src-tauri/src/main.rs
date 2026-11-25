// リリースビルドの Windows でコンソールウィンドウを開かないようにする。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod acknowledged;
mod dialogue;
mod focus;
mod hit;
mod icons;
mod limits;
mod scale;
mod settings;
mod shortcuts;
mod tray;
mod update;
mod watch;
mod window_pos;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use marimo_core::{Lang, LangSetting, MarimoHome, Provider, Snapshot, store};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    home: MarimoHome,
    hits: Arc<hit::HitState>,
    icons: OnceLock<icons::AppIcons>,
    // 立ち絵の縦横比は選んだキャラクターの manifest にあり、読むのはフロントエンドなので、
    // 知らされるまでは前回保存した比を使う。
    aspect: Mutex<f64>,
    // 立ち絵の倍率。display.json へは scale_saver が遅れて書くので、ウィンドウの配置にはこちらを使う。
    scale: Mutex<f64>,
    scale_saver: mpsc::Sender<f64>,
    // 立ち絵を表示しているかとパネルの倍率。非表示のときはパネル幅へウィンドウを詰める。
    frame: Mutex<scale::Frame>,
    tray: OnceLock<tray::Tray>,
    // フロントエンドが既読にしたトリガー。メニューバーのアイコンを決めるのに使う。
    acknowledged: Mutex<HashSet<String>>,
    // 立ち絵を置いているウィンドウの隅。ウィンドウを置いた場所で Rust が決め、フロントエンドは配置を合わせる。
    corner: Mutex<window_pos::Corner>,
    // 起動時と設定を変えたときに決める。OS の言語を起動中に変えても、次の起動まで反映しない。
    language: Mutex<LanguageState>,
    usage_refresh: Arc<limits::UsageRefresh>,
    shortcuts: shortcuts::ShortcutStateStore,
}

/// 設定で選んだものと、それを OS の言語で解決した結果。フロントエンドの i18n.ts の LanguageState と同じ形。
#[derive(Debug, Clone, Copy, Serialize)]
struct LanguageState {
    setting: LangSetting,
    lang: Lang,
}

impl LanguageState {
    fn load(home: &MarimoHome) -> Self {
        let setting = LangSetting::load(home);
        Self {
            setting,
            lang: setting.resolve(),
        }
    }
}

impl AppState {
    fn aspect(&self) -> f64 {
        *self.aspect.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn scale(&self) -> f64 {
        *self.scale.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn frame(&self) -> scale::Frame {
        *self.frame.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn corner(&self) -> window_pos::Corner {
        *self.corner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lang(&self) -> Lang {
        self.language.lock().unwrap_or_else(|e| e.into_inner()).lang
    }
}

/// 画面の文言を出す Rust の処理が、AppHandle から今の言語を引く。
pub fn lang(app: &AppHandle) -> Lang {
    app.try_state::<AppState>()
        .map_or_else(Lang::os, |s| s.lang())
}

const LANGUAGE_EVENT: &str = "language";

const LAYOUT_EVENT: &str = "layout";

// 大きさを変えるのはいつも立ち絵のウィンドウなので、呼び出し元のウィンドウではなくラベルで取る。
fn resize(app: &AppHandle, scale: f64, aspect: f64) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<AppState>();
    let (w, h) = scale::window_size_for(scale, aspect, state.frame());
    window_pos::resize_keeping_corner(&window, (w, h), state.corner());
}

/// 立ち絵の有無やパネルの倍率が変わったときだけ、ウィンドウの大きさを合わせ直す。
pub(crate) fn sync_window_frame(app: &AppHandle, frame: scale::Frame) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let mut current = state.frame.lock().unwrap_or_else(|e| e.into_inner());
    if *current == frame {
        return;
    }
    *current = frame;
    drop(current);
    resize(app, state.scale(), state.aspect());
}

pub(crate) fn reset_position(app: &AppHandle) {
    let (Some(window), Some(state)) = (app.get_webview_window("main"), app.try_state::<AppState>())
    else {
        return;
    };
    let size = scale::window_size_for(state.scale(), state.aspect(), state.frame());
    let corner = window_pos::reset(&window, &state.home, size);
    *state.corner.lock().unwrap_or_else(|e| e.into_inner()) = corner;
    if let Err(e) = app.emit(LAYOUT_EVENT, corner) {
        eprintln!("marimo: cannot send the layout: {e}");
    }
    tray::set_visible(app, true);
}

// ウィンドウを置き終えた位置に合わせて立ち絵の隅を選び直し、変わったらフロントエンドに配置を切り替えさせる。
fn relayout(app: &AppHandle, pos: (i32, i32)) -> ((i32, i32), window_pos::Corner) {
    let state = app.state::<AppState>();
    let current = state.corner();
    let Some(window) = app.get_webview_window("main") else {
        return (pos, current);
    };
    let (scale, aspect, frame) = (state.scale(), state.aspect(), state.frame());
    let Some(size) = window_pos::Screen::outer(&window) else {
        return (pos, current);
    };
    let rect = window_pos::Rect {
        x: pos.0,
        y: pos.1,
        ..size
    };
    let Some((next, moved)) = window_pos::settle(&window, rect, current, |c| {
        scale::stage_rect_for(scale, aspect, c, frame)
    }) else {
        return (pos, current);
    };
    *state.corner.lock().unwrap_or_else(|e| e.into_inner()) = next;
    if let Err(e) = app.emit(LAYOUT_EVENT, next) {
        eprintln!("marimo: cannot send the layout: {e}");
    }
    (moved, next)
}

#[tauri::command]
fn get_layout(state: State<'_, AppState>) -> window_pos::Corner {
    state.corner()
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Snapshot {
    store::load_snapshot(&state.home)
}

#[tauri::command]
fn get_dialogue(state: State<'_, AppState>) -> Value {
    dialogue::user_overrides(&state.home)
}

#[tauri::command]
fn get_scale(state: State<'_, AppState>) -> f64 {
    state.scale()
}

// 同期コマンドは Tauri のメインスレッドで順に実行されるので、ホイールで続けて
// 呼ばれてもウィンドウの大きさの変更が入れ違うことはない。保存は scale::spawn_saver のスレッドに任せる。
#[tauri::command]
fn set_scale(app: AppHandle, state: State<'_, AppState>, scale: f64) -> f64 {
    let scale = scale::clamp(scale);
    *state.scale.lock().unwrap_or_else(|e| e.into_inner()) = scale;
    resize(&app, scale, state.aspect());
    let _ = state.scale_saver.send(scale);
    scale
}

#[tauri::command]
fn get_panel_display(state: State<'_, AppState>) -> Option<scale::PanelDisplay> {
    scale::load_panel_display(&state.home)
}

#[tauri::command]
fn set_panel_display(
    app: AppHandle,
    state: State<'_, AppState>,
    display: scale::PanelDisplay,
) -> Result<(), String> {
    scale::save_panel_display(&state.home, display).map_err(|e| e.to_string())?;
    sync_window_frame(&app, display.frame());
    // 右クリックメニューやパネルの切り替えや設定のウィンドウで変えた表示を、トレイのメニューのチェックにも揃える。
    // トレイで選んだ表示はトレイ自身が保存してチェックを付けるので、ここは通らない。
    if let Some(tray) = state.tray.get() {
        tray.reflect(display);
    }
    Ok(())
}

#[tauri::command]
fn reset_window_position(app: AppHandle) {
    reset_position(&app);
}

#[tauri::command]
fn get_language(state: State<'_, AppState>) -> LanguageState {
    *state.language.lock().unwrap_or_else(|e| e.into_inner())
}

/// トレイのメニューと設定のウィンドウのタイトルは Rust が持つのでここで付け直し、画面の文言は
/// 全部のウィンドウへ送るイベントで切り替えさせる。
#[tauri::command]
fn set_language(
    app: AppHandle,
    state: State<'_, AppState>,
    setting: LangSetting,
) -> Result<LanguageState, String> {
    scale::save_language(&state.home, setting).map_err(|e| e.to_string())?;
    let next = LanguageState {
        setting,
        lang: setting.resolve(),
    };
    *state.language.lock().unwrap_or_else(|e| e.into_inner()) = next;
    if let Some(tray) = state.tray.get() {
        tray.relabel(next.lang);
    }
    settings::retitle(&app, next.lang);
    if let Err(e) = app.emit(LANGUAGE_EVENT, next) {
        eprintln!("marimo: cannot announce the language: {e}");
    }
    Ok(next)
}

#[tauri::command]
fn get_bubble_settings(state: State<'_, AppState>) -> scale::BubbleSettings {
    scale::load_bubble_settings(&state.home)
}

#[tauri::command]
fn set_bubble_settings(
    state: State<'_, AppState>,
    settings: scale::BubbleSettings,
) -> Result<(), String> {
    scale::save_bubble_settings(&state.home, settings).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_usage_refresh_minutes(state: State<'_, AppState>) -> u32 {
    scale::load_usage_refresh_minutes(&state.home)
}

#[tauri::command]
fn set_usage_refresh_minutes(state: State<'_, AppState>, minutes: u32) -> Result<(), String> {
    scale::save_usage_refresh_minutes(&state.home, minutes).map_err(|e| e.to_string())?;
    state.usage_refresh.notify_settings_changed();
    Ok(())
}

#[tauri::command]
fn get_usage_tools(state: State<'_, AppState>) -> scale::UsageTools {
    scale::load_usage_tools(&state.home)
}

#[tauri::command]
fn set_usage_tools(state: State<'_, AppState>, tools: scale::UsageTools) -> Result<(), String> {
    scale::save_usage_tools(&state.home, tools).map_err(|e| e.to_string())?;
    state.usage_refresh.notify_settings_changed();
    Ok(())
}

#[tauri::command]
fn get_usage_status(state: State<'_, AppState>) -> limits::UsageStatus {
    state.usage_refresh.snapshot(&state.home)
}

#[tauri::command]
fn refresh_usage_now(state: State<'_, AppState>) {
    state.usage_refresh.refresh_now(&state.home);
}

#[tauri::command]
fn get_shortcuts(state: State<'_, AppState>) -> scale::Shortcuts {
    state.shortcuts.current()
}

#[tauri::command]
fn set_shortcuts(
    app: AppHandle,
    state: State<'_, AppState>,
    shortcuts: scale::Shortcuts,
) -> Result<scale::Shortcuts, String> {
    shortcuts::apply(&app, &state.home, shortcuts, state.lang())
}

#[tauri::command]
fn get_character(state: State<'_, AppState>) -> String {
    scale::load_character(&state.home)
}

#[tauri::command]
fn set_character(state: State<'_, AppState>, id: String) -> Result<(), String> {
    scale::save_character(&state.home, &id).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_stage_aspect(app: AppHandle, state: State<'_, AppState>, aspect: f64) {
    let aspect = scale::clamp_aspect(aspect);
    *state.aspect.lock().unwrap_or_else(|e| e.into_inner()) = aspect;
    resize(&app, state.scale(), aspect);
    if let Err(e) = scale::save_aspect(&state.home, aspect) {
        eprintln!("marimo: cannot save stage aspect: {e}");
    }
}

#[tauri::command]
fn get_acknowledged(state: State<'_, AppState>) -> Vec<String> {
    acknowledged::load(&state.home)
}

#[tauri::command]
fn set_acknowledged(
    app: AppHandle,
    state: State<'_, AppState>,
    keys: Vec<String>,
) -> Result<(), String> {
    *state.acknowledged.lock().unwrap_or_else(|e| e.into_inner()) = keys.iter().cloned().collect();
    tray::show_status(&app, &store::load_snapshot(&state.home));
    acknowledged::save(&state.home, &keys).map_err(|e| e.to_string())
}

#[tauri::command]
fn focus_session(state: State<'_, AppState>, provider: Provider, session_id: String) {
    if let Some(session) = store::read_session(&state.home, provider, &session_id) {
        focus::run(focus::plan(&session));
    }
}

#[tauri::command]
fn remove_session(
    state: State<'_, AppState>,
    provider: Provider,
    session_id: String,
) -> Result<bool, String> {
    store::delete_session(&state.home, provider, &session_id).map_err(|e| e.to_string())
}

// 同期コマンドはメインスレッドで動き、子プロセスを待つ間は画面が止まるので、待つのは別のスレッドで行う。
#[tauri::command]
async fn copy_text(text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || focus::copy_text(&text))
        .await
        .map_err(|e| format!("clipboard copy stopped unexpectedly: {e}"))?
}

// 同期コマンドはメインスレッドで動くので、macOS では AppKit で描くのに都合がよい。
// アイコンは起動している間に変わることがまず無いので、一度だけ読む。
#[tauri::command]
fn app_icons(state: State<'_, AppState>) -> icons::AppIcons {
    state.icons.get_or_init(icons::load).clone()
}

#[tauri::command]
fn set_hit_regions(state: State<'_, AppState>, regions: hit::HitRegions) {
    state.hits.set(regions);
}

#[tauri::command]
fn set_any_unread(state: State<'_, AppState>, any: bool) {
    if let Some(tray) = state.tray.get() {
        tray.set_any_unread(any);
    }
}

#[tauri::command]
fn set_tray_sessions(state: State<'_, AppState>, items: Vec<tray::TraySession>, more: usize) {
    if let Some(tray) = state.tray.get() {
        tray.set_sessions(&items, more);
    }
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// 自動起動の登録は、有効にした時点の実行ファイルのパスを指したまま残る。有効かどうかは
/// 登録の名前だけで判定されるので、別の場所の実行ファイルで有効にした登録が残っていても
/// メニューには有効と出て、ログイン時にはそちらが起動する。有効なら今の実行ファイルで
/// 登録し直して、パスを追従させる。ソースツリーのビルド用フォルダから起動したときは、
/// 試しに動かしただけで登録を奪わないよう何もしない。
fn follow_autostart_path(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    if update::in_source_tree() {
        return;
    }
    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().unwrap_or(false)
        && let Err(e) = autolaunch.enable()
    {
        eprintln!("marimo: cannot update the autostart entry: {e}");
    }
}

fn main() {
    let home =
        MarimoHome::resolve().expect("cannot resolve the marimo home directory; set MARIMO_HOME");
    let _ = store::create_private_dir_all(&home.sessions_dir());
    if let Err(e) = dialogue::retire_shipped_default(&home) {
        eprintln!("marimo: cannot set aside the old default dialogue: {e}");
    }
    let context = tauri::generate_context!();
    let hits = Arc::new(hit::HitState::default());
    let aspect = scale::load_aspect(&home);
    let initial_scale = scale::load(&home);
    let initial_display = scale::load_panel_display(&home).unwrap_or_default();
    let usage_refresh = Arc::new(limits::UsageRefresh::default());

    // LaunchAgent は System Events への自動操作の許可を求めずに登録できる。
    // plist の名前は既定だと製品名の marimo になり、同名の別アプリと重なりうるので
    // bundle identifier を使う。
    let autostart = tauri_plugin_autostart::Builder::new();
    #[cfg(target_os = "macos")]
    let autostart = autostart
        .macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent)
        .app_name(context.config().identifier.clone());

    tauri::Builder::default()
        // 二つ目の起動は、他のプラグインが動き出す前に止める必要があるので最初に登録する。
        // 二つ目のプロセスはそのまま終わり、一つ目はトレイから隠していたウィンドウを出し直す。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::set_visible(app, true);
        }))
        .plugin(autostart.build())
        .plugin(shortcuts::plugin())
        .manage(AppState {
            home: home.clone(),
            hits: hits.clone(),
            icons: OnceLock::new(),
            aspect: Mutex::new(aspect),
            scale: Mutex::new(initial_scale),
            scale_saver: scale::spawn_saver(home.clone()),
            frame: Mutex::new(initial_display.frame()),
            tray: OnceLock::new(),
            acknowledged: Mutex::new(acknowledged::load(&home).into_iter().collect()),
            corner: Mutex::new(window_pos::Corner::default()),
            language: Mutex::new(LanguageState::load(&home)),
            usage_refresh: usage_refresh.clone(),
            shortcuts: shortcuts::ShortcutStateStore::default(),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_dialogue,
            get_scale,
            set_scale,
            get_layout,
            get_panel_display,
            set_panel_display,
            reset_window_position,
            get_language,
            set_language,
            get_bubble_settings,
            set_bubble_settings,
            get_usage_refresh_minutes,
            set_usage_refresh_minutes,
            get_usage_tools,
            set_usage_tools,
            get_usage_status,
            refresh_usage_now,
            get_shortcuts,
            set_shortcuts,
            get_character,
            set_character,
            set_stage_aspect,
            get_acknowledged,
            set_acknowledged,
            set_any_unread,
            set_tray_sessions,
            focus_session,
            remove_session,
            copy_text,
            settings::open_settings,
            settings::close_settings,
            app_icons,
            set_hit_regions,
            update::update_status,
            update::check_update,
            update::install_update,
            update::open_release_notes,
            quit
        ])
        .setup(move |app| {
            // .app では Info.plist の LSUIElement が起動の瞬間から Dock に出さない。これは
            // Info.plist を持たない開発中の実行ファイルでも Dock と Cmd+Tab から外すためにある。
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let window = app
                .get_webview_window("main")
                .expect("main window is declared in tauri.conf.json");
            let (w, h) = scale::window_size_for(initial_scale, aspect, initial_display.frame());
            let corner = window_pos::start(&window, &home, (w, h), |c| {
                scale::stage_rect_for(initial_scale, aspect, c, initial_display.frame())
            });
            *app.state::<AppState>()
                .corner
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = corner;
            window.show()?;
            let handle = app.handle().clone();
            window_pos::track(&window, home.clone(), move |pos| relayout(&handle, pos));
            let lang = app.state::<AppState>().lang();
            let tray = tray::build(app.handle(), initial_display, lang)?;
            let _ = app.state::<AppState>().tray.set(tray);
            tray::show_status(app.handle(), &store::load_snapshot(&home));
            shortcuts::init(app.handle(), &home);
            watch::spawn(app.handle().clone(), home.clone());
            watch::spawn_pruner(home.clone());
            watch::spawn_antigravity_steps(home.clone());
            limits::spawn_codex(app.handle().clone(), home.clone(), usage_refresh.clone());
            limits::spawn_claude(app.handle().clone(), home.clone(), usage_refresh.clone());
            limits::spawn_antigravity(app.handle().clone(), home.clone(), usage_refresh);
            hit::spawn(app.handle().clone(), window, hits.clone());
            follow_autostart_path(app.handle());
            update::spawn_checker(app.handle().clone());
            Ok(())
        })
        .build(context)
        .expect("error while building marimo")
        .run(|_app, _event| {
            // 起動している .app を Finder などから開き直すと、macOS は二つ目のプロセスを作らずに
            // 一つ目へ再度開く要求を送るので、single-instance の代わりにここでウィンドウを出し直す。
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                tray::set_visible(_app, true);
            }
        });
}
