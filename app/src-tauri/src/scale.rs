use std::fs;
use std::io;

use marimo_core::{MarimoHome, store};
use serde::{Deserialize, Serialize};

pub const MIN: f64 = 0.6;
pub const MAX: f64 = 2.5;
pub const DEFAULT: f64 = 1.0;

// 倍率 1.0 のときの大きさ。STAGE は style.css の #stage、WINDOW は tauri.conf.json の
// 窓の大きさと揃えておく必要がある。
const STAGE_W: f64 = 180.0;
const STAGE_H: f64 = 270.0;
const WINDOW_W: f64 = 260.0;
const WINDOW_H: f64 = 440.0;
const SIDE_MARGIN: f64 = 16.0;

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
    let width = WINDOW_W.max(STAGE_W * scale + SIDE_MARGIN * 2.0);
    let height = WINDOW_H + STAGE_H * (scale - 1.0);
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
        assert_eq!(window_size(1.0), (WINDOW_W, WINDOW_H));
        assert_eq!(window_size(2.5), (482.0, 845.0));
        assert_eq!(window_size(0.6), (WINDOW_W, 332.0));
    }
}
