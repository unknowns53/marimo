// リリースビルドの Windows でコンソール窓を開かないようにする。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod watch;
mod window_pos;

use std::fs;
use std::io;

use marimo_core::{MarimoHome, Snapshot, store};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

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
fn quit(app: AppHandle) {
    app.exit(0);
}

fn main() {
    let home =
        MarimoHome::resolve().expect("cannot resolve the marimo home directory; set MARIMO_HOME");
    let _ = fs::create_dir_all(home.sessions_dir());

    tauri::Builder::default()
        .manage(AppState { home: home.clone() })
        .invoke_handler(tauri::generate_handler![get_snapshot, get_dialogue, quit])
        .setup(move |app| {
            // macOS では skipTaskbar が Dock に効かないので、アクセサリ扱いにして Dock から外す。
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let window = app
                .get_webview_window("main")
                .expect("main window is declared in tauri.conf.json");
            window_pos::restore(&window, &home);
            window.show()?;
            window_pos::track(&window, home.clone());
            watch::spawn(app.handle().clone(), home.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running marimo");
}
