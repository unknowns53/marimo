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

/// 立ち絵を窓のどの隅に置くか。パネルは立ち絵の横の画面の内側の方へ置き、上下は立ち絵と同じ端を揃えて
/// 反対の向きへ伸ばす。立ち絵が画面の右か左か、上か下かの半分にあるかで決める。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Corner {
    pub left: bool,
    pub top: bool,
}

/// 窓の左上からの立ち絵の枠。論理 px。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

// 隅を持たない以前の window.json は、右下に置いていた窓の位置である。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SavedPosition {
    x: i32,
    y: i32,
    #[serde(default)]
    left: bool,
    #[serde(default)]
    top: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 保存した位置と隅を戻し、戻した隅を返す。
pub fn restore(window: &WebviewWindow, home: &MarimoHome, size: LogicalSize<f64>) -> Corner {
    let physical: PhysicalSize<u32> = size.to_physical(window.scale_factor().unwrap_or(1.0));
    let _ = window.set_size(physical);
    let (w, h) = (physical.width as i32, physical.height as i32);

    let monitors = window.available_monitors().unwrap_or_default();
    let saved = fs::read(home.window_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<SavedPosition>(&b).ok())
        // モニタの構成が変わって保存した位置が画面外になった場合は、既定の位置へ戻す。
        .filter(|p| monitors.iter().any(|m| m_contains(m, p.x + 16, p.y + 16)));
    let corner = saved.map_or(Corner::default(), |p| Corner {
        left: p.left,
        top: p.top,
    });
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
    corner
}

// 立ち絵のある隅の角を動かさずに大きさを変え、立ち絵が画面上で動かないようにする。
// そのうえで、はみ出した分は画面内へ押し戻す。
pub fn resize_keeping_corner(
    window: &WebviewWindow,
    size: LogicalSize<f64>,
    corner: Corner,
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
    let fixed_x = if corner.left {
        current.x
    } else {
        current.x + current.w - 1
    };
    let fixed_y = if corner.top {
        current.y
    } else {
        current.y + current.h - 1
    };
    let area = monitor_at(&monitors, fixed_x, fixed_y)
        .or_else(|| window.current_monitor().ok().flatten())
        .map(|m| work_area(&m));
    let (x, y) = anchor(current, new.width as i32, new.height as i32, corner, area);
    window.set_size(new)?;
    window.set_position(PhysicalPosition::new(x, y))
}

pub fn anchor(current: Rect, w: i32, h: i32, corner: Corner, area: Option<Rect>) -> (i32, i32) {
    let x = if corner.left {
        current.x
    } else {
        current.x + current.w - w
    };
    let y = if corner.top {
        current.y
    } else {
        current.y + current.h - h
    };
    fit(Rect { x, y, w, h }, area)
}

/// 窓を置き終えた位置で、立ち絵のある画面の半分に合う隅を選び直す。隅が変わったら、立ち絵が画面上で
/// 動かないよう窓をずらして、新しい隅と位置を返す。`stage` は隅ごとの窓の中の立ち絵の枠を返す。
pub fn settle(
    window: &WebviewWindow,
    pos: PhysicalPosition<i32>,
    current: Corner,
    stage: impl Fn(Corner) -> StageRect,
) -> Option<(Corner, PhysicalPosition<i32>)> {
    let factor = window.scale_factor().ok()?;
    let size = window.outer_size().ok()?;
    let window_rect = Rect {
        x: pos.x,
        y: pos.y,
        w: size.width as i32,
        h: size.height as i32,
    };
    let (cx, cy) = stage_center(window_rect, stage(current), factor);
    let monitors = window.available_monitors().unwrap_or_default();
    let area = monitor_at(&monitors, cx, cy)
        .or_else(|| window.current_monitor().ok().flatten())
        .map(|m| work_area(&m))?;
    let (next, (x, y)) = relocate(window_rect, current, &stage, factor, area)?;
    window.set_position(PhysicalPosition::new(x, y)).ok()?;
    Some((next, PhysicalPosition::new(x, y)))
}

fn stage_center(window: Rect, stage: StageRect, factor: f64) -> (i32, i32) {
    (
        window.x + ((stage.x + stage.w / 2.0) * factor).round() as i32,
        window.y + ((stage.y + stage.h / 2.0) * factor).round() as i32,
    )
}

// 隅の判定は、窓ではなく立ち絵の中心で行う。隅を変えても立ち絵は動かないので、ずらした窓で判定し直しても
// 隅が行き来しない。
fn relocate(
    window: Rect,
    current: Corner,
    stage: impl Fn(Corner) -> StageRect,
    factor: f64,
    area: Rect,
) -> Option<(Corner, (i32, i32))> {
    let (cx, cy) = stage_center(window, stage(current), factor);
    let next = Corner {
        left: cx < area.x + area.w / 2,
        top: cy < area.y + area.h / 2,
    };
    if next == current {
        return None;
    }
    let (from, to) = (stage(current), stage(next));
    let x = window.x + ((from.x - to.x) * factor).round() as i32;
    let y = window.y + ((from.y - to.y) * factor).round() as i32;
    Some((next, fit(Rect { x, y, ..window }, Some(area))))
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

/// 窓が止まるたびに `settled` を呼び、それが返した位置と隅を保存する。
pub fn track(
    window: &WebviewWindow,
    home: MarimoHome,
    settled: impl Fn(PhysicalPosition<i32>) -> (PhysicalPosition<i32>, Corner) + Send + 'static,
) {
    let (tx, rx) = mpsc::channel::<PhysicalPosition<i32>>();
    thread::spawn(move || {
        while let Ok(mut pos) = rx.recv() {
            while let Ok(next) = rx.recv_timeout(SAVE_AFTER) {
                pos = next;
            }
            let (pos, corner) = settled(pos);
            let saved = SavedPosition {
                x: pos.x,
                y: pos.y,
                left: corner.left,
                top: corner.top,
            };
            if let Err(e) = store::write_json_durable(&home.window_file(), &saved) {
                eprintln!("marimo: cannot save window position: {e}");
            }
        }
    });
    // 隠している窓の位置は画面上のどこでもないので、仮に届いても覚えない。
    let tracked = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Moved(pos) = event
            && tracked.is_visible().unwrap_or(true)
        {
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

    const BR: Corner = Corner {
        left: false,
        top: false,
    };

    #[test]
    fn anchor_cases() {
        let current = Rect {
            x: 2400,
            y: 1000,
            w: 520,
            h: 880,
        };
        // 立ち絵のある隅の角は、広げても縮めても動かない。
        for (corner, (w, h), expected) in [
            (BR, (964, 1690), (2920 - 964, 1880 - 1690)),
            (BR, (400, 700), (2920 - 400, 1880 - 700)),
            (
                Corner {
                    left: true,
                    top: false,
                },
                (400, 700),
                (2400, 1880 - 700),
            ),
            (
                Corner {
                    left: false,
                    top: true,
                },
                (400, 700),
                (2920 - 400, 1000),
            ),
            (
                Corner {
                    left: true,
                    top: true,
                },
                (400, 700),
                (2400, 1000),
            ),
        ] {
            assert_eq!(
                anchor(current, w, h, corner, Some(SCREEN)),
                expected,
                "{corner:?}"
            );
        }

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
            assert_eq!(anchor(current, w, h, BR, area), expected, "{label}");
        }
    }

    #[test]
    fn relocate_keeps_the_stage_still() {
        // 窓 500×400、立ち絵 180×270 を左右の端から 8、上の隅では上から 120、下の隅では下から 8 に置く。
        let stage = |c: Corner| StageRect {
            x: if c.left { 8.0 } else { 312.0 },
            y: if c.top { 120.0 } else { 122.0 },
            w: 180.0,
            h: 270.0,
        };
        let at = |x, y| Rect {
            x,
            y,
            w: 1000,
            h: 800,
        };
        let tl = Corner {
            left: true,
            top: true,
        };
        let cases = [
            ("stays bottom right", at(1800, 1000), BR, None),
            (
                "moved to the top left",
                at(100, 100),
                BR,
                Some((tl, (100 + 608, 100 + 4))),
            ),
            ("stays top left", at(708, 104), tl, None),
            // ずらした窓が画面からはみ出す分は押し戻す。
            (
                "pushed back in",
                at(100, 1300),
                BR,
                Some((
                    Corner {
                        left: true,
                        top: false,
                    },
                    (100 + 608, 1950 - 800),
                )),
            ),
        ];
        for (label, window, current, expected) in cases {
            assert_eq!(
                relocate(window, current, stage, 2.0, SCREEN),
                expected,
                "{label}"
            );
        }
        // 移した窓で判定し直しても、隅は変わらない。
        let (corner, (x, y)) = relocate(at(100, 100), BR, stage, 2.0, SCREEN).unwrap();
        assert_eq!(relocate(at(x, y), corner, stage, 2.0, SCREEN), None);
    }
}
