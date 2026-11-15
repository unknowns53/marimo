import type { BubbleSettings } from "./bubbleSettings";
import type { PanelDisplay } from "./panelModel";
import type { UsageTools } from "./types";

// 設定の持ち主はメインのウィンドウで、設定のウィンドウは変更を頼むだけにする。表示の状態と描画をメインのウィンドウが持つので、
// 設定のウィンドウが自分で保存すると、反映の処理を二か所に書くことになるからである。
export const MAIN_WINDOW = "main";
export const SETTINGS_WINDOW = "settings";

/** 設定のウィンドウからメインのウィンドウへ送る。 */
export const SETTINGS_REQUEST_EVENT = "settings-request";
/** メインのウィンドウから設定のウィンドウへ、反映したあとの状態の全体を送る。 */
export const SETTINGS_STATE_EVENT = "settings-state";

export interface CharacterChoice {
  id: string;
  name: string;
}

export interface SettingsState {
  display: PanelDisplay;
  bubble: BubbleSettings;
  /** manifest を読めたキャラクターだけを並べる。 */
  characters: CharacterChoice[];
  character: string | null;
  /** 倍率を読めなかったときは null で、大きさを変えられない。 */
  scale: number | null;
  /** ログイン項目の状態を読めなかったときは null で、切り替えられない。 */
  autostart: boolean | null;
  /** Claude Code、Codex、Antigravity に利用制限を再取得させる間隔（分）。0 は再取得しない。 */
  usage_refresh_minutes: number;
  usage_tools: UsageTools;
}

export type SettingsRequest =
  /** ウィンドウを開いたときに今の状態を求める。 */
  | { kind: "sync" }
  | { kind: "display"; patch: Partial<PanelDisplay> }
  /** スライダーを動かしている間の不透明度。見た目だけに反映し、保存は離したときの display で行う。 */
  | { kind: "opacity-preview"; opacity: number }
  | { kind: "bubble"; patch: Partial<BubbleSettings> }
  | { kind: "character"; id: string }
  | { kind: "scale"; scale: number }
  | { kind: "autostart"; enabled: boolean }
  | { kind: "usage-refresh"; minutes: number }
  | { kind: "usage-tools"; patch: Partial<UsageTools> };

/** 開いているウィンドウに、左の一覧のどの項目を出すかを送る。settings.rs の SECTION_EVENT と揃える。 */
export const SECTION_EVENT = "settings-section";

/** update.rs の UpdateStatus と同じ形。update_status と check_update が返し、確認のたびに UPDATE_STATUS_EVENT でも届く。 */
export interface UpdateStatus {
  current: string;
  /** まだ確認していないときと、確認に失敗したときは null。 */
  latest: string | null;
  newer: boolean;
  /** 確認した時刻（UNIX 時刻のミリ秒）。 */
  checked_at: number | null;
  error: string | null;
}

export const UPDATE_STATUS_EVENT = "update-status";
export const UPDATE_PROGRESS_EVENT = "update-progress";

export interface UpdateProgress {
  stage: "download" | "verify" | "extract" | "restart";
  message: string;
}
