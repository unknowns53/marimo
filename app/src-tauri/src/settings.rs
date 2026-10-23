use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "settings";

/// 設定の状態はメインのウィンドウが持つので、このウィンドウは内容を出すだけで、何を表示するかは settings.ts が
/// メインのウィンドウに問い合わせて決める。
#[tauri::command]
pub fn open_settings(app: AppHandle) {
    open(&app);
}

/// Windows の WebView2 はウィンドウを作るときにメインスレッドのメッセージループを要するので、メインスレッドで
/// 待つ同期コマンドやメニューのイベントからは作らず、非同期の実行環境へ回す。
pub fn open(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = show(&app) {
            eprintln!("marimo: cannot open the settings window: {e}");
        }
    });
}

fn show(app: &AppHandle) -> tauri::Result<()> {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => {
            window.unminimize()?;
            window.show()?;
            window
        }
        None => WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
            .title("marimo の設定")
            .inner_size(420.0, 640.0)
            .min_inner_size(360.0, 320.0)
            .maximizable(false)
            .center()
            .build()?,
    };
    // macOS では Accessory のアプリは前面のアプリにならないので、ウィンドウを出しただけでは使っているアプリの
    // 後ろに開く。set_focus はアプリを activate してからウィンドウをキーボードの入力先にする。
    window.set_focus()
}
