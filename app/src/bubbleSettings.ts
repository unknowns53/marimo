import type { Status } from "./types";

// 範囲と既定は Rust 側の scale.rs と揃える。保存するときは Rust が範囲に収め直す。
export const MAX_BUBBLE_SECONDS = 3600;

/** Rust の scale::BubbleSettings と同じ形で、display.json に保存する。 */
export interface BubbleSettings {
  bubble_waiting: boolean;
  bubble_done: boolean;
  bubble_error: boolean;
  /** 出してから自動で消すまでの秒数。0 は押すまで出し続ける。 */
  bubble_seconds: number;
}

export const DEFAULT_BUBBLE_SETTINGS: BubbleSettings = {
  bubble_waiting: true,
  bubble_done: true,
  bubble_error: true,
  bubble_seconds: 0,
};

/** メニューに並べる秒数。0 は押すまで出し続ける。 */
export const BUBBLE_SECONDS_CHOICES: readonly number[] = [0, 10, 30, 60, 300];

export function bubbleEnabled(settings: BubbleSettings, status: Status): boolean {
  switch (status) {
    case "waiting":
      return settings.bubble_waiting;
    case "done":
      return settings.bubble_done;
    case "error":
      return settings.bubble_error;
    default:
      return false;
  }
}
