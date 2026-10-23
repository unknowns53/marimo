import type { BubbleSettings } from "./bubbleSettings";
import type { PanelDisplay } from "./panelModel";

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
  | { kind: "autostart"; enabled: boolean };

/** アプリの更新のコマンド check_update が返す形。 */
export interface UpdateInfo {
  current: string;
  latest: string;
  newer: boolean;
  url: string;
}

export const UPDATE_PROGRESS_EVENT = "update-progress";

export interface UpdateProgress {
  stage: "download" | "verify" | "extract" | "restart";
  message: string;
}
