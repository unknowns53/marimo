import type { Point } from "./rowHover";

const DELAY_MS = 500;
/** カーソルの先端から下へずらす量。ポインタの絵に文字が隠れないようにする。 */
const OFFSET = 14;

/**
 * 前面に無いウィンドウの WebView には、title のツールチップが出ない。ウィンドウは focus: false で開くので、そのままでは
 * 一度ウィンドウを押すまでツールチップを読めない。そこで、ツールチップの文字は title ではなく data-tip に持たせ、
 * Rust から届くカーソル位置で、その下の要素の data-tip をこの要素に出す。title を残すと、前面にあるときに二重に出る。
 * ウィンドウの外へは描けないので、ウィンドウの内側に収まる位置へずらす。
 */
export class Tooltip {
  private cursor: Point | null = null;
  private text: string | null = null;
  private timer: number | undefined;

  constructor(private readonly node: HTMLElement) {}

  setCursor(cursor: Point | null): void {
    this.cursor = cursor;
    this.apply();
  }

  /** 描き直して要素が入れ替わっても、同じ文字なら出したままにする。 */
  apply(): void {
    const hit = this.cursor ? document.elementFromPoint(this.cursor.x, this.cursor.y) : null;
    const text = hit?.closest<HTMLElement>("[data-tip]")?.dataset.tip || null;
    if (text === this.text) return;
    this.text = text;
    this.hide();
    if (text) this.timer = window.setTimeout(() => this.show(), DELAY_MS);
  }

  /** 押したときは、ネイティブのツールチップと同じく消し、カーソルが別の要素へ移るまで出さない。 */
  dismiss(): void {
    this.hide();
  }

  private show(): void {
    if (!this.text || !this.cursor) return;
    this.node.textContent = this.text;
    this.node.hidden = false;
    this.place();
  }

  private hide(): void {
    window.clearTimeout(this.timer);
    this.timer = undefined;
    this.node.hidden = true;
  }

  private place(): void {
    if (!this.cursor) return;
    const { width, height } = this.node.getBoundingClientRect();
    const maxX = window.innerWidth - width;
    const below = this.cursor.y + OFFSET;
    const y = below + height <= window.innerHeight ? below : this.cursor.y - height - 4;
    this.node.style.left = `${Math.max(0, Math.min(this.cursor.x, maxX))}px`;
    this.node.style.top = `${Math.max(0, y)}px`;
  }
}
