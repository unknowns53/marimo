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
/// 立ち絵の縦横比（高さ ÷ 幅）。既定のキャラクターの素材 800×1200 の比で、比がまだ保存されていないときに窓を開くのに使う。
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

/// パネルの地の不透明度。文字や点は薄くしないので、下限は地が見分けられるだけの濃さにする。
pub const OPACITY_MIN: f64 = 0.2;
pub const OPACITY_MAX: f64 = 1.0;
pub const DEFAULT_OPACITY: f64 = 0.8;

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

/// 並べ方は詳細と件数のどちらの出し方にも効く。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowOrder {
    /// 始まった時刻で並べるので、状態が変わっても行が動かない。
    #[default]
    Started,
    Status,
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

/// 自動で消すまでの秒数の上限。手で書いた極端な値で吹き出しが事実上消えなくならないよう、1 時間に収める。
pub const MAX_BUBBLE_SECONDS: u32 = 3600;

/// 知らせの吹き出しは、承認待ち、完了、エラーの種類ごとに出すかどうかを選べる。
/// 秒数が 0 のときは、押して閉じるか、きっかけが終わるまで出し続ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleSettings {
    pub bubble_waiting: bool,
    pub bubble_done: bool,
    pub bubble_error: bool,
    pub bubble_seconds: u32,
}

impl Default for BubbleSettings {
    fn default() -> Self {
        Self {
            bubble_waiting: true,
            bubble_done: true,
            bubble_error: true,
            bubble_seconds: 0,
        }
    }
}

/// 立ち絵の有無と行の出し方と並べ方と地の不透明度はどれも独立に選べ、どの組み合わせでもパネルは常に出る。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanelDisplay {
    pub show_character: bool,
    #[serde(default)]
    pub show_cloud_sessions: bool,
    pub panel_style: PanelStyle,
    #[serde(default)]
    pub row_order: RowOrder,
    #[serde(default = "default_opacity")]
    pub panel_opacity: f64,
}

fn default_opacity() -> f64 {
    DEFAULT_OPACITY
}

impl Default for PanelDisplay {
    fn default() -> Self {
        Self {
            show_character: true,
            panel_style: PanelStyle::Detail,
            row_order: RowOrder::Started,
            panel_opacity: DEFAULT_OPACITY,
            show_cloud_sessions: false,
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
        panel_opacity: DEFAULT_OPACITY,
        show_cloud_sessions: false,
    })
}

// 組み込みのキャラクターの一覧。フロントエンドもビルド時に写された同じファイルをメニューに使うので、
// ここへ埋め込めば、保存してよい名前の一覧が両者でずれない。先頭が既定のキャラクターになる。
const CHARACTER_INDEX: &str = include_str!("../../../assets/character/index.json");
const FALLBACK_CHARACTER: &str = "koharu";

// display.json には倍率とパネルの表示とキャラクターと立ち絵の縦横比を一緒に置く。
// 一つを保存するときにほかを消さないよう、読んでから書き戻す。
#[derive(Debug, Default, Serialize, Deserialize)]
struct Display {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_character: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_cloud_sessions: Option<bool>,
    // 知らない値が書かれていてもファイル全体を読み損ねないよう、文字列のまま読んでから解釈する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    row_order: Option<String>,
    // 手で "80%" のように書かれても倍率やキャラクターまで読み損ねないよう、数に限らず受けてから解釈する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_opacity: Option<serde_json::Value>,
    // 吹き出しの設定も、手で書き損じた値がほかの設定まで巻き込まないよう、型を決めずに受けてから解釈する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bubble_waiting: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bubble_done: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bubble_error: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bubble_seconds: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    character: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stage_aspect: Option<f64>,
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
/// 新しい鍵が一部だけのときは、残りを panel_mode の読み替えか既定で補う。並べ方と不透明度は
/// panel_mode の頃には無かったので、無いか解釈できない値なら既定にする。
pub fn load_panel_display(home: &MarimoHome) -> Option<PanelDisplay> {
    let d = read_display(home);
    let style = d.panel_style.as_deref().and_then(PanelStyle::parse);
    let order = d.row_order.as_deref().and_then(RowOrder::parse);
    let opacity = d.panel_opacity.as_ref().and_then(serde_json::Value::as_f64);
    let legacy = d.panel_mode.as_deref().and_then(legacy_panel_mode);
    if d.show_cloud_sessions.is_none()
        && d.show_character.is_none()
        && style.is_none()
        && order.is_none()
        && opacity.is_none()
        && legacy.is_none()
    {
        return None;
    }
    let fallback = legacy.unwrap_or_default();
    Some(PanelDisplay {
        show_character: d.show_character.unwrap_or(fallback.show_character),
        show_cloud_sessions: d.show_cloud_sessions.unwrap_or(false),
        panel_style: style.unwrap_or(fallback.panel_style),
        row_order: order.unwrap_or_default(),
        panel_opacity: opacity.map_or(DEFAULT_OPACITY, clamp_opacity),
    })
}

pub fn snapshot(home: &MarimoHome) -> marimo_core::Snapshot {
    store::load_snapshot_with_cloud(
        home,
        load_panel_display(home)
            .unwrap_or_default()
            .show_cloud_sessions,
        marimo_core::time::now_ms(),
    )
}

pub fn save_panel_display(home: &MarimoHome, display: PanelDisplay) -> io::Result<()> {
    let mut d = read_display(home);
    d.show_character = Some(display.show_character);
    d.show_cloud_sessions = Some(display.show_cloud_sessions);
    d.panel_style = Some(
        match display.panel_style {
            PanelStyle::Detail => "detail",
            PanelStyle::Counts => "counts",
        }
        .to_owned(),
    );
    d.row_order = Some(display.row_order.as_str().to_owned());
    d.panel_opacity = Some(clamp_opacity(display.panel_opacity).into());
    d.panel_mode = None;
    store::write_json_atomic(&home.display_file(), &d)
}

/// 保存されていない項目や解釈できない値は、既定（種類は出す、秒数は押すまで）にする。
pub fn load_bubble_settings(home: &MarimoHome) -> BubbleSettings {
    let d = read_display(home);
    let flag = |v: &Option<serde_json::Value>| {
        v.as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    };
    BubbleSettings {
        bubble_waiting: flag(&d.bubble_waiting),
        bubble_done: flag(&d.bubble_done),
        bubble_error: flag(&d.bubble_error),
        bubble_seconds: d
            .bubble_seconds
            .as_ref()
            .and_then(serde_json::Value::as_u64)
            .map_or(0, |s| s.min(u64::from(MAX_BUBBLE_SECONDS)) as u32),
    }
}

pub fn save_bubble_settings(home: &MarimoHome, settings: BubbleSettings) -> io::Result<()> {
    let mut d = read_display(home);
    d.bubble_waiting = Some(settings.bubble_waiting.into());
    d.bubble_done = Some(settings.bubble_done.into());
    d.bubble_error = Some(settings.bubble_error.into());
    d.bubble_seconds = Some(settings.bubble_seconds.min(MAX_BUBBLE_SECONDS).into());
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

/// 組み込みの一覧から外れた名前が残っていても立ち絵の無い状態で起動しないよう、既定のキャラクターへ戻す。
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

// 縦横比は選んだキャラクターの manifest にあり、読めるのはフロントエンドだけである。起動のたびに
// 既定の比で窓を開いてから右下を保って縮めると、保存した左上がその差だけずれていくので、前回知らされた比を
// 残しておき、窓を最初からその大きさで開く。
pub fn load_aspect(home: &MarimoHome) -> f64 {
    read_display(home)
        .stage_aspect
        .map_or(DEFAULT_ASPECT, clamp_aspect)
}

pub fn save_aspect(home: &MarimoHome, aspect: f64) -> io::Result<()> {
    let mut d = read_display(home);
    d.stage_aspect = Some(clamp_aspect(aspect));
    store::write_json_atomic(&home.display_file(), &d)
}

pub fn clamp_aspect(aspect: f64) -> f64 {
    if !aspect.is_finite() || aspect <= 0.0 {
        return DEFAULT_ASPECT;
    }
    aspect.clamp(MIN_ASPECT, MAX_ASPECT)
}

// 手で書いた範囲外の値は、既定へ戻すより近い端へ寄せる方が書いた意図に近い。スライダーの 5% 刻みを
// 小数で表すと 0.35000000000000003 のような誤差が出るので、小数第 2 位に丸める。
pub fn clamp_opacity(opacity: f64) -> f64 {
    if !opacity.is_finite() {
        return DEFAULT_OPACITY;
    }
    (opacity.clamp(OPACITY_MIN, OPACITY_MAX) * 100.0).round() / 100.0
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
            r#"{"stage_aspect": 0}"#,
            r#"{"stage_aspect": -1.5}"#,
        ] {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(
                (load(&home), load_aspect(&home)),
                (DEFAULT, DEFAULT_ASPECT),
                "content {content:?}"
            );
        }
    }

    fn display(show_character: bool, panel_style: PanelStyle) -> PanelDisplay {
        PanelDisplay {
            show_character,
            panel_style,
            row_order: RowOrder::Started,
            panel_opacity: DEFAULT_OPACITY,
            show_cloud_sessions: false,
        }
    }

    #[test]
    fn display_settings_share_one_file() {
        let (_d, home) = home();
        assert_eq!(load(&home), DEFAULT);
        assert_eq!(load_panel_display(&home), None);
        let counts = display(true, PanelStyle::Counts);
        save(&home, 1.5).unwrap();
        save_panel_display(&home, counts).unwrap();
        assert_eq!(
            (load(&home), load_panel_display(&home)),
            (1.5, Some(counts))
        );
        save(&home, 2.0).unwrap();
        save_character(&home, "clawd").unwrap();
        assert_eq!(
            (load_panel_display(&home), load_character(&home).as_str()),
            (Some(counts), "clawd")
        );

        save_aspect(&home, 1.0).unwrap();
        assert_eq!(
            (
                load(&home),
                load_panel_display(&home),
                load_aspect(&home),
                load_character(&home).as_str()
            ),
            (2.0, Some(counts), 1.0, "clawd")
        );
        save(&home, 2.5).unwrap();
        save_panel_display(&home, display(false, PanelStyle::Detail)).unwrap();
        assert_eq!(load(&home), MAX);
        assert_eq!(
            load_panel_display(&home),
            Some(display(false, PanelStyle::Detail))
        );
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
                    panel_opacity: DEFAULT_OPACITY,
                    show_cloud_sessions: false,
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
            serde_json::json!({"scale": 1.3, "show_character": true, "show_cloud_sessions": false, "panel_style": "detail", "row_order": "status", "panel_opacity": 0.8})
        );
    }

    #[test]
    fn cloud_toggle_preserves_other_display_settings() {
        let (_dir, home) = home();
        fs::write(
            home.display_file(),
            r#"{"scale":1.3,"character":"koharu","panel_style":"counts"}"#,
        )
        .unwrap();
        assert!(!load_panel_display(&home).unwrap().show_cloud_sessions);
        for enabled in [true, false, true] {
            let mut current = load_panel_display(&home).unwrap();
            current.show_cloud_sessions = enabled;
            save_panel_display(&home, current).unwrap();
            assert_eq!(
                load_panel_display(&home).unwrap().show_cloud_sessions,
                enabled
            );
            assert_eq!(load(&home), 1.3);
            assert_eq!(load_character(&home), "koharu");
            assert_eq!(
                load_panel_display(&home).unwrap().panel_style,
                PanelStyle::Counts
            );
        }
    }

    #[test]
    fn panel_opacity_falls_back_and_clamps() {
        let (_d, home) = home();
        // 二つ目は内容を書いたあとに保存する不透明度で、None のときは保存せずに読むだけにする。
        let cases = [
            (r#"{"show_character": true}"#, None, Some(DEFAULT_OPACITY)),
            (r#"{"panel_opacity": 0.5}"#, None, Some(0.5)),
            (r#"{"panel_opacity": 1}"#, None, Some(1.0)),
            (r#"{"panel_opacity": 0.05}"#, None, Some(OPACITY_MIN)),
            (r#"{"panel_opacity": -3}"#, None, Some(OPACITY_MIN)),
            (r#"{"panel_opacity": 7.5}"#, None, Some(OPACITY_MAX)),
            (r#"{"panel_opacity": 0.456}"#, None, Some(0.46)),
            (
                r#"{"show_character": false, "panel_opacity": "80%"}"#,
                None,
                Some(DEFAULT_OPACITY),
            ),
            (
                r#"{"panel_style": "counts", "panel_opacity": null}"#,
                None,
                Some(DEFAULT_OPACITY),
            ),
            (r#"{"panel_opacity": "80%"}"#, None, None),
            (r#"{"scale": 1.2}"#, None, None),
            (r#"{"scale": 1.2}"#, Some(0.35000000000000003), Some(0.35)),
            (r#"{"panel_opacity": 0.5}"#, Some(0.0), Some(OPACITY_MIN)),
            (
                r#"{"panel_opacity": 0.5}"#,
                Some(f64::NAN),
                Some(DEFAULT_OPACITY),
            ),
            (r#"{"panel_opacity": 0.5}"#, Some(1.2), Some(OPACITY_MAX)),
        ];
        for (content, saved, expected) in cases {
            fs::write(home.display_file(), content).unwrap();
            if let Some(opacity) = saved {
                let current = load_panel_display(&home).unwrap_or_default();
                save_panel_display(
                    &home,
                    PanelDisplay {
                        panel_opacity: opacity,
                        ..current
                    },
                )
                .unwrap();
            }
            assert_eq!(
                load_panel_display(&home).map(|d| d.panel_opacity),
                expected,
                "content {content:?} saved {saved:?}"
            );
        }
        // 解釈できない不透明度があっても、同じファイルのほかの設定は読める。
        fs::write(
            home.display_file(),
            r#"{"scale": 1.5, "character": "clawd", "panel_opacity": "80%"}"#,
        )
        .unwrap();
        assert_eq!(
            (load(&home), load_character(&home).as_str()),
            (1.5, "clawd")
        );
    }

    #[test]
    fn bubble_settings_fall_back_and_share_the_file() {
        let (_d, home) = home();
        assert_eq!(load_bubble_settings(&home), BubbleSettings::default());
        let cases = [
            (r#"{"bubble_done": false}"#, (true, false, true, 0)),
            (
                r#"{"bubble_waiting": false, "bubble_error": false, "bubble_seconds": 30}"#,
                (false, true, false, 30),
            ),
            (
                r#"{"bubble_seconds": 99999}"#,
                (true, true, true, MAX_BUBBLE_SECONDS),
            ),
            // 解釈できない値は、ほかの設定を巻き込まずに既定へ戻る。
            (
                r#"{"bubble_waiting": "no", "bubble_seconds": -5, "scale": 1.5}"#,
                (true, true, true, 0),
            ),
            (r#"{"bubble_seconds": 2.5}"#, (true, true, true, 0)),
        ];
        for (content, (waiting, done, error, seconds)) in cases {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(
                load_bubble_settings(&home),
                BubbleSettings {
                    bubble_waiting: waiting,
                    bubble_done: done,
                    bubble_error: error,
                    bubble_seconds: seconds,
                },
                "content {content:?}"
            );
        }

        fs::write(
            home.display_file(),
            r#"{"scale": 1.5, "panel_style": "counts"}"#,
        )
        .unwrap();
        let settings = BubbleSettings {
            bubble_done: false,
            bubble_seconds: 60,
            ..BubbleSettings::default()
        };
        save_bubble_settings(&home, settings).unwrap();
        save_panel_display(&home, display(true, PanelStyle::Detail)).unwrap();
        assert_eq!(load_bubble_settings(&home), settings);
        assert_eq!(load(&home), 1.5);
    }

    #[test]
    fn character_choice_falls_back_to_default() {
        let (_d, home) = home();
        assert_eq!(default_character(), "koharu");
        assert_eq!(load_character(&home), "koharu");
        for (content, expected) in [
            (r#"{"character": "koharu"}"#, "koharu"),
            // Display が知らない鍵を含むファイルでも、ほかの鍵は読める。
            (
                r#"{"character": "clawd", "scale": 0.6, "usage_api": true}"#,
                "clawd",
            ),
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
    fn unknown_character_is_not_saved() {
        let (_d, home) = home();
        save_character(&home, "clawd").unwrap();
        assert!(save_character(&home, "bogus").is_err());
        assert_eq!(load_character(&home), "clawd");
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
