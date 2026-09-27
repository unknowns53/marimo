import { invoke } from "@tauri-apps/api/core";

// 範囲と丸めは Rust 側の scale.rs と揃える。最終的な値は set_scale の戻り値を正とする。
export const SCALE_MIN = 0.6;
export const SCALE_MAX = 2.5;
const WHEEL_STEP = 0.1;
// トラックパッドは一度のスワイプで wheel を数十回送るので、刻みの間隔に下限を置く。
const WHEEL_INTERVAL_MS = 60;

export const SCALE_PRESETS: ReadonlyArray<{ label: string; scale: number }> = [
  { label: "小（1.0）", scale: 1.0 },
  { label: "中（1.5）", scale: 1.5 },
  { label: "大（2.0）", scale: 2.0 },
  { label: "特大（2.5）", scale: 2.5 },
];

export function nearestPreset(scale: number): number {
  return SCALE_PRESETS.reduce((best, p) =>
    Math.abs(p.scale - scale) < Math.abs(best.scale - scale) ? p : best,
  ).scale;
}

export class ScaleControl {
  private requested: number;
  private applied: number;
  private lastWheel = 0;

  constructor(initial: number) {
    this.requested = initial;
    this.applied = initial;
    applyCss(initial);
  }

  get current(): number {
    return this.applied;
  }

  // 連続して呼ばれても、要求した値を積み上げてから Rust へ渡すので刻みを取りこぼさない。
  set(scale: number): void {
    this.requested = clamp(scale);
    void invoke<number>("set_scale", { scale: this.requested }).then(
      (applied) => {
        this.applied = applied;
        applyCss(applied);
      },
      (e) => console.error("set_scale", e),
    );
  }

  bindWheel(target: HTMLElement): void {
    const isMac = navigator.userAgent.includes("Mac");
    target.addEventListener(
      "wheel",
      (e) => {
        if (!(isMac ? e.metaKey : e.ctrlKey)) return;
        // WebView2 は Ctrl とホイールでページ全体を拡大するので、既定の動作を止める。
        e.preventDefault();
        const now = performance.now();
        if (e.deltaY === 0 || now - this.lastWheel < WHEEL_INTERVAL_MS) return;
        this.lastWheel = now;
        this.set(this.requested + (e.deltaY < 0 ? WHEEL_STEP : -WHEEL_STEP));
      },
      { passive: false },
    );
  }
}

function clamp(scale: number): number {
  if (!Number.isFinite(scale)) return 1;
  return Math.round(Math.min(SCALE_MAX, Math.max(SCALE_MIN, scale)) * 100) / 100;
}

function applyCss(scale: number): void {
  document.documentElement.style.setProperty("--scale", String(scale));
}
