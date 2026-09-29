import type { Rect } from "./hitArea";
import { DEFAULT_PANEL_OPACITY, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN } from "./panelModel";

// スライダーは百分率の整数で持つ。小数の刻みにすると値の文字列に 0.35000000000000003 のような誤差が出る。
const STEP_PERCENT = 5;
const EDGE_PX = 8;
const GAP_PX = 4;

export interface Size {
  w: number;
  h: number;
}

/** 範囲外は近い端へ寄せ、数でなければ既定にした百分率。 */
export function opacityPercent(opacity: number): number {
  const value = Number.isFinite(opacity)
    ? Math.min(PANEL_OPACITY_MAX, Math.max(PANEL_OPACITY_MIN, opacity))
    : DEFAULT_PANEL_OPACITY;
  return Math.round(value * 100);
}

/**
 * パネルのタブの列の上に、左右の中央を揃えて置く。窓の上端までに収まらないとき（立ち絵を隠していて
 * パネルが窓の上端近くまで伸びたときなど）はタブの列の下へ置き、パネルの上の方の行に重ねる。
 * どちらの場合も窓の端から EDGE_PX の内側に収める。
 */
export function placePopover(anchor: Rect, size: Size, view: Size): { x: number; y: number } {
  const above = anchor.y - GAP_PX - size.h;
  const y = above >= EDGE_PX ? above : anchor.y + anchor.h + GAP_PX;
  return { x: within(anchor.x + (anchor.w - size.w) / 2, size.w, view.w), y: within(y, size.h, view.h) };
}

function within(pos: number, size: number, limit: number): number {
  return Math.max(EDGE_PX, Math.min(pos, limit - EDGE_PX - size));
}

export interface OpacityPopoverHandlers {
  /** 小窓を寄せるタブの列の矩形。 */
  anchor: () => Rect | null;
  /** スライダーを動かしている間に呼ぶ。保存はしない。 */
  preview: (opacity: number) => void;
  /** スライダーを離したときに呼ぶ。 */
  commit: (opacity: number) => void;
  /** 閉じたときに呼ぶ。離す前に閉じた分の見た目を保存済みの値へ戻すのに使う。 */
  closed: () => void;
  /** 開閉や移動でクリックを受け取る領域が変わったときに呼ぶ。 */
  layoutChange: () => void;
}

/**
 * パネルの地の不透明度を変える小窓。値は動かしている間は見た目だけに反映し、離したときに一度だけ
 * 保存させる。input のたびに保存すると、ドラッグ一回で display.json を何十回も書き直すからである。
 */
export class OpacityPopover {
  private readonly slider: HTMLInputElement;
  private readonly label: HTMLElement;

  constructor(
    private readonly root: HTMLElement,
    private readonly handlers: OpacityPopoverHandlers,
  ) {
    this.slider = root.querySelector("input") as HTMLInputElement;
    this.label = root.querySelector("label") as HTMLElement;
    this.slider.min = String(opacityPercent(PANEL_OPACITY_MIN));
    this.slider.max = String(opacityPercent(PANEL_OPACITY_MAX));
    this.slider.step = String(STEP_PERCENT);
    this.slider.addEventListener("input", () => {
      this.showLabel(Number(this.slider.value));
      handlers.preview(this.value());
    });
    this.slider.addEventListener("change", () => handlers.commit(this.value()));
    root.querySelector("button")?.addEventListener("click", () => this.close());
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") this.close();
    });
    document.addEventListener(
      "mousedown",
      (e) => {
        if (!(e.target instanceof Node && root.contains(e.target))) this.close();
      },
      true,
    );
    // 窓の透明な部分のクリックは下のウィンドウへ抜けて mousedown が届かないので、ほかのアプリへ
    // 移ったことを窓がフォーカスを失ったことで知る。
    window.addEventListener("blur", () => this.close());
  }

  get isOpen(): boolean {
    return !this.root.hidden;
  }

  open(opacity: number): void {
    this.sync(opacity);
    this.root.hidden = false;
    this.place();
    this.slider.focus();
    this.handlers.layoutChange();
  }

  close(): void {
    if (!this.isOpen) return;
    this.root.hidden = true;
    this.handlers.closed();
    this.handlers.layoutChange();
  }

  /** 保存済みの値が別の経路で変わったときに、スライダーと百分率の表示を揃える。 */
  sync(opacity: number): void {
    const percent = opacityPercent(opacity);
    this.slider.value = String(percent);
    this.showLabel(percent);
  }

  /** パネルの高さや窓の大きさが変わったら、開いている間は置き直す。 */
  place(): void {
    const anchor = this.isOpen ? this.handlers.anchor() : null;
    if (!anchor) return;
    const size = { w: this.root.offsetWidth, h: this.root.offsetHeight };
    const { x, y } = placePopover(anchor, size, { w: window.innerWidth, h: window.innerHeight });
    this.root.style.left = `${x}px`;
    this.root.style.top = `${y}px`;
  }

  private value(): number {
    return Number(this.slider.value) / 100;
  }

  private showLabel(percent: number): void {
    this.label.textContent = `背景 ${percent}%`;
    const min = Number(this.slider.min);
    this.slider.style.setProperty("--fill", `${((percent - min) / (Number(this.slider.max) - min)) * 100}%`);
  }
}
