import type { Point } from "./expander";

const HOVER = "hover";
const SELECTORS = [".row", ".row-read", ".today-line", ".counts-group.clickable", "#row-order button", "#panel-drag"];

/**
 * ウィンドウは focus: false で開き、前面に無いウィンドウの WebView にはマウスの移動が届かないので、CSS の :hover は
 * 一度ウィンドウを押すまで付かない。Rust から届くカーソル位置で、その下の操作要素に hover のクラスを付ける。
 * パネルは描き直すたびに行を作り直すので、描き直した後にも同じ位置で付け直す。
 */
export class RowHover {
  private cursor: Point | null = null;

  constructor(private readonly root: HTMLElement) {}

  setCursor(cursor: Point | null): void {
    this.cursor = cursor;
    this.apply();
  }

  apply(): void {
    const hit = this.cursor ? document.elementFromPoint(this.cursor.x, this.cursor.y) : null;
    const inside = hit && this.root.contains(hit) ? hit : null;
    const marked = inside
      ? SELECTORS.map((sel) => inside.closest(sel)).filter((el): el is Element => !!el)
      : [];
    for (const el of this.root.querySelectorAll(`.${HOVER}`)) {
      if (!marked.includes(el)) el.classList.remove(HOVER);
    }
    for (const el of marked) el.classList.add(HOVER);
  }
}
