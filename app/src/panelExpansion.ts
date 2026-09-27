import { Expander, union, type ExpandTarget, type Point } from "./expander";
import { rectOf, type Rect } from "./hitArea";

const EXPAND_ATTR = "data-expand-id";

/**
 * パネルの畳んだ行（件数の行、作業中の要約、各行）を、Rust から届くカーソル位置で開閉する。
 * 開く順序は、層を見えない状態で置いて大きさを測り、その領域をクリックを受け取る領域として
 * Rust へ登録し終えてから見せる。見せた瞬間に層の上のカーソルが下のウィンドウへ抜けないためである。
 * 親の層（件数の行や作業中の一覧）の中の行も、同じ仕組みで開ける。
 */
export class PanelExpansion {
  // 件数の行の中の作業中の一覧、その中の各行、と三段まで入れ子になる。
  private readonly levels = [new Expander(), new Expander(), new Expander()];
  private cursor: Point | null = null;
  private timer: number | undefined;
  private opening = new Set<HTMLElement>();
  // 開いて見せている行の印。描き直しで作り直された行は、登録を待たずにそのまま開いて見せる。
  private shownIds = new Set<string>();

  constructor(
    private readonly panel: HTMLElement,
    private readonly registerRegions: () => Promise<void>,
    // 層を閉じたときや、描き直しの後にそのまま開き直したときに、領域を送り直してもらう。
    private readonly onLayoutChange: () => void,
  ) {}

  setCursor(cursor: Point | null): void {
    this.cursor = cursor;
    this.evaluate();
  }

  // パネルを描き直すと DOM が作り直されるので、開いている行の印もここで付け直す。
  evaluate(): void {
    window.clearTimeout(this.timer);
    const now = performance.now();
    const all = Array.from(this.panel.querySelectorAll<HTMLElement>(`[${EXPAND_ATTR}]`));

    // 各段の候補は、一つ上の段で開いている行の層の中にあるもの。深い段の開いている範囲を先に
    // 求め、浅い段を開いたままにする判定へ渡す。
    const candidatesAt = (container: HTMLElement | null) =>
      all.filter((el) => nearestLayer(el) === container);
    const previouslyOpen: (HTMLElement | null)[] = [];
    let container: HTMLElement | null = null;
    for (const level of this.levels) {
      const els = candidatesAt(container);
      const open = els.find((el) => el.getAttribute(EXPAND_ATTR) === level.openId) ?? null;
      previouslyOpen.push(open);
      container = open ? layerOf(open) : null;
      if (!container) break;
    }
    const keeps = this.levels.map((level, i) => {
      const parent = i === 0 ? null : previouslyOpen[i - 1];
      if (i > 0 && !parent) return null;
      return level.keepRegion(candidatesAt(i === 0 ? null : layerOf(parent ?? null)).map(targetOf));
    });

    const openEls = new Set<HTMLElement>();
    container = null;
    for (let i = 0; i < this.levels.length; i++) {
      const els = candidatesAt(container);
      const deeperKeep = keeps
        .slice(i + 1)
        .reduce<Rect | null>((acc, k) => (k ? union(k, acc) : acc), null);
      const id = this.levels[i].update(now, this.cursor, els.map(targetOf), deeperKeep);
      const el = els.find((e) => e.getAttribute(EXPAND_ATTR) === id) ?? null;
      if (!el) {
        for (const deeper of this.levels.slice(i + 1)) deeper.update(now, null, []);
        break;
      }
      openEls.add(el);
      container = layerOf(el);
    }
    for (const el of all) this.show(el, openEls.has(el));
    this.shownIds = new Set(Array.from(openEls, (el) => el.getAttribute(EXPAND_ATTR) ?? ""));

    const deadlines = this.levels.map((l) => l.nextDeadline()).filter((d): d is number => d !== null);
    if (deadlines.length > 0) {
      this.timer = window.setTimeout(() => this.evaluate(), Math.max(0, Math.min(...deadlines) - now) + 5);
    }
  }

  private show(el: HTMLElement, open: boolean): void {
    if (!open) {
      if (el.classList.contains("open") || el.classList.contains("measuring")) this.onLayoutChange();
      el.classList.remove("open", "measuring");
      this.opening.delete(el);
      return;
    }
    if (el.classList.contains("open") || this.opening.has(el)) return;
    if (this.shownIds.has(el.getAttribute(EXPAND_ATTR) ?? "")) {
      el.classList.add("open");
      this.onLayoutChange();
      return;
    }
    this.opening.add(el);
    el.classList.add("measuring");
    void this.registerRegions().finally(() => {
      if (!this.opening.delete(el) || !el.isConnected) return;
      el.classList.remove("measuring");
      el.classList.add("open");
    });
  }
}

function nearestLayer(el: HTMLElement): HTMLElement | null {
  return el.parentElement?.closest<HTMLElement>(".hover-layer") ?? null;
}

function layerOf(el: HTMLElement | null): HTMLElement | null {
  return el ? (Array.from(el.children).find((c) => c.classList.contains("hover-layer")) as HTMLElement | undefined) ?? null : null;
}

function targetOf(el: HTMLElement): ExpandTarget {
  const anchor = rectOf(el) ?? { x: 0, y: 0, w: 0, h: 0 };
  const layer = layerOf(el);
  const visible = el.classList.contains("open") || el.classList.contains("measuring");
  return { id: el.getAttribute(EXPAND_ATTR) ?? "", anchor, layer: visible ? rectOf(layer) : null };
}
