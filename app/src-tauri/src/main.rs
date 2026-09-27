// リリースビルドの Windows でコンソール窓を開かないようにする。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod acknowledged;
mod credentials;
mod dialogue;
mod focus;
mod hit;
mod scale;
mod usage;
mod watch;
mod window_pos;

use std::fs;
use std::sync::Arc;

use marimo_core::{MarimoHome, Snapshot, store};
use serde_json::Value;
use tauri::{AppHandle, LogicalSize, Manager, State, WebviewWindow};

struct AppState {
    home: MarimoHome,
    hits: Arc<hit::HitState>,
    usage: usage::Poller,
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
    let (w, h) = scale::window_size(scale);
    if let Err(e) = window_pos::resize_keeping_bottom_right(&window, LogicalSize::new(w, h)) {
        eprintln!("marimo: cannot resize window: {e}");
    }
    if let Err(e) = scale::save(&state.home, scale) {
        eprintln!("marimo: cannot save scale: {e}");
    }
    scale
}

#[tauri::command]
fn get_panel_mode(state: State<'_, AppState>) -> Option<String> {
    scale::load_panel_mode(&state.home)
}

#[tauri::command]
fn set_panel_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    scale::save_panel_mode(&state.home, &mode).map_err(|e| e.to_string())
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
fn focus_session(state: State<'_, AppState>, session_id: String) {
    if let Some(session) = store::read_session(&state.home, &session_id) {
        focus::run(focus::plan(&session));
    }
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
    let _ = fs::create_dir_all(home.sessions_dir());
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
        // 何もしないコールバックにしてあり、二つ目のプロセスはそのまま終わる。
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .plugin(autostart.build())
        .manage(AppState {
            home: home.clone(),
            hits: hits.clone(),
            usage: usage.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_dialogue,
            get_scale,
            set_scale,
            get_panel_mode,
            set_panel_mode,
            get_usage_api,
            set_usage_api,
            get_acknowledged,
            set_acknowledged,
            focus_session,
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
            let (w, h) = scale::window_size(scale::load(&home));
            window_pos::restore(&window, &home, LogicalSize::new(w, h));
            window.show()?;
            window_pos::track(&window, home.clone());
            watch::spawn(app.handle().clone(), home.clone());
            usage.spawn(home.clone());
            hit::spawn(app.handle().clone(), window, hits.clone());
            Ok(())
        })
        .run(context)
        .expect("error while running marimo");
}
