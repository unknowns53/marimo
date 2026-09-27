use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use marimo_core::{MarimoHome, store};
use serde::{Deserialize, Serialize};
use tauri::{Monitor, PhysicalPosition, WebviewWindow, WindowEvent};

// ドラッグ中は Moved が連続して届くので、止まってから一度だけ書く。
const SAVE_AFTER: Duration = Duration::from_millis(400);
const EDGE_MARGIN: f64 = 24.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SavedPosition {
    x: i32,
    y: i32,
}

pub fn restore(window: &WebviewWindow, home: &MarimoHome) {
    let monitors = window.available_monitors().unwrap_or_default();
    let saved = fs::read(home.window_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<SavedPosition>(&b).ok())
        // モニタの構成が変わって保存した位置が画面外になった場合は、既定の位置へ戻す。
        .filter(|p| monitors.iter().any(|m| contains(m, *p)));
    let pos = saved
        .map(|p| PhysicalPosition::new(p.x, p.y))
        .or_else(|| default_position(window));
    if let Some(pos) = pos {
        let _ = window.set_position(pos);
    }
}

pub fn track(window: &WebviewWindow, home: MarimoHome) {
    let (tx, rx) = mpsc::channel::<PhysicalPosition<i32>>();
    thread::spawn(move || {
        while let Ok(mut pos) = rx.recv() {
            while let Ok(next) = rx.recv_timeout(SAVE_AFTER) {
                pos = next;
            }
            let saved = SavedPosition { x: pos.x, y: pos.y };
            if let Err(e) = store::write_json_atomic(&home.window_file(), &saved) {
                eprintln!("marimo: cannot save window position: {e}");
            }
        }
    });
    window.on_window_event(move |event| {
        if let WindowEvent::Moved(pos) = event {
            let _ = tx.send(*pos);
        }
    });
}

fn contains(monitor: &Monitor, p: SavedPosition) -> bool {
    let origin = monitor.position();
    let size = monitor.size();
    let (x, y) = (i64::from(p.x) + 16, i64::from(p.y) + 16);
    x >= i64::from(origin.x)
        && y >= i64::from(origin.y)
        && x < i64::from(origin.x) + i64::from(size.width)
        && y < i64::from(origin.y) + i64::from(size.height)
}

fn default_position(window: &WebviewWindow) -> Option<PhysicalPosition<i32>> {
    let monitor = window.primary_monitor().ok().flatten()?;
    let area = monitor.work_area();
    let size = window.outer_size().ok()?;
    let margin = (EDGE_MARGIN * monitor.scale_factor()) as i32;
    Some(PhysicalPosition::new(
        area.position.x + area.size.width as i32 - size.width as i32 - margin,
        area.position.y + area.size.height as i32 - size.height as i32 - margin,
    ))
}
