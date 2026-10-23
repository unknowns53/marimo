use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use marimo_core::{MarimoHome, store};
use serde::{Deserialize, Serialize};
use tauri::{Monitor, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

// ドラッグ中は Moved が連続して届くので、止まってから一度だけ書く。
const SAVE_AFTER: Duration = Duration::from_millis(400);
const EDGE_MARGIN: f64 = 24.0;

/// 立ち絵をウィンドウのどの隅に置くか。パネルは立ち絵の横の画面の内側の方へ置き、上下は立ち絵と同じ端を揃えて
/// 反対の向きへ伸ばす。立ち絵が画面の右か左か、上か下かの半分にあるかで決める。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Corner {
    pub left: bool,
    pub top: bool,
}

/// ウィンドウの左上からの立ち絵の領域。論理 px。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

// 隅を持たない以前の window.json は、右下に置いていたウィンドウの位置である。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

impl Rect {
    fn contains(&self, x: i32, y: i32) -> bool {
        let (x, y) = (i64::from(x), i64::from(y));
        x >= i64::from(self.x)
            && y >= i64::from(self.y)
            && x < i64::from(self.x) + i64::from(self.w)
            && y < i64::from(self.y) + i64::from(self.h)
    }
}

/// モニタの全体と、メニューバーやタスクバーを除いた作業領域。どちらも物理 px。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    pub bounds: Rect,
    pub work: Rect,
    pub factor: f64,
}

/// ウィンドウの位置と大きさを読み書きする trait。実装は WebviewWindow で、テストでは画面を持たない fake を渡し、
/// 起動、移動、大きさの変更の流れを画面に何も出さずに確かめる。
pub trait Screen {
    fn factor(&self) -> f64;
    /// 枠を含めたウィンドウ全体の矩形。物理 px。
    fn outer(&self) -> Option<Rect>;
    fn displays(&self) -> Vec<Display>;
    fn primary(&self) -> Option<Display>;
    /// ウィンドウが今いるモニタ。どのモニタにも入らない位置のときに使う。
    fn current(&self) -> Option<Display>;
    fn set_size(&self, w: i32, h: i32);
    fn set_position(&self, x: i32, y: i32) -> bool;
}

fn display(monitor: Monitor) -> Display {
    let (origin, size, area) = (monitor.position(), monitor.size(), monitor.work_area());
    Display {
        bounds: Rect {
            x: origin.x,
            y: origin.y,
            w: size.width as i32,
            h: size.height as i32,
        },
        work: Rect {
            x: area.position.x,
            y: area.position.y,
            w: area.size.width as i32,
            h: area.size.height as i32,
        },
        factor: monitor.scale_factor(),
    }
}

impl Screen for WebviewWindow {
    fn factor(&self) -> f64 {
        self.scale_factor().unwrap_or(1.0)
    }

    fn outer(&self) -> Option<Rect> {
        let (pos, size) = (self.outer_position().ok()?, self.outer_size().ok()?);
        Some(Rect {
            x: pos.x,
            y: pos.y,
            w: size.width as i32,
            h: size.height as i32,
        })
    }

    fn displays(&self) -> Vec<Display> {
        self.available_monitors()
            .unwrap_or_default()
            .into_iter()
            .map(display)
            .collect()
    }

    fn primary(&self) -> Option<Display> {
        self.primary_monitor().ok().flatten().map(display)
    }

    fn current(&self) -> Option<Display> {
        self.current_monitor().ok().flatten().map(display)
    }

    fn set_size(&self, w: i32, h: i32) {
        let _ = WebviewWindow::set_size(self, PhysicalSize::new(w.max(1) as u32, h.max(1) as u32));
    }

    fn set_position(&self, x: i32, y: i32) -> bool {
        WebviewWindow::set_position(self, PhysicalPosition::new(x, y)).is_ok()
    }
}

fn to_physical(size: (f64, f64), factor: f64) -> (i32, i32) {
    (
        (size.0 * factor).round() as i32,
        (size.1 * factor).round() as i32,
    )
}

fn work_at(screen: &impl Screen, x: i32, y: i32) -> Option<Rect> {
    screen
        .displays()
        .into_iter()
        .find(|d| d.bounds.contains(x, y))
        .or_else(|| screen.current())
        .map(|d| d.work)
}

/// ウィンドウを開くときに、保存した位置と隅を戻してから、置き場所に合う隅を選び直す。選び直してウィンドウを動かしたら
/// 保存する。ウィンドウを出す前の移動は Moved として track に届かないので、ここで保存しないと残らない。
/// 以前の版が保存した位置には隅が無いので、そのときもここで選び直す。`size` は論理 px。
pub fn start(
    screen: &impl Screen,
    home: &MarimoHome,
    size: (f64, f64),
    stage: impl Fn(Corner) -> StageRect,
) -> Corner {
    let (corner, rect) = restore(screen, home, size);
    match settle(screen, rect, corner, stage) {
        Some((next, pos)) => {
            save(home, pos, next);
            next
        }
        None => corner,
    }
}

// macOS はウィンドウの大きさと位置の変更をあとで適用するので、直後に outer() で読み返すと変更前の値が返る。
// そのため読み返さず、置いた矩形を返す。
fn restore(screen: &impl Screen, home: &MarimoHome, size: (f64, f64)) -> (Corner, Rect) {
    let (w, h) = to_physical(size, screen.factor());
    screen.set_size(w, h);
    let displays = screen.displays();
    let saved = fs::read(home.window_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<SavedPosition>(&b).ok())
        // モニタの構成が変わって保存した位置が画面外になった場合は、既定の位置へ戻す。
        .filter(|p| {
            displays
                .iter()
                .any(|d| d.bounds.contains(p.x + 16, p.y + 16))
        });
    let Some(p) = saved else {
        let (x, y) = match default_position(screen, w, h) {
            Some((x, y)) => {
                screen.set_position(x, y);
                (x, y)
            }
            None => screen.outer().map_or((0, 0), |r| (r.x, r.y)),
        };
        return (Corner::default(), Rect { x, y, w, h });
    };
    let area = displays
        .iter()
        .find(|d| d.bounds.contains(p.x + 16, p.y + 16))
        .map(|d| d.work);
    let (x, y) = fit(
        Rect {
            x: p.x,
            y: p.y,
            w,
            h,
        },
        area,
    );
    screen.set_position(x, y);
    (
        Corner {
            left: p.left,
            top: p.top,
        },
        Rect { x, y, w, h },
    )
}

// 立ち絵のある隅の角を動かさずに大きさを変え、立ち絵が画面上で動かないようにする。
// そのうえで、はみ出した分は画面内へ押し戻す。`size` は論理 px。
pub fn resize_keeping_corner(screen: &impl Screen, size: (f64, f64), corner: Corner) {
    let Some(current) = screen.outer() else {
        return;
    };
    let (w, h) = to_physical(size, screen.factor());
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
    let area = work_at(screen, fixed_x, fixed_y);
    let (x, y) = anchor(current, w, h, corner, area);
    screen.set_size(w, h);
    screen.set_position(x, y);
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

/// ウィンドウを置き終えた位置で、立ち絵のある画面の半分に合う隅を選び直す。隅が変わったら、立ち絵が画面上で
/// 動かないようウィンドウをずらして、新しい隅と位置を返す。`stage` は隅ごとのウィンドウの中の立ち絵の領域を返す。
pub fn settle(
    screen: &impl Screen,
    window: Rect,
    current: Corner,
    stage: impl Fn(Corner) -> StageRect,
) -> Option<(Corner, (i32, i32))> {
    let factor = screen.factor();
    let (cx, cy) = stage_center(window, stage(current), factor);
    let area = work_at(screen, cx, cy)?;
    let (next, (x, y)) = relocate(window, current, &stage, factor, area)?;
    screen.set_position(x, y).then_some((next, (x, y)))
}

fn stage_center(window: Rect, stage: StageRect, factor: f64) -> (i32, i32) {
    (
        window.x + ((stage.x + stage.w / 2.0) * factor).round() as i32,
        window.y + ((stage.y + stage.h / 2.0) * factor).round() as i32,
    )
}

// 隅の判定は、ウィンドウではなく立ち絵の中心で行う。隅を変えても立ち絵は動かないので、ずらしたウィンドウで判定し直しても
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
        // ウィンドウが画面より大きいときは、左上をそろえて、ドラッグできる上端を画面に残す。
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

/// ウィンドウが止まるたびに `settled` を呼び、それが返した位置と隅を保存する。
pub fn track(
    window: &WebviewWindow,
    home: MarimoHome,
    settled: impl Fn((i32, i32)) -> ((i32, i32), Corner) + Send + 'static,
) {
    let (tx, rx) = mpsc::channel::<(i32, i32)>();
    thread::spawn(move || {
        while let Ok(mut pos) = rx.recv() {
            while let Ok(next) = rx.recv_timeout(SAVE_AFTER) {
                pos = next;
            }
            let (pos, corner) = settled(pos);
            save(&home, pos, corner);
        }
    });
    // 隠しているウィンドウの位置は画面上のどこでもないので、仮に届いても覚えない。
    let tracked = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Moved(pos) = event
            && tracked.is_visible().unwrap_or(true)
        {
            let _ = tx.send((pos.x, pos.y));
        }
    });
}

fn save(home: &MarimoHome, pos: (i32, i32), corner: Corner) {
    let saved = SavedPosition {
        x: pos.0,
        y: pos.1,
        left: corner.left,
        top: corner.top,
    };
    if let Err(e) = store::write_json_durable(&home.window_file(), &saved) {
        eprintln!("marimo: cannot save window position: {e}");
    }
}

fn default_position(screen: &impl Screen, w: i32, h: i32) -> Option<(i32, i32)> {
    let display = screen.primary()?;
    let area = display.work;
    let margin = (EDGE_MARGIN * display.factor) as i32;
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
    use std::cell::Cell;

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
        // 画面の上端近くに置いたウィンドウを大きくすると、上へ広げた分が画面外へ出るので押し戻す。
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
        // ウィンドウ 500×400、立ち絵 180×270 を左右の端から 8、上の隅では上から 120、下の隅では下から 8 に置く。
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
            // ずらしたウィンドウが画面からはみ出す分は押し戻す。
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
        // 移したウィンドウで判定し直しても、隅は変わらない。
        let (corner, (x, y)) = relocate(at(100, 100), BR, stage, 2.0, SCREEN).unwrap();
        assert_eq!(relocate(at(x, y), corner, stage, 2.0, SCREEN), None);
    }

    // 画面を持たないウィンドウ。モニタは一つで、上端 50 物理 px をメニューバーが使う。macOS と同じく、大きさと位置の
    // 変更は outer() に返らず、読み返すといつも開いたときの矩形が返る。
    struct FakeScreen {
        outer: Rect,
        placed: Cell<Rect>,
    }

    impl Screen for FakeScreen {
        fn factor(&self) -> f64 {
            2.0
        }
        fn outer(&self) -> Option<Rect> {
            Some(self.outer)
        }
        fn displays(&self) -> Vec<Display> {
            vec![Display {
                bounds: Rect {
                    y: 0,
                    h: 1950,
                    ..SCREEN
                },
                work: SCREEN,
                factor: 2.0,
            }]
        }
        fn primary(&self) -> Option<Display> {
            self.displays().pop()
        }
        fn current(&self) -> Option<Display> {
            self.primary()
        }
        fn set_size(&self, w: i32, h: i32) {
            self.placed.set(Rect {
                w,
                h,
                ..self.placed.get()
            });
        }
        fn set_position(&self, x: i32, y: i32) -> bool {
            self.placed.set(Rect {
                x,
                y,
                ..self.placed.get()
            });
            true
        }
    }

    #[test]
    fn start_restores_and_saves_the_corner() {
        let stage = |c: Corner| StageRect {
            x: if c.left { 8.0 } else { 312.0 },
            y: if c.top { 120.0 } else { 122.0 },
            w: 180.0,
            h: 270.0,
        };
        let tl = Corner {
            left: true,
            top: true,
        };
        let saved = |x, y, c: Corner| SavedPosition {
            x,
            y,
            left: c.left,
            top: c.top,
        };
        // 既定の位置は作業領域の右下から 24 論理 px 内側。
        let default = (3000 - 1000 - 48, 1950 - 800 - 48);
        let cases = [
            ("first start", None, BR, default, None),
            (
                "old file at the top left",
                Some(r#"{"x":100,"y":100}"#),
                tl,
                (708, 104),
                Some(saved(708, 104, tl)),
            ),
            (
                "saved corner",
                Some(r#"{"x":708,"y":104,"left":true,"top":true}"#),
                tl,
                (708, 104),
                Some(saved(708, 104, tl)),
            ),
            (
                "off screen",
                Some(r#"{"x":9000,"y":9000}"#),
                BR,
                default,
                Some(saved(9000, 9000, BR)),
            ),
        ];
        for (label, file, corner, (x, y), after) in cases {
            let dir = tempfile::tempdir().unwrap();
            let home = MarimoHome::at(dir.path());
            if let Some(json) = file {
                fs::write(home.window_file(), json).unwrap();
            }
            // tauri.conf の大きさで画面の中央に開いたウィンドウ。
            let opened = Rect {
                x: 984,
                y: 577,
                w: 1032,
                h: 796,
            };
            let screen = FakeScreen {
                outer: opened,
                placed: Cell::new(opened),
            };
            assert_eq!(
                start(&screen, &home, (500.0, 400.0), stage),
                corner,
                "{label}"
            );
            assert_eq!(
                screen.placed.get(),
                Rect {
                    x,
                    y,
                    w: 1000,
                    h: 800
                },
                "{label}"
            );
            let written = fs::read(home.window_file())
                .ok()
                .map(|b| serde_json::from_slice::<SavedPosition>(&b).unwrap());
            assert_eq!(written, after, "{label}");
        }
    }
}
