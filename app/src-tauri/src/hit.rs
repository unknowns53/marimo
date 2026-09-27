use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

// カーソルが窓の上にあるときは、切り替えの遅れが体感に出ないよう短い周期で判定する。
// 窓の外では判定を急ぐ必要がないので、周期を延ばして常駐の負担を減らす。
const INSIDE_INTERVAL: Duration = Duration::from_millis(40);
const OUTSIDE_INTERVAL: Duration = Duration::from_millis(150);

pub const PORTRAIT_HOVER_EVENT: &str = "portrait-hover";
pub const WINDOW_CURSOR_EVENT: &str = "window-cursor";

/// 窓の左上からの CSS px で表したカーソルの位置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CursorPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Mask {
    #[serde(flatten)]
    pub rect: Rect,
    pub cols: usize,
    pub rows: usize,
    pub bits: String,
}

/// フロントエンドが送る、クリックを受け取る領域。座標は窓の左上からの CSS px。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HitRegions {
    pub rects: Vec<Rect>,
    pub mask: Option<Mask>,
}

impl HitRegions {
    pub fn hit(&self, x: f64, y: f64) -> bool {
        self.rects.iter().any(|r| r.contains(x, y))
            || self.mask.as_ref().is_some_and(|m| m.hit(x, y))
    }
}

impl Mask {
    fn hit(&self, x: f64, y: f64) -> bool {
        if !self.rect.contains(x, y) || self.cols == 0 || self.rows == 0 {
            return false;
        }
        let col = (((x - self.rect.x) / self.rect.w) * self.cols as f64) as usize;
        let row = (((y - self.rect.y) / self.rect.h) * self.rows as f64) as usize;
        let i = row.min(self.rows - 1) * self.cols + col.min(self.cols - 1);
        self.bits.as_bytes().get(i) == Some(&b'1')
    }
}

#[derive(Debug, Clone, Copy)]
struct Geometry {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    scale: f64,
}

#[derive(Default)]
pub struct HitState {
    regions: Mutex<Option<HitRegions>>,
}

impl HitState {
    pub fn set(&self, regions: HitRegions) {
        #[cfg(feature = "cursor-replay")]
        replay::log_regions(&regions);
        if let Ok(mut r) = self.regions.lock() {
            *r = Some(regions);
        }
    }

    fn over_portrait(&self, x: f64, y: f64) -> bool {
        match self.regions.lock() {
            Ok(r) => r
                .as_ref()
                .and_then(|r| r.mask.as_ref())
                .is_some_and(|m| m.hit(x, y)),
            Err(_) => false,
        }
    }

    // 領域がまだ届いていないうちは、窓全体でクリックを受け取る。操作できない状態を作らないためである。
    fn wants_cursor(&self, x: f64, y: f64) -> bool {
        match self.regions.lock() {
            Ok(r) => r.as_ref().is_none_or(|r| r.hit(x, y)),
            Err(_) => true,
        }
    }
}

// 窓がクリックを通す状態になると、カーソルの移動イベントも届かなくなる。そこで、カーソルの
// 画面上の位置を AppHandle::cursor_position で定期的に読み、窓の位置と大きさはイベントで覚えておき、
// set_ignore_cursor_events を切り替える。
pub fn spawn(app: AppHandle, window: WebviewWindow, state: Arc<HitState>) {
    let geometry = Arc::new(Mutex::new(Geometry {
        position: window.outer_position().unwrap_or_default(),
        size: window.outer_size().unwrap_or_default(),
        scale: window.scale_factor().unwrap_or(1.0),
    }));
    {
        let geometry = geometry.clone();
        window.on_window_event(move |event| {
            let Ok(mut g) = geometry.lock() else {
                return;
            };
            match event {
                WindowEvent::Moved(p) => g.position = *p,
                WindowEvent::Resized(s) => g.size = *s,
                WindowEvent::ScaleFactorChanged {
                    scale_factor,
                    new_inner_size,
                    ..
                } => {
                    g.scale = *scale_factor;
                    g.size = *new_inner_size;
                }
                _ => {}
            }
        });
    }

    thread::spawn(move || {
        let mut ignoring = false;
        let mut hovering = false;
        let mut last_cursor: Option<CursorPoint> = None;
        #[cfg(feature = "cursor-replay")]
        let replay = replay::Script::from_env();
        loop {
            let g = match geometry.lock() {
                Ok(g) => *g,
                Err(_) => return,
            };
            #[cfg(feature = "cursor-replay")]
            let cursor = match &replay {
                Some(script) => Ok(script.position(g.position, g.scale)),
                None => app.cursor_position(),
            };
            #[cfg(not(feature = "cursor-replay"))]
            let cursor = app.cursor_position();
            let Ok(cursor) = cursor else {
                thread::sleep(OUTSIDE_INTERVAL);
                continue;
            };
            let x = (cursor.x - f64::from(g.position.x)) / g.scale;
            let y = (cursor.y - f64::from(g.position.y)) / g.scale;
            let inside = cursor.x >= f64::from(g.position.x)
                && cursor.y >= f64::from(g.position.y)
                && cursor.x < f64::from(g.position.x) + f64::from(g.size.width)
                && cursor.y < f64::from(g.position.y) + f64::from(g.size.height);
            if inside {
                let want_ignore = !state.wants_cursor(x, y);
                if want_ignore != ignoring && window.set_ignore_cursor_events(want_ignore).is_ok() {
                    ignoring = want_ignore;
                }
            }
            // 透明な部分では窓にマウスのイベントが届かず、DOM の mouseleave が来ないことがあるので、
            // 立ち絵の上にいるかどうかもここで判定してフロントエンドへ知らせる。
            let over = inside && state.over_portrait(x, y);
            if over != hovering && app.emit(PORTRAIT_HOVER_EVENT, over).is_ok() {
                hovering = over;
            }
            // 畳んだ行を広げるかどうかも、同じ理由で DOM のホバーには頼れない。カーソルの位置を
            // そのまま知らせ、開閉の判断はフロントエンドの状態機械に任せる。
            let point = inside.then_some(CursorPoint { x, y });
            let moved = match (point, last_cursor) {
                (Some(a), Some(b)) => (a.x - b.x).abs() >= 0.5 || (a.y - b.y).abs() >= 0.5,
                (a, b) => a.is_some() != b.is_some(),
            };
            if moved && app.emit(WINDOW_CURSOR_EVENT, point).is_ok() {
                last_cursor = point;
            }
            thread::sleep(if inside {
                INSIDE_INTERVAL
            } else {
                OUTSIDE_INTERVAL
            });
        }
    });
}

#[cfg(feature = "cursor-replay")]
mod replay {
    //! 試験用のビルドだけで使う。実際のマウスを動かさずに開閉の動きを確かめるため、
    //! MARIMO_CURSOR_REPLAY が指すファイルの「経過ミリ秒 x y」（窓の左上からの CSS px）の列を、
    //! 本物のカーソル位置の代わりに流し込み、クリックを受け取る領域の変化を stderr へ記録する。
    use std::time::Instant;

    use tauri::PhysicalPosition;

    use super::HitRegions;

    // 流し込みの時刻と記録の時刻を同じ起点で数え、記録から開閉の時刻を読めるようにする。
    fn clock() -> u64 {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_millis() as u64
    }

    pub struct Script {
        steps: Vec<(u64, f64, f64)>,
    }

    impl Script {
        pub fn from_env() -> Option<Self> {
            let path = std::env::var_os("MARIMO_CURSOR_REPLAY")?;
            let text = std::fs::read_to_string(path).ok()?;
            let steps = text
                .lines()
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    Some((
                        it.next()?.parse().ok()?,
                        it.next()?.parse().ok()?,
                        it.next()?.parse().ok()?,
                    ))
                })
                .collect();
            clock();
            Some(Self { steps })
        }

        pub fn position(&self, window: PhysicalPosition<i32>, scale: f64) -> PhysicalPosition<f64> {
            let t = clock();
            let (x, y) = self
                .steps
                .iter()
                .rev()
                .find(|(at, _, _)| *at <= t)
                .map(|(_, x, y)| (*x, *y))
                .unwrap_or((-1000.0, -1000.0));
            PhysicalPosition::new(
                f64::from(window.x) + x * scale,
                f64::from(window.y) + y * scale,
            )
        }
    }

    pub fn log_regions(regions: &HitRegions) {
        let rects: Vec<String> = regions
            .rects
            .iter()
            .map(|r| format!("({:.0},{:.0} {:.0}x{:.0})", r.x, r.y, r.w, r.h))
            .collect();
        let t = clock();
        eprintln!("REPLAY t={t} regions {}", rects.join(" "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> HitRegions {
        // 4×4 の格子で、中央の 2×2 だけが不透明な絵を 100×100 の位置に置く。
        HitRegions {
            rects: vec![Rect {
                x: 0.0,
                y: 300.0,
                w: 340.0,
                h: 100.0,
            }],
            mask: Some(Mask {
                rect: Rect {
                    x: 100.0,
                    y: 100.0,
                    w: 80.0,
                    h: 120.0,
                },
                cols: 4,
                rows: 4,
                bits: "0000011001100000".into(),
            }),
        }
    }

    #[test]
    fn hits_panel_rects_and_opaque_mask_cells_only() {
        let r = regions();
        assert!(r.hit(10.0, 350.0), "panel");
        assert!(!r.hit(10.0, 250.0), "transparent gap above the panel");
        assert!(r.hit(140.0, 160.0), "opaque centre of the portrait");
        assert!(
            !r.hit(105.0, 105.0),
            "transparent corner inside the portrait box"
        );
        assert!(!r.hit(179.9, 219.9), "transparent bottom-right cell");
        assert!(!r.hit(50.0, 50.0), "outside everything");
    }

    #[test]
    fn missing_regions_accept_everything() {
        let state = HitState::default();
        assert!(state.wants_cursor(1.0, 1.0));
        state.set(regions());
        assert!(!state.wants_cursor(1.0, 1.0));
        assert!(state.wants_cursor(140.0, 160.0));
        assert!(state.over_portrait(140.0, 160.0));
        assert!(
            !state.over_portrait(10.0, 350.0),
            "the panel is not the portrait"
        );
    }

    #[test]
    fn parses_frontend_payload() {
        let json = r#"{"rects":[{"x":8,"y":430,"w":340,"h":120}],
                       "mask":{"x":88,"y":160,"w":180,"h":270,"cols":2,"rows":1,"bits":"01"}}"#;
        let r: HitRegions = serde_json::from_str(json).unwrap();
        assert!(r.hit(200.0, 200.0));
        assert!(!r.hit(100.0, 200.0));
    }
}
