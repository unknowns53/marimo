use std::fs;
use std::io;

use marimo_core::{MarimoHome, store};
use serde::{Deserialize, Serialize};

pub const MIN: f64 = 0.6;
pub const MAX: f64 = 2.5;
pub const DEFAULT: f64 = 1.0;

// style.css の配置と揃えておく必要がある。立ち絵（倍率 1.0 で 180×270）を窓の右下に固定し、
// パネル（幅 260）をその左に下端を揃えて置く。倍率 1.0 の窓の大きさは tauri.conf.json にも書く。
const STAGE_W: f64 = 180.0;
const STAGE_H: f64 = 270.0;
const PANEL_W: f64 = 260.0;
const MARGIN: f64 = 8.0;
// 吹き出しは立ち絵の頭の上に出すので、その分を立ち絵の上に空けておく。
const BUBBLE_ROOM: f64 = 100.0;
// パネルは行の増減や、作業中の一覧を広げたときに上へ伸びる。その最大の高さ。
const PANEL_COLUMN_H: f64 = 380.0;

#[derive(Debug, Serialize, Deserialize)]
struct Display {
    scale: f64,
}

// 手で編集されたり壊れたりした値で窓が極端な大きさにならないよう、範囲外は既定値へ戻す。
pub fn load(home: &MarimoHome) -> f64 {
    fs::read(home.display_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Display>(&bytes).ok())
        .map(|d| d.scale)
        .filter(|s| s.is_finite() && (MIN..=MAX).contains(s))
        .unwrap_or(DEFAULT)
}

pub fn save(home: &MarimoHome, scale: f64) -> io::Result<()> {
    store::write_json_atomic(&home.display_file(), &Display { scale })
}

// ホイールで 0.1 ずつ足すと 1.2000000000000002 のような誤差が積もるので、
// 範囲に収めたうえで小数第 2 位に丸める。
pub fn clamp(scale: f64) -> f64 {
    if !scale.is_finite() {
        return DEFAULT;
    }
    (scale.clamp(MIN, MAX) * 100.0).round() / 100.0
}

// 行と吹き出しの文字は倍率で変えないので、窓は立ち絵が大きくなった分だけ広げる。
pub fn window_size(scale: f64) -> (f64, f64) {
    let width = MARGIN + PANEL_W + MARGIN + STAGE_W * scale + MARGIN;
    let height = PANEL_COLUMN_H.max(MARGIN + STAGE_H * scale + BUBBLE_ROOM);
    (width.round(), height.round())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, MarimoHome) {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        (dir, home)
    }

    #[test]
    fn save_then_load_round_trips() {
        let (_d, home) = home();
        assert_eq!(load(&home), DEFAULT);
        save(&home, 1.7).unwrap();
        assert_eq!(load(&home), 1.7);
        save(&home, MAX).unwrap();
        assert_eq!(load(&home), MAX);
    }

    #[test]
    fn invalid_stored_values_fall_back_to_default() {
        let (_d, home) = home();
        for content in [
            "{broken",
            "",
            "[]",
            r#"{"scale": "big"}"#,
            r#"{"scale": 0.1}"#,
            r#"{"scale": 3.0}"#,
            r#"{"scale": -1}"#,
            r#"{"other": 1.5}"#,
        ] {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(load(&home), DEFAULT, "content {content:?}");
        }
    }

    #[test]
    fn clamp_keeps_requests_in_range() {
        assert_eq!(clamp(0.1), MIN);
        assert_eq!(clamp(9.0), MAX);
        assert_eq!(clamp(f64::NAN), DEFAULT);
        assert_eq!(clamp(1.0 + 0.1 + 0.1), 1.2);
    }

    #[test]
    fn window_grows_with_portrait_only() {
        assert_eq!(window_size(1.0), (464.0, 380.0));
        assert_eq!(window_size(2.5), (734.0, 783.0));
        assert_eq!(window_size(0.6), (392.0, 380.0));
    }
}
