use std::sync::Mutex;

use marimo_core::Status;
use serde::Deserialize;
use tauri::image::Image;
use tauri::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem,
};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::AppState;
use crate::scale::{self, PanelDisplay, PanelStyle};

/// トレイで選んだ表示を保存したあと、その表示の全体をフロントエンドへ送るイベント。フロントエンドは
/// 反映だけをして保存し直さない。保存をフロントエンドの折り返しに任せると、聞き始める前の選択が消え、
/// 続けて選んだ反転が同じ保存値から作られて一回分になるからである。
const PANEL_DISPLAY_EVENT: &str = "tray-panel-display";

/// 既読の集合はフロントエンドが持つので、トレイの「完了をすべて既読にする」は押されたことだけを送る。
const MARK_ALL_READ_EVENT: &str = "tray-mark-all-read";

/// どのセッションを開くかを決めるのに要る既読の集合はフロントエンドが持つので、選ばれた鍵だけを送る。
const SESSION_EVENT: &str = "tray-session";

// 右クリックメニューの項目もこのアイコンの on_menu_event に届くので、自動で振られる ID と重ならない
// 接頭辞を付けて見分ける。自動の ID は数字だけなので、どちらの接頭辞とも重ならない。セッションの項目は
// 鍵が何であっても固定の項目の ID にならないよう、別の接頭辞にする。
const SESSION_PREFIX: &str = "tray-session:";
const TOGGLE: &str = "tray:toggle";
const SHOW_CHARACTER: &str = "tray:show-character";
const STYLE_DETAIL: &str = "tray:style-detail";
const STYLE_COUNTS: &str = "tray:style-counts";
const MARK_ALL_READ: &str = "tray:mark-all-read";
const QUIT: &str = "tray:quit";

pub struct Tray {
    icon: TrayIcon<Wry>,
    menu: Menu<Wry>,
    // メニューの先頭に差し込んだセッションの項目と、その後ろの区切り。差し替えるときにこれだけを外す。
    sessions: Mutex<Vec<MenuItemKind<Wry>>>,
    attention: Mutex<Attention>,
    toggle: MenuItem<Wry>,
    show_character: CheckMenuItem<Wry>,
    detail: CheckMenuItem<Wry>,
    counts: CheckMenuItem<Wry>,
    mark_all_read: MenuItem<Wry>,
}

/// フロントエンドの panelModel.ts の TrayItem と同じ形。key は sessionKey である。
#[derive(Debug, Deserialize)]
pub struct TraySession {
    key: String,
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attention {
    None,
    Waiting,
    Error,
}

// 集約は承認待ちをエラーより上に置くので、集約だけを見れば「一つでも承認待ちがあれば承認待ち、
// 無くてエラーがあればエラー」になる。完了はフロントエンドが既読で畳むが、承認待ちとエラーは
// 解消するまで行に残すので、既読を考えずにスナップショットの集約から決めてよい。
fn attention(aggregate: Status) -> Attention {
    match aggregate {
        Status::Waiting => Attention::Waiting,
        Status::Error => Attention::Error,
        Status::Idle | Status::Working | Status::Done => Attention::None,
    }
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

    pub fn set_unread_done(&self, any: bool) {
        if let Err(e) = self.mark_all_read.set_enabled(any) {
            eprintln!("marimo: cannot update the tray menu: {e}");
        }
    }

    /// メニューを作り直すと他の項目の印や使えるかどうかを付け直すことになるので、先頭の項目だけを差し替える。
    pub fn set_sessions(&self, sessions: &[TraySession], more: usize) {
        let mut leading = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = self.replace_sessions(&mut leading, sessions, more) {
            eprintln!("marimo: cannot update the tray menu: {e}");
        }
    }

    // 途中で失敗しても、メニューに残っている項目だけを leading に残し、次の差し替えで外せるようにする。
    fn replace_sessions(
        &self,
        leading: &mut Vec<MenuItemKind<Wry>>,
        sessions: &[TraySession],
        more: usize,
    ) -> tauri::Result<()> {
        while let Some(item) = leading.pop() {
            if let Err(e) = self.menu.remove(&item) {
                leading.push(item);
                return Err(e);
            }
        }
        let app = self.menu.app_handle();
        let mut next = Vec::new();
        for session in sessions {
            // muda は & を手前の文字のニーモニックの印として扱うので、題名の & はそのまま見えるよう重ねる。
            let label = session.label.replace('&', "&&");
            let id = format!("{SESSION_PREFIX}{}", session.key);
            next.push(MenuItem::with_id(app, id, label, true, None::<&str>)?.kind());
        }
        if more > 0 {
            next.push(MenuItem::new(app, format!("ほか {more} 件"), false, None::<&str>)?.kind());
        }
        if !next.is_empty() {
            next.push(PredefinedMenuItem::separator(app)?.kind());
        }
        for (position, item) in next.into_iter().enumerate() {
            self.menu.insert(&item, position)?;
            leading.push(item);
        }
        Ok(())
    }

    fn show_attention(&self, next: Attention) {
        let mut current = self.attention.lock().unwrap_or_else(|e| e.into_inner());
        if *current == next {
            return;
        }
        // set_icon は macOS でテンプレートの印を外してしまうので、画像と印を一度に替える。
        let (image, template) = icon(next);
        match self.icon.set_icon_with_as_template(Some(image), template) {
            Ok(()) => *current = next,
            Err(e) => eprintln!("marimo: cannot update the tray icon: {e}"),
        }
    }
}

/// 窓が隠れている間は webview が止まりうるので、フロントエンドを通さずスナップショットを作った側から呼ぶ。
pub fn show_status(app: &AppHandle, aggregate: Status) {
    if let Some(state) = app.try_state::<AppState>()
        && let Some(tray) = state.tray.get()
    {
        tray.show_attention(attention(aggregate));
    }
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
fn quiet_icon() -> (Image<'static>, bool) {
    (tauri::include_image!("icons/tray-template.png"), true)
}

#[cfg(not(target_os = "macos"))]
fn quiet_icon() -> (Image<'static>, bool) {
    (tauri::include_image!("icons/tray-color.png"), false)
}

// 気付かせたい状態の絵は色で見分けるので、macOS でもテンプレート画像にしない。
fn icon(attention: Attention) -> (Image<'static>, bool) {
    match attention {
        Attention::None => quiet_icon(),
        Attention::Waiting => (tauri::include_image!("icons/tray-waiting.png"), false),
        Attention::Error => (tauri::include_image!("icons/tray-error.png"), false),
    }
}

pub fn build(app: &AppHandle, display: PanelDisplay) -> tauri::Result<Tray> {
    let toggle_item = MenuItem::with_id(app, TOGGLE, toggle_label(true), true, None::<&str>)?;
    let show_character = CheckMenuItem::with_id(
        app,
        SHOW_CHARACTER,
        "絵を表示",
        true,
        display.show_character,
        None::<&str>,
    )?;
    let detail = CheckMenuItem::with_id(
        app,
        STYLE_DETAIL,
        "詳細を表示",
        true,
        display.panel_style == PanelStyle::Detail,
        None::<&str>,
    )?;
    let counts = CheckMenuItem::with_id(
        app,
        STYLE_COUNTS,
        "件数だけ表示",
        true,
        display.panel_style == PanelStyle::Counts,
        None::<&str>,
    )?;
    let mark_all_read = MenuItem::with_id(
        app,
        MARK_ALL_READ,
        "完了をすべて既読にする",
        false,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle_item,
            &PredefinedMenuItem::separator(app)?,
            &show_character,
            &detail,
            &counts,
            &PredefinedMenuItem::separator(app)?,
            &mark_all_read,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT, "終了", true, None::<&str>)?,
        ],
    )?;
    let (image, template) = icon(Attention::None);
    let icon = TrayIconBuilder::with_id("main")
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
    Ok(Tray {
        icon,
        menu,
        sessions: Mutex::new(Vec::new()),
        attention: Mutex::new(Attention::None),
        toggle: toggle_item,
        show_character,
        detail,
        counts,
        mark_all_read,
    })
}

fn chosen_display(id: &str, current: PanelDisplay) -> Option<PanelDisplay> {
    match id {
        SHOW_CHARACTER => Some(PanelDisplay {
            show_character: !current.show_character,
            ..current
        }),
        STYLE_DETAIL => Some(PanelDisplay {
            panel_style: PanelStyle::Detail,
            ..current
        }),
        STYLE_COUNTS => Some(PanelDisplay {
            panel_style: PanelStyle::Counts,
            ..current
        }),
        _ => None,
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        TOGGLE => toggle(app),
        MARK_ALL_READ => {
            if let Err(e) = app.emit(MARK_ALL_READ_EVENT, ()) {
                eprintln!("marimo: cannot send the tray choice: {e}");
            }
        }
        QUIT => app.exit(0),
        id if id.starts_with(SESSION_PREFIX) => {
            let key = &id[SESSION_PREFIX.len()..];
            if let Err(e) = app.emit(SESSION_EVENT, key) {
                eprintln!("marimo: cannot send the tray choice: {e}");
            }
        }
        id => {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let current = scale::load_panel_display(&state.home).unwrap_or_default();
            let Some(next) = chosen_display(id, current) else {
                return;
            };
            // 押した印はメニューが自分で反転させるので、保存できたかどうかに合わせて付け直す。
            // 保存できなかったときは、次の選択も今の保存値から作られるよう画面にも反映しない。
            let saved = match scale::save_panel_display(&state.home, next) {
                Ok(()) => next,
                Err(e) => {
                    eprintln!("marimo: cannot save the tray choice: {e}");
                    current
                }
            };
            if let Some(tray) = state.tray.get() {
                tray.reflect(saved);
            }
            if saved == next
                && let Err(e) = app.emit(PANEL_DISPLAY_EVENT, saved)
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use marimo_core::Status::*;
    use marimo_core::aggregate;

    #[test]
    fn attention_follows_the_most_urgent_session() {
        let cases: &[(&[Status], Attention)] = &[
            (&[], Attention::None),
            (&[Idle, Working, Done], Attention::None),
            (&[Done, Error, Working], Attention::Error),
            (&[Working, Waiting], Attention::Waiting),
            (&[Error, Waiting, Done], Attention::Waiting),
        ];
        for (statuses, expected) in cases {
            assert_eq!(
                attention(aggregate(statuses.iter().copied())),
                *expected,
                "{statuses:?}"
            );
        }
    }
}
