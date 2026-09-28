use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::AppState;
use crate::scale::{self, PanelDisplay, PanelStyle};

/// 表示の切り替えを選んだことをフロントエンドへ知らせるイベント。フロントエンドは右クリックメニューと
/// 同じ関数で反映し、保存のコマンドを通じて印を付け直させる。
pub const PANEL_DISPLAY_EVENT: &str = "tray-panel-display";

// 右クリックメニューの項目もこのアイコンの on_menu_event に届くので、自動で振られる ID と重ならない
// 接頭辞を付けて見分ける。
const TOGGLE: &str = "tray:toggle";
const SHOW_CHARACTER: &str = "tray:show-character";
const STYLE_DETAIL: &str = "tray:style-detail";
const STYLE_COUNTS: &str = "tray:style-counts";
const QUIT: &str = "tray:quit";

pub struct Tray {
    toggle: MenuItem<Wry>,
    show_character: CheckMenuItem<Wry>,
    detail: CheckMenuItem<Wry>,
    counts: CheckMenuItem<Wry>,
}

impl Tray {
    pub fn reflect(&self, display: PanelDisplay) {
        let results = [
            self.show_character.set_checked(display.show_character),
            self.detail
                .set_checked(display.panel_style == PanelStyle::Detail),
            self.counts
                .set_checked(display.panel_style == PanelStyle::Counts),
        ];
        if let Some(Err(e)) = results.into_iter().find(Result::is_err) {
            eprintln!("marimo: cannot update the tray menu: {e}");
        }
    }
}

#[derive(Clone, Serialize)]
struct PanelDisplayChange {
    show_character: Option<bool>,
    panel_style: Option<PanelStyle>,
}

fn toggle_label(visible: bool) -> &'static str {
    if visible {
        "marimo を隠す"
    } else {
        "marimo を表示"
    }
}

// macOS のメニューバーは黒だけのテンプレート画像を明暗に合わせて塗り分ける。Windows の通知領域は
// テンプレート画像を扱わず黒のままになり暗いタスクバーに沈むので、色の付いた画像を使う。
#[cfg(target_os = "macos")]
fn icon() -> (Image<'static>, bool) {
    (tauri::include_image!("icons/tray-template.png"), true)
}

#[cfg(not(target_os = "macos"))]
fn icon() -> (Image<'static>, bool) {
    (tauri::include_image!("icons/tray-color.png"), false)
}

pub fn build(app: &AppHandle, display: PanelDisplay) -> tauri::Result<Tray> {
    let tray = Tray {
        toggle: MenuItem::with_id(app, TOGGLE, toggle_label(true), true, None::<&str>)?,
        show_character: CheckMenuItem::with_id(
            app,
            SHOW_CHARACTER,
            "絵を表示",
            true,
            display.show_character,
            None::<&str>,
        )?,
        detail: CheckMenuItem::with_id(
            app,
            STYLE_DETAIL,
            "詳細を表示",
            true,
            display.panel_style == PanelStyle::Detail,
            None::<&str>,
        )?,
        counts: CheckMenuItem::with_id(
            app,
            STYLE_COUNTS,
            "件数だけ表示",
            true,
            display.panel_style == PanelStyle::Counts,
            None::<&str>,
        )?,
    };
    let menu = Menu::with_items(
        app,
        &[
            &tray.toggle,
            &PredefinedMenuItem::separator(app)?,
            &tray.show_character,
            &tray.detail,
            &tray.counts,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT, "終了", true, None::<&str>)?,
        ],
    )?;
    let (image, template) = icon();
    TrayIconBuilder::with_id("main")
        .icon(image)
        .icon_as_template(template)
        .tooltip("marimo")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(tray)
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let change = |current: PanelDisplay| match event.id().as_ref() {
        SHOW_CHARACTER => Some(PanelDisplayChange {
            show_character: Some(!current.show_character),
            panel_style: None,
        }),
        STYLE_DETAIL => Some(PanelDisplayChange {
            show_character: None,
            panel_style: Some(PanelStyle::Detail),
        }),
        STYLE_COUNTS => Some(PanelDisplayChange {
            show_character: None,
            panel_style: Some(PanelStyle::Counts),
        }),
        _ => None,
    };
    match event.id().as_ref() {
        TOGGLE => toggle(app),
        QUIT => app.exit(0),
        _ => {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let current = scale::load_panel_display(&state.home).unwrap_or_default();
            let Some(change) = change(current) else {
                return;
            };
            // 押した印はメニューが自分で反転させるので、保存してある状態へいったん戻す。
            // フロントエンドが反映して保存すると、そのコマンドが新しい状態で付け直す。
            if let Some(tray) = state.tray.get() {
                tray.reflect(current);
            }
            if let Err(e) = app.emit(PANEL_DISPLAY_EVENT, change) {
                eprintln!("marimo: cannot send the tray choice: {e}");
            }
        }
    }
}

fn toggle(app: &AppHandle) {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(true);
    set_visible(app, !visible);
}

/// 隠したことは保存せず、起動するといつも表示から始める。隠している間は、クリックを通す判定の
/// スレッドがカーソルを追わないようにする。
pub fn set_visible(app: &AppHandle, visible: bool) {
    let (Some(window), Some(state)) = (app.get_webview_window("main"), app.try_state::<AppState>())
    else {
        return;
    };
    state.hits.set_hidden(!visible);
    let result = if visible {
        window.show()
    } else {
        window.hide()
    };
    if let Err(e) = result {
        eprintln!("marimo: cannot change window visibility: {e}");
        state.hits.set_hidden(visible);
        return;
    }
    if let Some(tray) = state.tray.get()
        && let Err(e) = tray.toggle.set_text(toggle_label(visible))
    {
        eprintln!("marimo: cannot update the tray menu: {e}");
    }
}
