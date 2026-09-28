// リリースビルドの Windows でコンソール窓を開かないようにする。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod acknowledged;
mod credentials;
mod dialogue;
mod focus;
mod hit;
mod icons;
mod scale;
mod tray;
mod usage;
mod watch;
mod window_pos;

use std::sync::{Arc, Mutex, OnceLock};

use marimo_core::{MarimoHome, Provider, Snapshot, store};
use serde_json::Value;
use tauri::{AppHandle, LogicalSize, Manager, State, WebviewWindow};

struct AppState {
    home: MarimoHome,
    hits: Arc<hit::HitState>,
    usage: usage::Poller,
    icons: OnceLock<icons::AppIcons>,
    // 立ち絵の縦横比は選んだキャラクターの manifest にあり、読むのはフロントエンドなので、
    // 知らされるまでは既定のキャラクターの比で窓を開く。
    aspect: Mutex<f64>,
    tray: OnceLock<tray::Tray>,
}

impl AppState {
    fn aspect(&self) -> f64 {
        *self.aspect.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn resize(window: &WebviewWindow, scale: f64, aspect: f64) {
    let (w, h) = scale::window_size(scale, aspect);
    if let Err(e) = window_pos::resize_keeping_bottom_right(window, LogicalSize::new(w, h)) {
        eprintln!("marimo: cannot resize window: {e}");
    }
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
    scale::load(&state.home)
}

// 同期コマンドは Tauri のメインスレッドで順に実行されるので、ホイールで続けて
// 呼ばれても窓の大きさの変更が入れ違うことはない。
#[tauri::command]
fn set_scale(window: WebviewWindow, state: State<'_, AppState>, scale: f64) -> f64 {
    let scale = scale::clamp(scale);
    resize(&window, scale, state.aspect());
    if let Err(e) = scale::save(&state.home, scale) {
        eprintln!("marimo: cannot save scale: {e}");
    }
    scale
}

#[tauri::command]
fn get_panel_display(state: State<'_, AppState>) -> Option<scale::PanelDisplay> {
    scale::load_panel_display(&state.home)
}

#[tauri::command]
fn set_panel_display(
    state: State<'_, AppState>,
    display: scale::PanelDisplay,
) -> Result<(), String> {
    scale::save_panel_display(&state.home, display).map_err(|e| e.to_string())?;
    // 表示はトレイのメニューからも、右クリックメニューやパネルの切り替えからも変わるので、
    // どれで変えても必ず通るここでトレイの印を付け直す。
    if let Some(tray) = state.tray.get() {
        tray.reflect(display);
    }
    Ok(())
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
fn set_stage_aspect(window: WebviewWindow, state: State<'_, AppState>, aspect: f64) {
    let aspect = scale::clamp_aspect(aspect);
    *state.aspect.lock().unwrap_or_else(|e| e.into_inner()) = aspect;
    resize(&window, scale::load(&state.home), aspect);
}

#[tauri::command]
fn get_usage_api(state: State<'_, AppState>) -> bool {
    scale::load_usage_api(&state.home)
}

#[tauri::command]
fn set_usage_api(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    scale::save_usage_api(&state.home, enabled).map_err(|e| e.to_string())?;
    state.usage.set_enabled(enabled);
    Ok(())
}

#[tauri::command]
fn get_acknowledged(state: State<'_, AppState>) -> Vec<String> {
    acknowledged::load(&state.home)
}

#[tauri::command]
fn set_acknowledged(state: State<'_, AppState>, keys: Vec<String>) -> Result<(), String> {
    acknowledged::save(&state.home, &keys).map_err(|e| e.to_string())
}

#[tauri::command]
fn focus_session(state: State<'_, AppState>, provider: Provider, session_id: String) {
    if let Some(session) = store::read_session(&state.home, provider, &session_id) {
        focus::run(focus::plan(&session));
    }
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
fn quit(app: AppHandle) {
    app.exit(0);
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
    let usage = usage::Poller::new(scale::load_usage_api(&home));

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
        // 二つ目のプロセスはそのまま終わり、一つ目はトレイから隠していた窓を出し直す。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::set_visible(app, true);
        }))
        .plugin(autostart.build())
        .manage(AppState {
            home: home.clone(),
            hits: hits.clone(),
            usage: usage.clone(),
            icons: OnceLock::new(),
            aspect: Mutex::new(scale::DEFAULT_ASPECT),
            tray: OnceLock::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_dialogue,
            get_scale,
            set_scale,
            get_panel_display,
            set_panel_display,
            get_character,
            set_character,
            set_stage_aspect,
            get_usage_api,
            set_usage_api,
            get_acknowledged,
            set_acknowledged,
            focus_session,
            app_icons,
            set_hit_regions,
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
            let (w, h) = scale::window_size(scale::load(&home), scale::DEFAULT_ASPECT);
            window_pos::restore(&window, &home, LogicalSize::new(w, h));
            window.show()?;
            window_pos::track(&window, home.clone());
            let display = scale::load_panel_display(&home).unwrap_or_default();
            let tray = tray::build(app.handle(), display)?;
            let _ = app.state::<AppState>().tray.set(tray);
            watch::spawn(app.handle().clone(), home.clone());
            watch::spawn_pruner(home.clone());
            usage.spawn(home.clone());
            hit::spawn(app.handle().clone(), window, hits.clone());
            Ok(())
        })
        .build(context)
        .expect("error while building marimo")
        .run(|_app, _event| {
            // 起動している .app を Finder などから開き直すと、macOS は二つ目のプロセスを作らずに
            // 一つ目へ再度開く要求を送るので、single-instance の代わりにここで窓を出し直す。
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                tray::set_visible(_app, true);
            }
        });
}
