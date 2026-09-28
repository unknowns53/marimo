use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use marimo_core::{MarimoHome, store};
use serde::{Deserialize, Serialize};
use tauri::{LogicalSize, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

// ドラッグ中は Moved が連続して届くので、止まってから一度だけ書く。
const SAVE_AFTER: Duration = Duration::from_millis(400);
const EDGE_MARGIN: f64 = 24.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SavedPosition {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn restore(window: &WebviewWindow, home: &MarimoHome, size: LogicalSize<f64>) {
    let physical: PhysicalSize<u32> = size.to_physical(window.scale_factor().unwrap_or(1.0));
    let _ = window.set_size(physical);
    let (w, h) = (physical.width as i32, physical.height as i32);

    let monitors = window.available_monitors().unwrap_or_default();
    let saved = fs::read(home.window_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<SavedPosition>(&b).ok())
        // モニタの構成が変わって保存した位置が画面外になった場合は、既定の位置へ戻す。
        .filter(|p| monitors.iter().any(|m| m_contains(m, p.x + 16, p.y + 16)));
    let pos = match saved {
        Some(p) => {
            let area = monitor_at(&monitors, p.x + 16, p.y + 16).map(|m| work_area(&m));
            Some(fit(
                Rect {
                    x: p.x,
                    y: p.y,
                    w,
                    h,
                },
                area,
            ))
        }
        None => default_position(window, w, h),
    };
    if let Some((x, y)) = pos {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

// 画面の右下に置く使い方が基本なので、大きさを変えても右下の角を動かさず、
// 左と上へ広げる。そのうえで、はみ出した分は画面内へ押し戻す。
pub fn resize_keeping_bottom_right(
    window: &WebviewWindow,
    size: LogicalSize<f64>,
) -> tauri::Result<()> {
    let pos = window.outer_position()?;
    let old = window.outer_size()?;
    let new: PhysicalSize<u32> = size.to_physical(window.scale_factor()?);
    let current = Rect {
        x: pos.x,
        y: pos.y,
        w: old.width as i32,
        h: old.height as i32,
    };
    let monitors = window.available_monitors().unwrap_or_default();
    let corner = (current.x + current.w - 1, current.y + current.h - 1);
    let area = monitor_at(&monitors, corner.0, corner.1)
        .or_else(|| window.current_monitor().ok().flatten())
        .map(|m| work_area(&m));
    let (x, y) = anchor_bottom_right(current, new.width as i32, new.height as i32, area);
    window.set_size(new)?;
    window.set_position(PhysicalPosition::new(x, y))
}

pub fn anchor_bottom_right(current: Rect, w: i32, h: i32, area: Option<Rect>) -> (i32, i32) {
    let x = current.x + current.w - w;
    let y = current.y + current.h - h;
    fit(Rect { x, y, w, h }, area)
}

fn fit(rect: Rect, area: Option<Rect>) -> (i32, i32) {
    let Some(a) = area else {
        return (rect.x, rect.y);
    };
    let along = |v: i32, len: i32, start: i32, span: i32| {
        let max = start + span - len;
        // 窓が画面より大きいときは、左上をそろえて上端のつまみどころを残す。
        if max < start {
            start
        } else {
            v.clamp(start, max)
        }
    };
    (
        along(rect.x, rect.w, a.x, a.w),
        along(rect.y, rect.h, a.y, a.h),
    )
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

fn m_contains(monitor: &Monitor, x: i32, y: i32) -> bool {
    let origin = monitor.position();
    let size = monitor.size();
    let (x, y) = (i64::from(x), i64::from(y));
    x >= i64::from(origin.x)
        && y >= i64::from(origin.y)
        && x < i64::from(origin.x) + i64::from(size.width)
        && y < i64::from(origin.y) + i64::from(size.height)
}

fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<Monitor> {
    monitors.iter().find(|m| m_contains(m, x, y)).cloned()
}

fn work_area(monitor: &Monitor) -> Rect {
    let area = monitor.work_area();
    Rect {
        x: area.position.x,
        y: area.position.y,
        w: area.size.width as i32,
        h: area.size.height as i32,
    }
}

fn default_position(window: &WebviewWindow, w: i32, h: i32) -> Option<(i32, i32)> {
    let monitor = window.primary_monitor().ok().flatten()?;
    let area = work_area(&monitor);
    let margin = (EDGE_MARGIN * monitor.scale_factor()) as i32;
    let rect = Rect {
        x: area.x + area.w - w - margin,
        y: area.y + area.h - h - margin,
        w,
        h,
    };
    Some(fit(rect, Some(area)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 50,
        w: 3000,
        h: 1900,
    };

    #[test]
    fn anchor_bottom_right_cases() {
        let current = Rect {
            x: 2400,
            y: 1000,
            w: 520,
            h: 880,
        };
        let (x, y) = anchor_bottom_right(current, 964, 1690, Some(SCREEN));
        assert_eq!((x + 964, y + 1690), (2920, 1880));
        let (x, y) = anchor_bottom_right(current, 400, 700, Some(SCREEN));
        assert_eq!((x + 400, y + 700), (2920, 1880));

        let rect = |x, y| Rect {
            x,
            y,
            w: 520,
            h: 880,
        };
        // 画面の上端近くに置いた窓を大きくすると、上へ広げた分が画面外へ出るので押し戻す。
        let cases = [
            (
                "near top",
                rect(100, 60),
                (964, 1690),
                Some(SCREEN),
                (0, 50),
            ),
            (
                "off right",
                rect(2900, 1500),
                (964, 1690),
                Some(SCREEN),
                (3000 - 964, 1950 - 1690),
            ),
            (
                "larger than screen",
                rect(10, 60),
                (964, 2400),
                Some(SCREEN),
                (0, 50),
            ),
            (
                "no monitor info",
                Rect {
                    x: -50,
                    y: -50,
                    w: 100,
                    h: 100,
                },
                (200, 300),
                None,
                (-150, -250),
            ),
        ];
        for (label, current, (w, h), area, expected) in cases {
            assert_eq!(
                anchor_bottom_right(current, w, h, area),
                expected,
                "{label}"
            );
        }
    }
}
