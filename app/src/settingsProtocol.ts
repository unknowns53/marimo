import type { BubbleSettings } from "./bubbleSettings";
import type { PanelDisplay } from "./panelModel";

// 設定の持ち主はメインの窓で、設定の窓は変更を頼むだけにする。表示の状態と描画をメインの窓が持つので、
// 設定の窓が自分で保存すると、反映の処理を二か所に書くことになるからである。
export const MAIN_WINDOW = "main";
export const SETTINGS_WINDOW = "settings";

/** 設定の窓からメインの窓へ送る。 */
export const SETTINGS_REQUEST_EVENT = "settings-request";
/** メインの窓から設定の窓へ、反映したあとの状態の全体を送る。 */
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
  /** 窓を開いたときに今の状態を求める。 */
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
