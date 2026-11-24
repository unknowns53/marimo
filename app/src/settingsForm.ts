import { BUBBLE_SECONDS_CHOICES } from "./bubbleSettings";
import { DEFAULT_PANEL_OPACITY, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN, PANEL_STYLES, ROW_ORDERS } from "./panelModel";
import { LANGUAGES, t } from "./i18n";
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
  if (seconds === 0) return t("閉じるまで", "Until dismissed");
  if (seconds % 60 === 0) return t(`${seconds / 60} 分`, `${seconds / 60} min`);
  return t(`${seconds} 秒`, `${seconds} sec`);
}

/** display.json を手で書き換えて選択肢に無い秒数が入っていても、今の値として選べるよう加える。 */
export function bubbleSecondsOptions(current: number): number[] {
  if (BUBBLE_SECONDS_CHOICES.includes(current)) return [...BUBBLE_SECONDS_CHOICES];
  return [...BUBBLE_SECONDS_CHOICES, current].sort((a, b) => a - b);
}

// 範囲は Rust 側の scale.rs と揃える（sharedConstants.test.ts が照合する）。
export const MAX_USAGE_REFRESH_MINUTES = 1440;

/** 一覧に並べる間隔（分）。0 は再取得しない。 */
export const USAGE_REFRESH_CHOICES: readonly number[] = [0, 5, 10, 15, 30, 60];

export function usageRefreshLabel(minutes: number): string {
  return minutes === 0 ? t("しない", "Never") : t(`${minutes} 分ごと`, `Every ${minutes} min`);
}

/** display.json を手で書き換えて一覧に無い間隔が入っていても、今の値として選べるよう加える。 */
export function usageRefreshOptions(current: number): number[] {
  if (USAGE_REFRESH_CHOICES.includes(current)) return [...USAGE_REFRESH_CHOICES];
  return [...USAGE_REFRESH_CHOICES, current].sort((a, b) => a - b);
}

export function scaleLabel(scale: number): string {
  return t(`${scale.toFixed(1)} 倍`, `${scale.toFixed(1)}×`);
}

export interface ShortcutKeyInput {
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

const MODIFIER_CODES = new Set([
  "MetaLeft",
  "MetaRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
]);

const SPECIAL_KEY_CODES: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Minus: "Minus",
  Equal: "Equal",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
  Backquote: "Backquote",
};

function keyFromCode(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  return SPECIAL_KEY_CODES[code] ?? null;
}

/**
 * キー入力から Tauri の global-shortcut が解釈する文字列を組み立てる。
 * 通常のタイピングを奪わないよう、Shift 以外の修飾キー（Ctrl、Alt、Cmd、Windows キー）が押されているときだけ返す。
 * Shift だけの組み合わせは、OS 全体に登録するとどのアプリでもその文字を打てなくなる。
 */
export function shortcutFromKeyEvent(input: ShortcutKeyInput, isMac: boolean): string | null {
  if (MODIFIER_CODES.has(input.code)) return null;
  const key = keyFromCode(input.code);
  if (!key) return null;
  const mods: string[] = [];
  if (isMac) {
    if (input.ctrlKey) mods.push("Control");
    if (input.altKey) mods.push("Alt");
    if (input.shiftKey) mods.push("Shift");
    if (input.metaKey) mods.push("Command");
  } else {
    if (input.ctrlKey) mods.push("Control");
    if (input.altKey) mods.push("Alt");
    if (input.shiftKey) mods.push("Shift");
    if (input.metaKey) mods.push("Super");
  }
  if (!input.ctrlKey && !input.altKey && !input.metaKey) return null;
  return [...mods, key].join("+");
}

const MAC_SYMBOLS: Record<string, string> = {
  Command: "⌘",
  CommandOrControl: "⌘",
  Super: "⌘",
  Control: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Up: "↑",
  Down: "↓",
  Left: "←",
  Right: "→",
};

const WIN_LABELS: Record<string, string> = {
  Command: "Win",
  CommandOrControl: "Ctrl",
  Super: "Win",
  Control: "Ctrl",
  Alt: "Alt",
  Shift: "Shift",
};

/** 保存しているショートカット文字列を、OS に合った見やすい表記へ整える。 */
export function formatShortcut(value: string | null, isMac: boolean): string {
  if (!value) return t("未設定", "Not set");
  const parts = value.split("+").filter(Boolean);
  if (isMac) return parts.map((p) => MAC_SYMBOLS[p] ?? p).join(" ");
  return parts.map((p) => WIN_LABELS[p] ?? p).join("+");
}

/**
 * フォームの部品の name ごとの値。チェックボックスは真偽値、それ以外は value に入れる文字列で、
 * null はその設定を読めず操作できないことを表す。
 */
export type FormValues = Record<string, string | boolean | null>;

export function formValues(state: SettingsState): FormValues {
  const { display, bubble } = state;
  return {
    language: state.language,
    show_character: display.show_character,
    panel_style: display.panel_style,
    row_order: display.row_order,
    panel_opacity: String(opacityPercent(display.panel_opacity)),
    bubble_waiting: bubble.bubble_waiting,
    bubble_done: bubble.bubble_done,
    bubble_error: bubble.bubble_error,
    bubble_seconds: String(bubble.bubble_seconds),
    character: state.characters.length > 0 ? state.character ?? "" : null,
    scale: state.scale === null ? null : String(state.scale),
    autostart: state.autostart,
    usage_refresh_minutes: String(state.usage_refresh_minutes),
    usage_claude: state.usage_tools.claude,
    usage_codex: state.usage_tools.codex,
    usage_antigravity: state.usage_tools.antigravity,
  };
}

export interface FormControl {
  name: string;
  value: string;
  checked: boolean;
}

/**
 * フォームの部品の変化から、メインのウィンドウへ送る要求を作る。スライダーは動かしている間（input）から送り、
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
    case "language": {
      const setting = LANGUAGES.find((l) => l === value);
      return setting ? { kind: "language", setting } : null;
    }
    case "show_character":
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
    case "usage_refresh_minutes": {
      const minutes = Number(value);
      return Number.isInteger(minutes) && minutes >= 0 ? { kind: "usage-refresh", minutes } : null;
    }
    case "usage_claude":
      return { kind: "usage-tools", patch: { claude: checked } };
    case "usage_codex":
      return { kind: "usage-tools", patch: { codex: checked } };
    case "usage_antigravity":
      return { kind: "usage-tools", patch: { antigravity: checked } };
    default:
      return null;
  }
}
