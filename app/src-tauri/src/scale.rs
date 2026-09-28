use std::fs;
use std::io;

use marimo_core::{MarimoHome, store};
use serde::{Deserialize, Serialize};

pub const MIN: f64 = 0.6;
pub const MAX: f64 = 2.5;
pub const DEFAULT: f64 = 1.0;

// style.css の配置と揃えておく必要がある。立ち絵（倍率 1.0 で幅 180、高さは幅に素材の縦横比を
// 掛けたもの）を窓の右下に固定し、パネル（幅 312）をその左に下端を揃えて置く。倍率 1.0 で
// 既定のキャラクターのときの窓の大きさは tauri.conf.json にも書く。
const STAGE_W: f64 = 180.0;
/// 立ち絵の縦横比（高さ ÷ 幅）。既定のキャラクターの素材 800×1200 の比で、窓を最初に開くときに使う。
pub const DEFAULT_ASPECT: f64 = 1.5;
// 手で書き換えた manifest の極端な縦横比で、窓が画面を覆うほど大きくならないようにする。
const MIN_ASPECT: f64 = 0.25;
const MAX_ASPECT: f64 = 4.0;
const PANEL_W: f64 = 312.0;
const MARGIN: f64 = 8.0;
// 吹き出しは立ち絵の頭の上に出すので、その分を立ち絵の上に空けておく。
const BUBBLE_ROOM: f64 = 120.0;
// パネルは行の増減や、件数の行に詳細を重ねたときに上へ伸びる。その最大の高さ。
const PANEL_COLUMN_H: f64 = 380.0;

/// パネルの行の出し方。行を 1 セッションずつ並べる詳細か、状態ごとの件数の 1 行に畳むか。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelStyle {
    #[default]
    Detail,
    Counts,
}

impl PanelStyle {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "detail" => Some(Self::Detail),
            "counts" => Some(Self::Counts),
            _ => None,
        }
    }
}

/// パネルの行の並べ方。詳細と件数のどちらの出し方にも効く。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowOrder {
    /// 始まった時刻の新しい順。状態が変わっても行が動かない。
    #[default]
    Started,
    /// 状態の優先度の順。
    Status,
    /// 最後の更新の新しい順。
    Updated,
}

impl RowOrder {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "started" => Some(Self::Started),
            "status" => Some(Self::Status),
            "updated" => Some(Self::Updated),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Status => "status",
            Self::Updated => "updated",
        }
    }
}

/// 立ち絵を出すかどうかと、パネルの行の出し方と並べ方。どれも独立に選べ、パネルはどれでも常に出る。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelDisplay {
    pub show_character: bool,
    pub panel_style: PanelStyle,
    #[serde(default)]
    pub row_order: RowOrder,
}

impl Default for PanelDisplay {
    fn default() -> Self {
        Self {
            show_character: true,
            panel_style: PanelStyle::Detail,
            row_order: RowOrder::Started,
        }
    }
}

// 以前の版は、立ち絵の有無と行の出し方を混ぜた四つの段階を panel_mode に一つだけ持っていた。
// 行を出さない「絵だけ」は選べなくなったので、立ち絵を残して最も場所を取らない件数だけへ読み替える。
fn legacy_panel_mode(mode: &str) -> Option<PanelDisplay> {
    let (show_character, panel_style) = match mode {
        "detail" => (true, PanelStyle::Detail),
        "counts" | "picture" => (true, PanelStyle::Counts),
        "list" => (false, PanelStyle::Detail),
        _ => return None,
    };
    Some(PanelDisplay {
        show_character,
        panel_style,
        row_order: RowOrder::default(),
    })
}

// 組み込みのキャラクターの一覧。フロントエンドもビルド時に写された同じファイルをメニューに使うので、
// ここへ埋め込めば、保存してよい名前の一覧が両者でずれない。先頭が既定のキャラクターになる。
const CHARACTER_INDEX: &str = include_str!("../../../assets/character/index.json");
const FALLBACK_CHARACTER: &str = "koharu";

// display.json には倍率とパネルの表示と利用制限の取得元とキャラクターを一緒に置く。一つを保存するときに
// ほかを消さないよう、読んでから書き戻す。
#[derive(Debug, Default, Serialize, Deserialize)]
struct Display {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_character: Option<bool>,
    // 知らない値が書かれていてもファイル全体を読み損ねないよう、文字列のまま読んでから解釈する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    row_order: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage_api: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    character: Option<String>,
}

fn read_display(home: &MarimoHome) -> Display {
    fs::read(home.display_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Display>(&bytes).ok())
        .unwrap_or_default()
}

// 手で編集されたり壊れたりした値で窓が極端な大きさにならないよう、範囲外は既定値へ戻す。
pub fn load(home: &MarimoHome) -> f64 {
    read_display(home)
        .scale
        .filter(|s| s.is_finite() && (MIN..=MAX).contains(s))
        .unwrap_or(DEFAULT)
}

pub fn save(home: &MarimoHome, scale: f64) -> io::Result<()> {
    let mut d = read_display(home);
    d.scale = Some(scale);
    store::write_json_atomic(&home.display_file(), &d)
}

/// 新しい鍵も以前の panel_mode も保存されていなければ None を返し、フロントエンドに既定を決めさせる。
/// 新しい鍵が一部だけのときは、残りを panel_mode の読み替えか既定で補う。並べ方は panel_mode の
/// 頃には無かったので、無いか知らない値なら始まった順にする。
pub fn load_panel_display(home: &MarimoHome) -> Option<PanelDisplay> {
    let d = read_display(home);
    let style = d.panel_style.as_deref().and_then(PanelStyle::parse);
    let order = d.row_order.as_deref().and_then(RowOrder::parse);
    let legacy = d.panel_mode.as_deref().and_then(legacy_panel_mode);
    if d.show_character.is_none() && style.is_none() && order.is_none() && legacy.is_none() {
        return None;
    }
    let fallback = legacy.unwrap_or_default();
    Some(PanelDisplay {
        show_character: d.show_character.unwrap_or(fallback.show_character),
        panel_style: style.unwrap_or(fallback.panel_style),
        row_order: order.unwrap_or_default(),
    })
}

pub fn save_panel_display(home: &MarimoHome, display: PanelDisplay) -> io::Result<()> {
    let mut d = read_display(home);
    d.show_character = Some(display.show_character);
    d.panel_style = Some(
        match display.panel_style {
            PanelStyle::Detail => "detail",
            PanelStyle::Counts => "counts",
        }
        .to_owned(),
    );
    d.row_order = Some(display.row_order.as_str().to_owned());
    d.panel_mode = None;
    store::write_json_atomic(&home.display_file(), &d)
}

// 利用者の資格情報を読むので、明示して有効にしたときだけ API から取る。
pub fn load_usage_api(home: &MarimoHome) -> bool {
    read_display(home).usage_api.unwrap_or(false)
}

pub fn save_usage_api(home: &MarimoHome, enabled: bool) -> io::Result<()> {
    let mut d = read_display(home);
    d.usage_api = Some(enabled);
    store::write_json_atomic(&home.display_file(), &d)
}

fn characters() -> Vec<String> {
    serde_json::from_str(CHARACTER_INDEX).unwrap_or_default()
}

pub fn default_character() -> String {
    characters()
        .into_iter()
        .next()
        .unwrap_or_else(|| FALLBACK_CHARACTER.to_owned())
}

/// 保存されていない、または一覧に無い名前なら既定のキャラクターを返す。
pub fn load_character(home: &MarimoHome) -> String {
    read_display(home)
        .character
        .filter(|c| characters().contains(c))
        .unwrap_or_else(default_character)
}

pub fn save_character(home: &MarimoHome, id: &str) -> io::Result<()> {
    if !characters().iter().any(|c| c == id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown character",
        ));
    }
    let mut d = read_display(home);
    d.character = Some(id.to_owned());
    store::write_json_atomic(&home.display_file(), &d)
}

pub fn clamp_aspect(aspect: f64) -> f64 {
    if !aspect.is_finite() || aspect <= 0.0 {
        return DEFAULT_ASPECT;
    }
    aspect.clamp(MIN_ASPECT, MAX_ASPECT)
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
pub fn window_size(scale: f64, aspect: f64) -> (f64, f64) {
    let width = MARGIN + PANEL_W + MARGIN + STAGE_W * scale + MARGIN;
    let height = PANEL_COLUMN_H.max(MARGIN + STAGE_W * aspect * scale + BUBBLE_ROOM);
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

    fn display(show_character: bool, panel_style: PanelStyle) -> PanelDisplay {
        PanelDisplay {
            show_character,
            panel_style,
            row_order: RowOrder::Started,
        }
    }

    #[test]
    fn display_settings_share_one_file() {
        let (_d, home) = home();
        assert_eq!(load(&home), DEFAULT);
        assert_eq!(load_panel_display(&home), None);
        assert!(!load_usage_api(&home));
        let counts = display(true, PanelStyle::Counts);
        save(&home, 1.5).unwrap();
        save_panel_display(&home, counts).unwrap();
        assert_eq!(
            (load(&home), load_panel_display(&home)),
            (1.5, Some(counts))
        );
        save(&home, 2.0).unwrap();
        assert_eq!(load_panel_display(&home), Some(counts));

        save_usage_api(&home, true).unwrap();
        assert!(load_usage_api(&home));
        assert_eq!(
            (load(&home), load_panel_display(&home)),
            (2.0, Some(counts))
        );
        save(&home, 2.5).unwrap();
        save_panel_display(&home, display(false, PanelStyle::Detail)).unwrap();
        assert!(load_usage_api(&home));
        save_usage_api(&home, false).unwrap();
        assert!(!load_usage_api(&home));
        assert_eq!(load(&home), MAX);
        assert_eq!(
            load_panel_display(&home),
            Some(display(false, PanelStyle::Detail))
        );

        for content in [r#"{"usage_api": "yes"}"#, "{broken", r#"{"scale": 1.2}"#] {
            fs::write(home.display_file(), content).unwrap();
            assert!(!load_usage_api(&home), "content {content:?}");
        }
    }

    #[test]
    fn panel_display_reads_legacy_panel_mode_until_saved() {
        use PanelStyle::{Counts, Detail};
        use RowOrder::{Started, Status, Updated};
        let (_d, home) = home();
        let cases = [
            (r#"{"panel_mode": "detail"}"#, Some((true, Detail, Started))),
            (r#"{"panel_mode": "counts"}"#, Some((true, Counts, Started))),
            (r#"{"panel_mode": "list"}"#, Some((false, Detail, Started))),
            (
                r#"{"panel_mode": "picture"}"#,
                Some((true, Counts, Started)),
            ),
            (r#"{"scale": 1.2, "panel_mode": "tiny"}"#, None),
            (r#"{"scale": 1.7}"#, None),
            (r#"{"panel_style": "bogus"}"#, None),
            (r#"{"row_order": "bogus"}"#, None),
            // 新しい鍵は panel_mode より優先し、無い方だけを panel_mode の読み替えで補う。
            (
                r#"{"panel_mode": "list", "show_character": true, "panel_style": "counts"}"#,
                Some((true, Counts, Started)),
            ),
            (
                r#"{"panel_mode": "list", "panel_style": "counts"}"#,
                Some((false, Counts, Started)),
            ),
            (
                r#"{"show_character": false}"#,
                Some((false, Detail, Started)),
            ),
            (
                r#"{"panel_mode": "picture", "panel_style": "bogus"}"#,
                Some((true, Counts, Started)),
            ),
            (r#"{"row_order": "status"}"#, Some((true, Detail, Status))),
            (
                r#"{"panel_mode": "list", "row_order": "updated"}"#,
                Some((false, Detail, Updated)),
            ),
            (
                r#"{"show_character": true, "panel_style": "counts", "row_order": "bogus"}"#,
                Some((true, Counts, Started)),
            ),
        ];
        for (content, expected) in cases {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(
                load_panel_display(&home),
                expected.map(|(show_character, panel_style, row_order)| PanelDisplay {
                    show_character,
                    panel_style,
                    row_order,
                }),
                "content {content:?}"
            );
        }

        fs::write(
            home.display_file(),
            r#"{"scale": 1.3, "panel_mode": "list"}"#,
        )
        .unwrap();
        save_panel_display(
            &home,
            PanelDisplay {
                row_order: Status,
                ..display(true, Detail)
            },
        )
        .unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(home.display_file()).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({"scale": 1.3, "show_character": true, "panel_style": "detail", "row_order": "status"})
        );
    }

    #[test]
    fn character_choice_falls_back_to_default() {
        let (_d, home) = home();
        assert_eq!(default_character(), "koharu");
        assert_eq!(load_character(&home), "koharu");
        save(&home, 1.5).unwrap();
        save_character(&home, "clawd").unwrap();
        save_panel_display(&home, display(false, PanelStyle::Counts)).unwrap();
        assert_eq!(
            (load_character(&home).as_str(), load(&home)),
            ("clawd", 1.5)
        );
        assert!(save_character(&home, "bogus").is_err());
        assert_eq!(load_character(&home), "clawd");

        for (content, expected) in [
            (r#"{"character": "koharu"}"#, "koharu"),
            (r#"{"character": "clawd", "scale": 0.6}"#, "clawd"),
            (r#"{"character": "missing"}"#, "koharu"),
            (r#"{"character": 3}"#, "koharu"),
            (r#"{"scale": 1.2}"#, "koharu"),
            ("{broken", "koharu"),
        ] {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(load_character(&home), expected, "content {content:?}");
        }
    }

    #[test]
    fn window_follows_stage_aspect() {
        // 倍率 1.0 の既定のキャラクターの大きさは tauri.conf.json の窓の大きさと同じになる。
        for (scale, aspect, size) in [
            (1.0, DEFAULT_ASPECT, (516.0, 398.0)),
            (2.5, DEFAULT_ASPECT, (786.0, 803.0)),
            (1.0, 0.5, (516.0, 380.0)),
            (2.5, 0.5, (786.0, 380.0)),
            (1.0, f64::NAN, (516.0, 398.0)),
            (1.0, 0.0, (516.0, 398.0)),
            (1.0, 100.0, (516.0, 848.0)),
        ] {
            assert_eq!(
                window_size(scale, clamp_aspect(aspect)),
                size,
                "scale {scale} aspect {aspect}"
            );
        }
    }

    #[test]
    fn clamp_keeps_requests_in_range() {
        assert_eq!(clamp(0.1), MIN);
        assert_eq!(clamp(9.0), MAX);
        assert_eq!(clamp(f64::NAN), DEFAULT);
        assert_eq!(clamp(1.0 + 0.1 + 0.1), 1.2);
    }
}
