use marimo_core::Lang;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "settings";
/// 開いているウィンドウに、左の一覧のどの項目を出すかを送る。settingsProtocol.ts の SECTION_EVENT と揃える。
const SECTION_EVENT: &str = "settings-section";

/// 設定の状態はメインのウィンドウが持つので、このウィンドウは内容を出すだけで、何を表示するかは settings.ts が
/// メインのウィンドウに問い合わせて決める。
#[tauri::command]
pub fn open_settings(app: AppHandle) {
    open(&app);
}

/// Windows の WebView2 はウィンドウを作るときにメインスレッドのメッセージループを要するので、メインスレッドで
/// 待つ同期コマンドやメニューのイベントからは作らず、非同期の実行環境へ回す。
pub fn open(app: &AppHandle) {
    spawn_show(app, None);
}

/// section は settings.html の左の一覧の data-section の値で、開いたウィンドウにその項目を出す。
pub fn open_section(app: &AppHandle, section: &'static str) {
    spawn_show(app, Some(section));
}

fn title(lang: Lang) -> &'static str {
    lang.pick("marimo の設定", "marimo Settings")
}

/// 言語を変えたときに、開いている設定のウィンドウのタイトルを付け直す。
pub fn retitle(app: &AppHandle, lang: Lang) {
    if let Some(window) = app.get_webview_window(LABEL)
        && let Err(e) = window.set_title(title(lang))
    {
        eprintln!("marimo: cannot retitle the settings window: {e}");
    }
}

fn spawn_show(app: &AppHandle, section: Option<&'static str>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = show(&app, section) {
            eprintln!("marimo: cannot open the settings window: {e}");
        }
    });
}

fn show(app: &AppHandle, section: Option<&'static str>) -> tauri::Result<()> {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => {
            window.unminimize()?;
            window.show()?;
            if let Some(section) = section {
                window.emit_to(LABEL, SECTION_EVENT, section)?;
            }
            window
        }
        None => {
            let page = section.map_or_else(
                || "settings.html".to_string(),
                |s| format!("settings.html#{s}"),
            );
            WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(page.into()))
                .title(title(crate::lang(app)))
                .inner_size(640.0, 560.0)
                .min_inner_size(520.0, 360.0)
                .maximizable(false)
                .center()
                .build()?
        }
    };
    // macOS では Accessory のアプリは前面のアプリにならないので、ウィンドウを出しただけでは使っているアプリの
    // 後ろに開く。set_focus はアプリを activate してからウィンドウをキーボードの入力先にする。
    window.set_focus()
}
