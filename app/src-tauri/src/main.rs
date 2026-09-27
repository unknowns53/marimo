// リリースビルドの Windows でコンソール窓を開かないようにする。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod scale;
mod watch;
mod window_pos;

use std::fs;
use std::io;

use marimo_core::{MarimoHome, Snapshot, store};
use serde_json::Value;
use tauri::{AppHandle, LogicalSize, Manager, State, WebviewWindow};

const DEFAULT_DIALOGUE: &str = include_str!("../../../assets/character/default/dialogue.json");

struct AppState {
    home: MarimoHome,
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Snapshot {
    store::load_snapshot(&state.home)
}

// ユーザーが編集する `$MARIMO_HOME/dialogue.json` を優先する。無ければ既定のセリフを
// そこへ書き出し、編集の起点になるファイルを用意しておく。
#[tauri::command]
fn get_dialogue(state: State<'_, AppState>) -> Value {
    let path = state.home.dialogue_file();
    match fs::read(&path) {
        Ok(bytes) => {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                return value;
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let _ = store::write_atomic(&path, DEFAULT_DIALOGUE.as_bytes());
        }
        Err(_) => {}
    }
    serde_json::from_str(DEFAULT_DIALOGUE).unwrap_or(Value::Null)
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
fn quit(app: AppHandle) {
    app.exit(0);
}

fn main() {
    let home =
        MarimoHome::resolve().expect("cannot resolve the marimo home directory; set MARIMO_HOME");
    let _ = fs::create_dir_all(home.sessions_dir());

    tauri::Builder::default()
        .manage(AppState { home: home.clone() })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_dialogue,
            get_scale,
            set_scale,
            quit
        ])
        .setup(move |app| {
            // macOS では skipTaskbar が Dock に効かないので、アクセサリ扱いにして Dock から外す。
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running marimo");
}
