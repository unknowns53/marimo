use std::collections::HashMap;
use std::sync::Mutex;

use marimo_core::{Lang, MarimoHome, Provider, SessionState, Status, store};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::scale::{self, Shortcuts};
use crate::{AppState, acknowledged, focus, tray};

/// 最優先セッションへの移動ショートカットが動いたとき、フロントエンドで既読と吹き出し閉じを揃える。
pub const SHORTCUT_MARK_READ_EVENT: &str = "shortcut-mark-read";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    FocusUrgent,
    ToggleVisible,
}

#[derive(Default)]
pub struct ShortcutStateStore {
    current: Mutex<Shortcuts>,
    bindings: Mutex<HashMap<Shortcut, Action>>,
}

impl ShortcutStateStore {
    pub fn current(&self) -> Shortcuts {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn action_for(&self, shortcut: &Shortcut) -> Option<Action> {
        self.bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(shortcut)
            .copied()
    }

    fn replace(&self, shortcuts: Shortcuts, bindings: HashMap<Shortcut, Action>) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = shortcuts;
        *self.bindings.lock().unwrap_or_else(|e| e.into_inner()) = bindings;
    }
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let Some(action) = state.shortcuts.action_for(shortcut) else {
                return;
            };
            match action {
                Action::FocusUrgent => focus_urgent(app),
                Action::ToggleVisible => tray::toggle(app),
            }
        })
        .build()
}

/// 起動時に保存済みのショートカットを登録する。他アプリとの競合などで失敗しても起動自体は止めない。
pub fn init(app: &AppHandle, home: &MarimoHome) {
    let saved = scale::load_shortcuts(home);
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let mut bindings = HashMap::new();
    let mut registered = Shortcuts::default();
    for (raw, action) in [
        (saved.focus_urgent.as_deref(), Action::FocusUrgent),
        (saved.toggle_visible.as_deref(), Action::ToggleVisible),
    ] {
        let Some(text) = raw else { continue };
        match text.parse::<Shortcut>() {
            Ok(sc) if !bindings.contains_key(&sc) => {
                if let Err(e) = app.global_shortcut().register(sc) {
                    eprintln!("marimo: cannot register shortcut {text}: {e}");
                } else {
                    bindings.insert(sc, action);
                    let slot = match action {
                        Action::FocusUrgent => &mut registered.focus_urgent,
                        Action::ToggleVisible => &mut registered.toggle_visible,
                    };
                    *slot = Some(text.to_owned());
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("marimo: invalid shortcut {text}: {e}"),
        }
    }
    // 設定のウィンドウには、実際に登録できたものだけを有効として見せる。他のアプリと競合して登録できなかった
    // ものも display.json には残すので、競合が解けた次の起動で登録し直す。
    state.shortcuts.replace(registered, bindings);
}

/// 新しいショートカット設定を検証・登録して保存する。失敗したら直前の登録へ戻して理由を返す。
pub fn apply(
    app: &AppHandle,
    home: &MarimoHome,
    next: Shortcuts,
    lang: Lang,
) -> Result<Shortcuts, String> {
    let normalized = normalize(next);
    let parsed = parse_pair(&normalized, lang)?;
    let state = app.state::<AppState>();
    let previous = state.shortcuts.current();
    let prev_parsed = parse_pair(&previous, lang).unwrap_or_default();

    let manager = app.global_shortcut();
    // 登録か保存に失敗したときは、直前に登録していたものへ戻す。登録したキーと bindings を食い違わせない。
    let restore = || {
        let _ = manager.unregister_all();
        let mut restored = HashMap::new();
        for (old_sc, old_action, _) in &prev_parsed {
            if manager.register(*old_sc).is_ok() {
                restored.insert(*old_sc, *old_action);
            }
        }
        state.shortcuts.replace(previous.clone(), restored);
    };
    let _ = manager.unregister_all();
    let mut bindings = HashMap::new();
    for (sc, action, raw) in &parsed {
        if let Err(e) = manager.register(*sc) {
            restore();
            return Err(lang.pick(
                format!("ショートカット「{raw}」を登録できませんでした（他のアプリと競合している可能性があります: {e}）"),
                format!("Could not register shortcut \"{raw}\" (it may conflict with another app: {e})"),
            ));
        }
        bindings.insert(*sc, *action);
    }

    if let Err(e) = scale::save_shortcuts(home, &normalized) {
        restore();
        return Err(lang.pick(
            format!("ショートカットを保存できませんでした（{e}）"),
            format!("Could not save shortcuts ({e})"),
        ));
    }
    state.shortcuts.replace(normalized.clone(), bindings);
    Ok(normalized)
}

fn normalize(s: Shortcuts) -> Shortcuts {
    let clean = |v: Option<String>| v.map(|x| x.trim().to_owned()).filter(|x| !x.is_empty());
    Shortcuts {
        focus_urgent: clean(s.focus_urgent),
        toggle_visible: clean(s.toggle_visible),
    }
}

fn parse_pair(s: &Shortcuts, lang: Lang) -> Result<Vec<(Shortcut, Action, String)>, String> {
    let mut out = Vec::new();
    for (raw, action) in [
        (s.focus_urgent.as_deref(), Action::FocusUrgent),
        (s.toggle_visible.as_deref(), Action::ToggleVisible),
    ] {
        let Some(text) = raw else { continue };
        let sc: Shortcut = text.parse().map_err(|_| {
            lang.pick(
                format!("ショートカット「{text}」の形式を解釈できませんでした。"),
                format!("Could not parse shortcut \"{text}\"."),
            )
        })?;
        // Shift だけか修飾キーの無い組み合わせを OS 全体に登録すると、どのアプリでもその文字が打てなくなる。
        if !sc
            .mods
            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
        {
            return Err(lang.pick(
                format!("ショートカット「{text}」には Ctrl、Alt、Cmd（Windows キー）のいずれかを含めてください。Shift だけの組み合わせは、ほかのアプリで文字を打てなくなります。"),
                format!("Shortcut \"{text}\" must include Ctrl, Alt or Cmd (the Windows key). A combination with only Shift would stop that character from being typed in other apps."),
            ));
        }
        if out.iter().any(|(existing, _, _)| existing == &sc) {
            return Err(lang
                .pick(
                    "同じショートカットを両方の操作には割り当てられません。",
                    "Both actions cannot use the same shortcut.",
                )
                .into());
        }
        out.push((sc, action, text.to_owned()));
    }
    Ok(out)
}

/// 未読の要対応セッション（承認待ち→エラー→完了）のうち最優先のものへ移動する。
/// ウィンドウを隠している間でも即座に反応できるよう、フォーカス自体は Rust 側で直接実行し、
/// 既読と吹き出しの更新をフロントエンドへ知らせる。
fn focus_urgent(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let snapshot = store::load_snapshot(&state.home);
    let read = state.acknowledged.lock().unwrap_or_else(|e| e.into_inner());
    let target = pick_urgent(&snapshot.sessions, &read).cloned();
    drop(read);
    let Some(session) = target else {
        return;
    };
    focus::run(focus::plan(&session));
    let key = session_key(&session);
    if let Err(e) = app.emit(SHORTCUT_MARK_READ_EVENT, key) {
        eprintln!("marimo: cannot send shortcut mark-read: {e}");
    }
}

fn pick_urgent<'a>(
    sessions: &'a [SessionState],
    read: &std::collections::HashSet<String>,
) -> Option<&'a SessionState> {
    sessions.iter().find(|s| match s.status {
        Status::Waiting => true,
        Status::Error | Status::Done => !read.contains(&acknowledged::trigger_key(s)),
        Status::Working | Status::Idle => false,
    })
}

fn session_key(s: &SessionState) -> String {
    if s.provider == Provider::Claude {
        s.session_id.clone()
    } else {
        let prefix = match s.provider {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Hermes => "hermes",
            Provider::Opencode => "opencode",
            Provider::Antigravity => "antigravity",
        };
        format!("{prefix}:{}", s.session_id)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn parses_shortcuts_and_rejects_duplicates_or_invalid() {
        let ok = parse_pair(
            &Shortcuts {
                focus_urgent: Some("CommandOrControl+Shift+M".into()),
                toggle_visible: Some("Alt+Shift+M".into()),
            },
            Lang::En,
        )
        .unwrap();
        assert_eq!(ok.len(), 2);

        let dup = parse_pair(
            &Shortcuts {
                focus_urgent: Some("CommandOrControl+Shift+M".into()),
                toggle_visible: Some("CommandOrControl+Shift+M".into()),
            },
            Lang::En,
        );
        assert!(dup.is_err());

        let invalid = parse_pair(
            &Shortcuts {
                focus_urgent: Some("NotAKey+++".into()),
                toggle_visible: None,
            },
            Lang::En,
        );
        assert!(invalid.is_err());

        for text in ["Shift+A", "A", "Shift+1"] {
            let typing = parse_pair(
                &Shortcuts {
                    focus_urgent: Some(text.into()),
                    toggle_visible: None,
                },
                Lang::En,
            );
            assert!(typing.is_err(), "{text}");
        }
    }

    #[test]
    fn picks_highest_priority_unread_attention_session() {
        let mk = |id: &str, status: Status| SessionState {
            status,
            status_since: 10,
            ..SessionState::new(id)
        };
        let done_read = mk("d1", Status::Done);
        let done_unread = mk("d2", Status::Done);
        let err_unread = mk("e1", Status::Error);
        let wait = mk("w1", Status::Waiting);
        let busy = mk("b1", Status::Working);
        let read = HashSet::from([acknowledged::trigger_key(&done_read)]);

        // load_snapshot は優先度順に並べる前提。
        let cases: &[(&str, Vec<SessionState>, Option<&str>)] = &[
            (
                "waiting beats error",
                vec![wait.clone(), err_unread.clone()],
                Some("w1"),
            ),
            (
                "skips read done",
                vec![done_read.clone(), done_unread.clone()],
                Some("d2"),
            ),
            (
                "ignores working",
                vec![busy.clone(), done_read.clone()],
                None,
            ),
            (
                "waiting even if key in read",
                vec![wait.clone()],
                Some("w1"),
            ),
        ];
        for (label, list, want) in cases {
            let mut r = read.clone();
            r.insert(acknowledged::trigger_key(&wait));
            assert_eq!(
                pick_urgent(list, &r).map(|s| s.session_id.as_str()),
                *want,
                "{label}"
            );
        }
    }
}
