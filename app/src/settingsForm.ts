import { BUBBLE_SECONDS_CHOICES } from "./bubbleSettings";
import { DEFAULT_PANEL_OPACITY, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN, PANEL_STYLES, ROW_ORDERS } from "./panelModel";
import type { SettingsRequest, SettingsState } from "./settingsProtocol";

// スライダーは百分率の整数で持つ。小数の刻みにすると値の文字列に 0.35000000000000003 のような誤差が出る。
export const OPACITY_STEP_PERCENT = 5;

/** 範囲外は近い端へ寄せ、数でなければ既定にした百分率。 */
export function opacityPercent(opacity: number): number {
  const value = Number.isFinite(opacity)
    ? Math.min(PANEL_OPACITY_MAX, Math.max(PANEL_OPACITY_MIN, opacity))
    : DEFAULT_PANEL_OPACITY;
  return Math.round(value * 100);
}

export function bubbleSecondsLabel(seconds: number): string {
  if (seconds === 0) return "押すまで";
  return seconds % 60 === 0 ? `${seconds / 60} 分` : `${seconds} 秒`;
}

/** display.json を手で書き換えて選択肢に無い秒数が入っていても、今の値として選べるよう加える。 */
export function bubbleSecondsOptions(current: number): number[] {
  if (BUBBLE_SECONDS_CHOICES.includes(current)) return [...BUBBLE_SECONDS_CHOICES];
  return [...BUBBLE_SECONDS_CHOICES, current].sort((a, b) => a - b);
}

export function scaleLabel(scale: number): string {
  return `${scale.toFixed(1)} 倍`;
}

/**
 * フォームの部品の name ごとの値。チェックボックスは真偽値、それ以外は value に入れる文字列で、
 * null はその設定を読めず操作できないことを表す。
 */
export type FormValues = Record<string, string | boolean | null>;

export function formValues(state: SettingsState): FormValues {
  const { display, bubble } = state;
  return {
    show_character: display.show_character,
    panel_style: display.panel_style,
    row_order: display.row_order,
    show_cloud_sessions: display.show_cloud_sessions,
    panel_opacity: String(opacityPercent(display.panel_opacity)),
    bubble_waiting: bubble.bubble_waiting,
    bubble_done: bubble.bubble_done,
    bubble_error: bubble.bubble_error,
    bubble_seconds: String(bubble.bubble_seconds),
    character: state.characters.length > 0 ? state.character ?? "" : null,
    scale: state.scale === null ? null : String(state.scale),
    autostart: state.autostart,
  };
}

export interface FormControl {
  name: string;
  value: string;
  checked: boolean;
}

/**
 * フォームの部品の変化から、メインの窓へ送る要求を作る。スライダーは動かしている間（input）から送り、
 * それ以外は確定したとき（change）だけ送る。チェックボックスや選択でも input は届くので、二重に送らない。
 * 背景の不透明度は、動かしている間は見た目だけを変えさせ、離したときに一度だけ保存させる。input の
 * たびに保存すると、ドラッグ一回で display.json を何十回も書き直すからである。
 */
export function requestFor(control: FormControl, phase: "input" | "change"): SettingsRequest | null {
  const { name, value, checked } = control;
  if (name === "panel_opacity") {
    const opacity = Number(value) / 100;
    return phase === "input" ? { kind: "opacity-preview", opacity } : { kind: "display", patch: { panel_opacity: opacity } };
  }
  if (name === "scale") return phase === "input" ? { kind: "scale", scale: Number(value) } : null;
  if (phase !== "change") return null;
  switch (name) {
    case "show_character":
    case "show_cloud_sessions":
      return { kind: "display", patch: { [name]: checked } };
    case "panel_style": {
      const style = PANEL_STYLES.find((s) => s === value);
      return style ? { kind: "display", patch: { panel_style: style } } : null;
    }
    case "row_order": {
      const order = ROW_ORDERS.find((o) => o === value);
      return order ? { kind: "display", patch: { row_order: order } } : null;
    }
    case "bubble_waiting":
    case "bubble_done":
    case "bubble_error":
      return { kind: "bubble", patch: { [name]: checked } };
    case "bubble_seconds": {
      const seconds = Number(value);
      return Number.isInteger(seconds) && seconds >= 0 ? { kind: "bubble", patch: { bubble_seconds: seconds } } : null;
    }
    case "character":
      return value ? { kind: "character", id: value } : null;
    case "autostart":
      return { kind: "autostart", enabled: checked };
    default:
      return null;
  }
}
