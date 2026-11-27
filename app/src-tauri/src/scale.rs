use std::fs;
use std::io;
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use marimo_core::{LangSetting, MarimoHome, store};
use serde::{Deserialize, Serialize};

use crate::window_pos::{Corner, StageRect};

pub const MIN: f64 = 0.6;
pub const MAX: f64 = 4.0;
pub const DEFAULT: f64 = 1.0;

// style.css の配置と揃えておく必要があり、window_follows_stage_aspect が照合する。立ち絵（倍率 1.0 で
// 幅 108、高さは幅に素材の縦横比を掛けたもの）をウィンドウの隅の一つに固定し、パネル（幅 312）をその横に置く。
// 倍率 1.0 で既定のキャラクターのときのウィンドウの大きさは tauri.conf.json にも書く。
const STAGE_W: f64 = 108.0;
// display.json の scale のキーは、立ち絵の領域の幅を 180 とした倍率で、0.6 から 2.5 の範囲で書かれている。
// stage_scale がまだ無いときは、同じ大きさで開くよう、幅の比を掛けて今の倍率へ読み替える。
const LEGACY_STAGE_W: f64 = 180.0;
const LEGACY_MIN: f64 = 0.6;
const LEGACY_MAX: f64 = 2.5;
/// 立ち絵の縦横比（高さ ÷ 幅）。既定のキャラクターの素材 800×1200 の比で、比がまだ保存されていないときにウィンドウを開くのに使う。
pub const DEFAULT_ASPECT: f64 = 1.5;
// 手で書き換えた manifest の極端な縦横比で、ウィンドウが画面を覆うほど大きくならないようにする。
const MIN_ASPECT: f64 = 0.25;
const MAX_ASPECT: f64 = 4.0;
const PANEL_W: f64 = 312.0;
const MARGIN: f64 = 8.0;
// 吹き出しは立ち絵の頭の上に出すので、その分を立ち絵の上に空けておく。上の隅に置くときも、立ち絵の上端は
// ウィンドウの上端からこれだけ下にする。
const BUBBLE_ROOM: f64 = 120.0;
// パネルは行の増減で上へ伸びる。パネルの倍率 1.0 での最大の高さで、倍率に合わせて伸び縮みさせ、
// 倍率を変えても並ぶ行の数が変わらないようにする。
const PANEL_COLUMN_H: f64 = 380.0;

/// パネルの背景の不透明度。文字や点は薄くしないので、下限は背景が見分けられるだけの濃さにする。
pub const OPACITY_MIN: f64 = 0.2;
pub const OPACITY_MAX: f64 = 1.0;
pub const DEFAULT_OPACITY: f64 = 0.8;

/// パネルの文字と幅の倍率。行の文字が読める小ささから、画面の隅に収まる大きさまでにする。
pub const PANEL_SCALE_MIN: f64 = 0.8;
pub const PANEL_SCALE_MAX: f64 = 1.3;
pub const DEFAULT_PANEL_SCALE: f64 = 1.0;

/// 行の状態の色の濃さに掛ける倍率。上限は、手を貸す必要のある行の色（style.css の --tint-strength）に掛けても
/// 100% を超えない値にする。下限でも状態の色を見分けられるようにする。
pub const TINT_MIN: f64 = 0.4;
pub const TINT_MAX: f64 = 2.0;
pub const DEFAULT_TINT: f64 = 1.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelStyle {
    #[default]
    Detail,
    /// パネルを隠す。立ち絵も隠しているときは、利用制限の欄だけを残す。
    Collapsed,
}

impl PanelStyle {
    // 以前の「件数だけ」は、リストを 1 行に畳んで場所を空けるものだったので、パネルを隠す表示へ読み替える。
    fn parse(s: &str) -> Option<Self> {
        match s {
            "detail" => Some(Self::Detail),
            "collapsed" | "counts" => Some(Self::Collapsed),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Detail => "detail",
            Self::Collapsed => "collapsed",
        }
    }
}

/// 並べ方は行の出し方によらず効く。
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
    pub const ALL: [Self; 3] = [Self::Started, Self::Status, Self::Updated];

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "started" => Some(Self::Started),
            "status" => Some(Self::Status),
            "updated" => Some(Self::Updated),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Status => "status",
            Self::Updated => "updated",
        }
    }
}

/// 自動で消すまでの秒数の上限。手で書いた極端な値で吹き出しが事実上消えなくならないよう、1 時間に収める。
pub const MAX_BUBBLE_SECONDS: u32 = 3600;

/// Claude Code と Codex に利用制限を再取得させる間隔の上限（分）。
pub const MAX_USAGE_REFRESH_MINUTES: u32 = 1440;

/// 知らせの吹き出しは、承認待ち、完了、エラーの種類ごとに出すかどうかを選べる。
/// 秒数が 0 のときは、押して閉じるか、トリガーが終わるまで出し続ける。
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

/// パネルの利用制限の行に値を出すツール。出さないツールには利用制限を再取得させない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTools {
    pub claude: bool,
    pub codex: bool,
    pub antigravity: bool,
}

/// 任意のグローバルショートカット。空または None の項目は登録しない（既定はどちらも無効）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcuts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_urgent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle_visible: Option<String>,
}

impl Default for UsageTools {
    fn default() -> Self {
        Self {
            claude: true,
            codex: true,
            antigravity: true,
        }
    }
}

/// 立ち絵の有無、行の出し方、並べ方、地の不透明度、パネルの倍率、状態の色の濃さ、数値の出し方、隠すときに利用制限の欄を
/// 残すかはどれも独立に選べる。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanelDisplay {
    pub show_character: bool,
    pub panel_style: PanelStyle,
    #[serde(default)]
    pub row_order: RowOrder,
    #[serde(default = "default_opacity")]
    pub panel_opacity: f64,
    #[serde(default = "default_panel_scale")]
    pub panel_scale: f64,
    #[serde(default = "default_tint")]
    pub tint_strength: f64,
    /// コンテキスト使用率と利用制限の数値を、ツールチップだけでなくバーの横にも出す。
    #[serde(default)]
    pub show_numbers: bool,
    /// パネルを隠すときも、立ち絵の有無によらず利用制限の欄を残す。
    #[serde(default)]
    pub hide_keeps_limits: bool,
}

fn default_opacity() -> f64 {
    DEFAULT_OPACITY
}

fn default_panel_scale() -> f64 {
    DEFAULT_PANEL_SCALE
}

fn default_tint() -> f64 {
    DEFAULT_TINT
}

impl PanelDisplay {
    pub fn frame(&self) -> Frame {
        Frame {
            show_character: self.show_character,
            panel_scale: clamp_panel_scale(self.panel_scale),
        }
    }
}

/// ウィンドウの大きさを決める表示の設定。立ち絵を隠すとパネルの幅へ詰め、パネルの倍率で幅と高さが変わる。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub show_character: bool,
    pub panel_scale: f64,
}

impl Default for PanelDisplay {
    fn default() -> Self {
        Self {
            show_character: true,
            panel_style: PanelStyle::Detail,
            row_order: RowOrder::Started,
            panel_opacity: DEFAULT_OPACITY,
            panel_scale: DEFAULT_PANEL_SCALE,
            tint_strength: DEFAULT_TINT,
            show_numbers: false,
            hide_keeps_limits: false,
        }
    }
}

// 以前のバージョンは、立ち絵の有無と行の出し方を混ぜた四つの段階を panel_mode に一つだけ持っていた。
// 「件数だけ」と行を出さない「絵だけ」は、立ち絵を残してパネルを隠す表示へ読み替える。
fn legacy_panel_mode(mode: &str) -> Option<PanelDisplay> {
    let (show_character, panel_style) = match mode {
        "detail" => (true, PanelStyle::Detail),
        "counts" | "picture" => (true, PanelStyle::Collapsed),
        "list" => (false, PanelStyle::Detail),
        _ => return None,
    };
    Some(PanelDisplay {
        show_character,
        panel_style,
        ..PanelDisplay::default()
    })
}

// 組み込みのキャラクターの一覧。フロントエンドもビルド時にコピーされた同じファイルをメニューに使うので、
// ここへ埋め込めば、保存してよい名前の一覧が両者でずれない。先頭が既定のキャラクターになる。
const CHARACTER_INDEX: &str = include_str!("../../../assets/character/index.json");
const FALLBACK_CHARACTER: &str = "koharu";

// display.json には倍率とパネルの表示とキャラクターと立ち絵の縦横比を一緒に置く。
// 一つを保存するときにほかを消さないよう、読んでから書き戻す。型を決めて読む項目は lenient で読むので、
// 一つの項目の型が違っても、その項目だけが書かれていないものとして読まれる。
#[derive(Debug, Default, Serialize, Deserialize)]
struct Display {
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    stage_scale: Option<f64>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    scale: Option<f64>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    show_character: Option<bool>,
    // 知らない値が書かれていてもファイル全体を読み損ねないよう、文字列のまま読んでから解釈する。
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    panel_style: Option<String>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    panel_mode: Option<String>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    row_order: Option<String>,
    // 手で "80%" のように書かれても倍率やキャラクターまで読み損ねないよう、数に限らず受けてから解釈する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_opacity: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_scale: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tint_strength: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_numbers: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hide_keeps_limits: Option<serde_json::Value>,
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
    usage_refresh_minutes: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage_tools: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shortcuts: Option<serde_json::Value>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    character: Option<String>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    stage_aspect: Option<f64>,
    // 読むのは marimo_core::LangSetting::load で、フックと CLI も同じ規則で読む。ここに持つのは、
    // ほかの項目を保存するときに消さないためである。
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    language: Option<String>,
    // このバージョンが知らない項目。新しいバージョンが書いた項目を、古いバージョンで保存したときに消さない。
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

/// 型の合わない値を、その項目が書かれていないものとして読む。手で "1.5" のように書き損じた一つの項目で
/// ファイル全体を既定として読み、次の保存でほかの設定まで消してしまわないようにする。
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

// display.json を書くのはこのアプリだけだが、倍率は spawn_saver のスレッドが書き、ほかの項目はメインスレッドが書く。
// 読んでから書き戻すまでの間に別の項目の保存が割り込んで消えないよう、プロセスの中で一つずつにする。
static DISPLAY_LOCK: Mutex<()> = Mutex::new(());

fn update_display(home: &MarimoHome, edit: impl FnOnce(&mut Display)) -> io::Result<()> {
    let _guard = DISPLAY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = read_display(home);
    edit(&mut d);
    store::write_json_durable(&home.display_file(), &d)
}

fn read_display(home: &MarimoHome) -> Display {
    fs::read(home.display_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Display>(&bytes).ok())
        .unwrap_or_default()
}

// 手で編集されたり壊れたりした値でウィンドウが極端な大きさにならないよう、範囲外は既定値へ戻す。
pub fn load(home: &MarimoHome) -> f64 {
    let d = read_display(home);
    d.stage_scale
        .filter(|s| s.is_finite() && (MIN..=MAX).contains(s))
        .or_else(|| {
            d.scale
                .filter(|s| s.is_finite() && (LEGACY_MIN..=LEGACY_MAX).contains(s))
                .map(|s| clamp(s * LEGACY_STAGE_W / STAGE_W))
        })
        .unwrap_or(DEFAULT)
}

pub fn save(home: &MarimoHome, scale: f64) -> io::Result<()> {
    update_display(home, |d| {
        d.stage_scale = Some(scale);
        d.scale = None;
    })
}

// スライダーのドラッグやホイールでは倍率が続けて届く。display.json は write_json_durable で書き、macOS の
// sync_all は F_FULLFSYNC になって一回に数十ミリ秒かかるので、届くたびにメインスレッドで書くとウィンドウの
// 大きさの変更が詰まる。別のスレッドで、この時間届かなくなってから最後の値だけを書く。
const SCALE_SAVE_AFTER: Duration = Duration::from_millis(300);

/// 送られた倍率を、続けて届く間は待ってから保存するスレッドを起こす。
pub fn spawn_saver(home: MarimoHome) -> mpsc::Sender<f64> {
    let (tx, rx) = mpsc::channel::<f64>();
    thread::spawn(move || {
        while let Ok(mut scale) = rx.recv() {
            while let Ok(next) = rx.recv_timeout(SCALE_SAVE_AFTER) {
                scale = next;
            }
            if let Err(e) = save(&home, scale) {
                eprintln!("marimo: cannot save scale: {e}");
            }
        }
    });
    tx
}

/// 新しいキーも以前の panel_mode も保存されていなければ None を返し、フロントエンドに既定を決めさせる。
/// 新しいキーが一部だけのときは、残りを panel_mode の読み替えか既定で補う。並べ方より後に加えた項目は
/// panel_mode の頃には無かったので、無いか解釈できない値なら既定にする。
pub fn load_panel_display(home: &MarimoHome) -> Option<PanelDisplay> {
    let d = read_display(home);
    let style = d.panel_style.as_deref().and_then(PanelStyle::parse);
    let order = d.row_order.as_deref().and_then(RowOrder::parse);
    let opacity = d.panel_opacity.as_ref().and_then(serde_json::Value::as_f64);
    let panel_scale = d.panel_scale.as_ref().and_then(serde_json::Value::as_f64);
    let tint = d.tint_strength.as_ref().and_then(serde_json::Value::as_f64);
    let numbers = d.show_numbers.as_ref().and_then(serde_json::Value::as_bool);
    let keeps_limits = d
        .hide_keeps_limits
        .as_ref()
        .and_then(serde_json::Value::as_bool);
    let legacy = d.panel_mode.as_deref().and_then(legacy_panel_mode);
    if d.show_character.is_none()
        && style.is_none()
        && order.is_none()
        && opacity.is_none()
        && panel_scale.is_none()
        && tint.is_none()
        && numbers.is_none()
        && keeps_limits.is_none()
        && legacy.is_none()
    {
        return None;
    }
    let fallback = legacy.unwrap_or_default();
    Some(PanelDisplay {
        show_character: d.show_character.unwrap_or(fallback.show_character),
        panel_style: style.unwrap_or(fallback.panel_style),
        row_order: order.unwrap_or_default(),
        panel_opacity: opacity.map_or(DEFAULT_OPACITY, clamp_opacity),
        panel_scale: panel_scale.map_or(DEFAULT_PANEL_SCALE, clamp_panel_scale),
        tint_strength: tint.map_or(DEFAULT_TINT, clamp_tint),
        show_numbers: numbers.unwrap_or(false),
        hide_keeps_limits: keeps_limits.unwrap_or(false),
    })
}

pub fn save_panel_display(home: &MarimoHome, display: PanelDisplay) -> io::Result<()> {
    update_display(home, |d| {
        d.show_character = Some(display.show_character);
        d.panel_style = Some(display.panel_style.as_str().to_owned());
        d.row_order = Some(display.row_order.as_str().to_owned());
        d.panel_opacity = Some(clamp_opacity(display.panel_opacity).into());
        d.panel_scale = Some(clamp_panel_scale(display.panel_scale).into());
        d.tint_strength = Some(clamp_tint(display.tint_strength).into());
        d.show_numbers = Some(display.show_numbers.into());
        d.hide_keeps_limits = Some(display.hide_keeps_limits.into());
        d.panel_mode = None;
    })
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
    update_display(home, |d| {
        d.bubble_waiting = Some(settings.bubble_waiting.into());
        d.bubble_done = Some(settings.bubble_done.into());
        d.bubble_error = Some(settings.bubble_error.into());
        d.bubble_seconds = Some(settings.bubble_seconds.min(MAX_BUBBLE_SECONDS).into());
    })
}

/// 0 は再取得しないことを表し、保存されていないときや解釈できない値のときもそうする。Claude Code、
/// Codex、Antigravity を起動する操作は利用者が選んだときだけ行う。
pub fn load_usage_refresh_minutes(home: &MarimoHome) -> u32 {
    read_display(home)
        .usage_refresh_minutes
        .as_ref()
        .and_then(serde_json::Value::as_u64)
        .map_or(0, |m| m.min(u64::from(MAX_USAGE_REFRESH_MINUTES)) as u32)
}

pub fn save_usage_refresh_minutes(home: &MarimoHome, minutes: u32) -> io::Result<()> {
    update_display(home, |d| {
        d.usage_refresh_minutes = Some(minutes.min(MAX_USAGE_REFRESH_MINUTES).into());
    })
}

/// 保存されていないツールや解釈できない値は、出すことにする。
pub fn load_usage_tools(home: &MarimoHome) -> UsageTools {
    let d = read_display(home);
    let shown = |key: &str| {
        d.usage_tools
            .as_ref()
            .and_then(|t| t.get(key))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    };
    UsageTools {
        claude: shown("claude"),
        codex: shown("codex"),
        antigravity: shown("antigravity"),
    }
}

pub fn save_usage_tools(home: &MarimoHome, tools: UsageTools) -> io::Result<()> {
    update_display(home, |d| {
        d.usage_tools = serde_json::to_value(tools).ok();
    })
}

/// 空文字列や解釈できない値は未設定（None）として読む。既定はどちらも無効。
pub fn load_shortcuts(home: &MarimoHome) -> Shortcuts {
    let d = read_display(home);
    let field = |key: &str| {
        d.shortcuts
            .as_ref()
            .and_then(|s| s.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Shortcuts {
        focus_urgent: field("focus_urgent"),
        toggle_visible: field("toggle_visible"),
    }
}

pub fn save_shortcuts(home: &MarimoHome, shortcuts: &Shortcuts) -> io::Result<()> {
    update_display(home, |d| {
        d.shortcuts = serde_json::to_value(shortcuts).ok();
    })
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
    update_display(home, |d| {
        d.character = Some(id.to_owned());
    })
}

// 縦横比は選んだキャラクターの manifest にあり、読めるのはフロントエンドだけである。起動のたびに
// 既定の比でウィンドウを開いてから右下を保って縮めると、保存した左上がその差だけずれていくので、前回知らされた比を
// 残しておき、ウィンドウを最初からその大きさで開く。
pub fn load_aspect(home: &MarimoHome) -> f64 {
    read_display(home)
        .stage_aspect
        .map_or(DEFAULT_ASPECT, clamp_aspect)
}

pub fn save_aspect(home: &MarimoHome, aspect: f64) -> io::Result<()> {
    update_display(home, |d| {
        d.stage_aspect = Some(clamp_aspect(aspect));
    })
}

pub fn save_language(home: &MarimoHome, setting: LangSetting) -> io::Result<()> {
    update_display(home, |d| {
        d.language = Some(setting.as_str().to_owned());
    })
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

// スライダーの 0.05 刻みを小数で表した誤差を残さないよう、不透明度と同じく小数第 2 位に丸める。
pub fn clamp_panel_scale(scale: f64) -> f64 {
    if !scale.is_finite() {
        return DEFAULT_PANEL_SCALE;
    }
    (scale.clamp(PANEL_SCALE_MIN, PANEL_SCALE_MAX) * 100.0).round() / 100.0
}

// スライダーの 10% 刻みを小数で表した誤差を残さないよう、不透明度と同じく小数第 2 位に丸める。
pub fn clamp_tint(tint: f64) -> f64 {
    if !tint.is_finite() {
        return DEFAULT_TINT;
    }
    (tint.clamp(TINT_MIN, TINT_MAX) * 100.0).round() / 100.0
}

// ホイールで 0.1 ずつ足すと 1.2000000000000002 のような誤差が積もるので、
// 範囲に収めたうえで小数第 2 位に丸める。
pub fn clamp(scale: f64) -> f64 {
    if !scale.is_finite() {
        return DEFAULT;
    }
    (scale.clamp(MIN, MAX) * 100.0).round() / 100.0
}

// 行と吹き出しの文字は立ち絵の倍率で変えないので、ウィンドウは立ち絵が大きくなった分と、パネルの倍率で
// パネルが広がった分だけ広げる。立ち絵を隠しているときはパネルの幅だけに詰め、画面の左右どちらの端にも寄せられるようにする。
pub fn window_size_for(scale: f64, aspect: f64, frame: Frame) -> (f64, f64) {
    let panel_w = PANEL_W * frame.panel_scale;
    let column_h = PANEL_COLUMN_H * frame.panel_scale;
    if !frame.show_character {
        return ((MARGIN + panel_w + MARGIN).round(), column_h.round());
    }
    let width = MARGIN + panel_w + MARGIN + STAGE_W * scale + MARGIN;
    let height = column_h.max(MARGIN + STAGE_W * aspect * scale + BUBBLE_ROOM);
    (width.round(), height.round())
}

/// style.css の #stage と同じ置き方で、ウィンドウの中の立ち絵の領域を返す。
/// 立ち絵を隠しているときはパネルの領域を返し、ドラッグ後に画面のどの半分へ置いたかで上下左右の隅を決める。
pub fn stage_rect_for(scale: f64, aspect: f64, corner: Corner, frame: Frame) -> StageRect {
    let (width, height) = window_size_for(scale, aspect, frame);
    if !frame.show_character {
        return StageRect {
            x: MARGIN,
            y: MARGIN,
            w: PANEL_W * frame.panel_scale,
            h: (height - MARGIN * 2.0).max(1.0),
        };
    }
    let (w, h) = (STAGE_W * scale, STAGE_W * aspect * scale);
    StageRect {
        x: if corner.left {
            MARGIN
        } else {
            width - MARGIN - w
        },
        y: if corner.top {
            BUBBLE_ROOM
        } else {
            height - MARGIN - h
        },
        w,
        h,
    }
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
            r#"{"stage_scale": 0.1}"#,
            r#"{"stage_scale": 5.0}"#,
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
            ..PanelDisplay::default()
        }
    }

    fn frame(show_character: bool) -> Frame {
        Frame {
            show_character,
            panel_scale: DEFAULT_PANEL_SCALE,
        }
    }

    #[test]
    fn display_settings_share_one_file() {
        let (_d, home) = home();
        assert_eq!(load(&home), DEFAULT);
        assert_eq!(load_panel_display(&home), None);
        let collapsed = display(true, PanelStyle::Collapsed);
        save(&home, 1.5).unwrap();
        save_panel_display(&home, collapsed).unwrap();
        assert_eq!(
            (load(&home), load_panel_display(&home)),
            (1.5, Some(collapsed))
        );
        save(&home, 2.0).unwrap();
        save_character(&home, "clawd").unwrap();
        assert_eq!(
            (load_panel_display(&home), load_character(&home).as_str()),
            (Some(collapsed), "clawd")
        );

        save_aspect(&home, 1.0).unwrap();
        assert_eq!(
            (
                load(&home),
                load_panel_display(&home),
                load_aspect(&home),
                load_character(&home).as_str()
            ),
            (2.0, Some(collapsed), 1.0, "clawd")
        );
        save(&home, 4.0).unwrap();
        save_panel_display(&home, display(false, PanelStyle::Detail)).unwrap();
        assert_eq!(load(&home), MAX);
        assert_eq!(
            load_panel_display(&home),
            Some(display(false, PanelStyle::Detail))
        );
    }

    #[test]
    fn panel_display_reads_legacy_panel_mode_until_saved() {
        use PanelStyle::{Collapsed, Detail};
        use RowOrder::{Started, Status, Updated};
        let (_d, home) = home();
        let cases = [
            (r#"{"panel_mode": "detail"}"#, Some((true, Detail, Started))),
            (
                r#"{"panel_mode": "counts"}"#,
                Some((true, Collapsed, Started)),
            ),
            (r#"{"panel_mode": "list"}"#, Some((false, Detail, Started))),
            (
                r#"{"panel_mode": "picture"}"#,
                Some((true, Collapsed, Started)),
            ),
            (r#"{"scale": 1.2, "panel_mode": "tiny"}"#, None),
            (r#"{"scale": 1.7}"#, None),
            (r#"{"panel_style": "bogus"}"#, None),
            (r#"{"row_order": "bogus"}"#, None),
            // 新しいキーは panel_mode より優先し、無い方だけを panel_mode の読み替えで補う。
            (
                r#"{"panel_mode": "list", "show_character": true, "panel_style": "counts"}"#,
                Some((true, Collapsed, Started)),
            ),
            (
                r#"{"panel_mode": "list", "panel_style": "collapsed"}"#,
                Some((false, Collapsed, Started)),
            ),
            (
                r#"{"show_character": false}"#,
                Some((false, Detail, Started)),
            ),
            (
                r#"{"panel_mode": "counts", "panel_style": "bogus"}"#,
                Some((true, Collapsed, Started)),
            ),
            (r#"{"row_order": "status"}"#, Some((true, Detail, Status))),
            (
                r#"{"panel_mode": "list", "row_order": "updated"}"#,
                Some((false, Detail, Updated)),
            ),
            (
                r#"{"show_character": true, "panel_style": "counts", "row_order": "bogus"}"#,
                Some((true, Collapsed, Started)),
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
                    ..PanelDisplay::default()
                }),
                "content {content:?}"
            );
        }

        fs::write(
            home.display_file(),
            r#"{"scale": 1.3, "panel_mode": "list", "future_key": [1]}"#,
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
            serde_json::json!({"scale": 1.3, "show_character": true, "panel_style": "detail", "row_order": "status", "panel_opacity": 0.8, "panel_scale": 1.0, "tint_strength": 1.0, "show_numbers": false, "hide_keeps_limits": false, "future_key": [1]})
        );
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
            r#"{"stage_scale": 1.5, "character": "clawd", "panel_opacity": "80%"}"#,
        )
        .unwrap();
        assert_eq!(
            (load(&home), load_character(&home).as_str()),
            (1.5, "clawd")
        );
    }

    #[test]
    fn usage_refresh_minutes_default_to_off() {
        let (_d, home) = home();
        let cases = [
            (r#"{}"#, 0),
            (r#"{"usage_refresh_minutes": 10}"#, 10),
            (
                r#"{"usage_refresh_minutes": 99999}"#,
                MAX_USAGE_REFRESH_MINUTES,
            ),
            (r#"{"usage_refresh_minutes": -5}"#, 0),
            (r#"{"usage_refresh_minutes": "10"}"#, 0),
        ];
        for (content, minutes) in cases {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(
                load_usage_refresh_minutes(&home),
                minutes,
                "content {content:?}"
            );
        }
        fs::write(home.display_file(), r#"{"stage_scale": 1.5}"#).unwrap();
        save_usage_refresh_minutes(&home, 15).unwrap();
        assert_eq!((load(&home), load_usage_refresh_minutes(&home)), (1.5, 15));
    }

    #[test]
    fn usage_tools_default_to_shown() {
        let (_d, home) = home();
        let tools = |claude, codex, antigravity| UsageTools {
            claude,
            codex,
            antigravity,
        };
        let cases = [
            (r#"{}"#, tools(true, true, true)),
            (
                r#"{"usage_tools": {"codex": false, "antigravity": "no"}}"#,
                tools(true, false, true),
            ),
            (r#"{"usage_tools": false}"#, tools(true, true, true)),
        ];
        for (content, expected) in cases {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(load_usage_tools(&home), expected, "content {content:?}");
        }
        fs::write(home.display_file(), r#"{"usage_refresh_minutes": 15}"#).unwrap();
        save_usage_tools(&home, tools(false, true, false)).unwrap();
        assert_eq!(
            (load_usage_tools(&home), load_usage_refresh_minutes(&home)),
            (tools(false, true, false), 15)
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
            r#"{"stage_scale": 1.5, "panel_style": "counts"}"#,
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
            // Display が知らないキーを含むファイルでも、ほかのキーは読める。
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
        // 倍率 1.0 の既定のキャラクターの大きさは tauri.conf.json のウィンドウの大きさと同じになる。
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let declared = &conf["app"]["windows"][0];
        assert_eq!(
            window_size_for(DEFAULT, DEFAULT_ASPECT, frame(true)),
            (
                declared["width"].as_f64().unwrap(),
                declared["height"].as_f64().unwrap()
            )
        );
        // ウィンドウの中の配置は style.css が決めるので、同じ寸法を使っていることも確認する。
        let css = include_str!("../../src/style.css");
        for (var, value) in [
            ("--stage-w", STAGE_W),
            ("--panel-w", PANEL_W),
            ("--margin", MARGIN),
            ("--bubble-room", BUBBLE_ROOM),
        ] {
            assert!(css.contains(&format!("{var}: {value}px;")), "{var}");
        }
        for (scale, aspect, size) in [
            (1.0, DEFAULT_ASPECT, (444.0, 380.0)),
            (4.0, DEFAULT_ASPECT, (768.0, 776.0)),
            (1.0, 0.5, (444.0, 380.0)),
            (4.0, 0.5, (768.0, 380.0)),
            (3.0, f64::NAN, (660.0, 614.0)),
            (3.0, 0.0, (660.0, 614.0)),
            (1.0, 100.0, (444.0, 560.0)),
        ] {
            assert_eq!(
                window_size_for(scale, clamp_aspect(aspect), frame(true)),
                size,
                "scale {scale} aspect {aspect}"
            );
        }
        // 立ち絵を隠しているときは倍率や縦横比によらずパネル幅だけに詰める。
        assert_eq!(
            window_size_for(4.0, DEFAULT_ASPECT, frame(false)),
            (328.0, 380.0)
        );
    }

    #[test]
    fn shortcuts_round_trip_and_fall_back_to_empty() {
        let (_d, home) = home();
        let cases = [
            (r#"{}"#, (None, None)),
            (
                r#"{"shortcuts": {"focus_urgent": "Shift+Command+M", "toggle_visible": "  "}}"#,
                (Some("Shift+Command+M"), None),
            ),
            (
                r#"{"shortcuts": {"focus_urgent": 123, "toggle_visible": "Alt+Space"}}"#,
                (None, Some("Alt+Space")),
            ),
            (r#"{"shortcuts": false}"#, (None, None)),
        ];
        for (content, (focus, toggle)) in cases {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(
                load_shortcuts(&home),
                Shortcuts {
                    focus_urgent: focus.map(str::to_owned),
                    toggle_visible: toggle.map(str::to_owned),
                },
                "content {content:?}"
            );
        }
        let saved = Shortcuts {
            focus_urgent: Some("Control+Alt+M".into()),
            toggle_visible: None,
        };
        save_shortcuts(&home, &saved).unwrap();
        assert_eq!(load_shortcuts(&home), saved);
    }

    #[test]
    fn legacy_scale_keeps_the_same_size() {
        let (_d, home) = home();
        for (content, expected) in [
            (r#"{"scale": 0.6}"#, 1.0),
            (r#"{"scale": 1.0}"#, 1.67),
            (r#"{"scale": 1}"#, 1.67),
            (r#"{"scale": 2.5}"#, MAX),
            (r#"{"scale": 1.0, "stage_scale": 0.8}"#, 0.8),
        ] {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(load(&home), expected, "content {content:?}");
        }
        save(&home, 0.7).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(home.display_file()).unwrap()).unwrap();
        assert_eq!(saved, serde_json::json!({"stage_scale": 0.7}));
    }
}
