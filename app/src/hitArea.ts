import { invoke } from "@tauri-apps/api/core";

// 立ち絵の不透明な部分を粗い格子で表したもの。bits は行優先の "0" と "1" の並び。
export interface HitMask {
  cols: number;
  rows: number;
  bits: string;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface HitRegions {
  rects: Rect[];
  mask: (Rect & HitMask) | null;
}

// 素材を横 40 マスに縮めると、1 マスは倍率 1.0 で 4.5 px 四方になる。指先ほどの精度があれば
// 足り、Rust 側へ送る量と判定の手間も小さく済む。縦のマス数は素材の縦横比から決め、マスを正方形に保つ。
export const MASK_COLS = 40;
// 縮小した画素のアルファがこれを超えるマスを不透明とみなす。髪の毛先のような薄い縁は拾わない。
const ALPHA_THRESHOLD = 24;

export function maskFromAlpha(alpha: ArrayLike<number>, cols: number, rows: number): HitMask {
  const solid: boolean[] = Array.from({ length: cols * rows }, (_, i) => alpha[i] > ALPHA_THRESHOLD);
  return { cols, rows, bits: dilate(solid, cols, rows).map((b) => (b ? "1" : "0")).join("") };
}

// 縁ぎりぎりを狙ったクリックが下のウィンドウへ抜けないよう、1 マスぶん広げる。
export function dilate(solid: boolean[], cols: number, rows: number): boolean[] {
  return solid.map((_, i) => {
    const cx = i % cols;
    const cy = Math.floor(i / cols);
    for (let dy = -1; dy <= 1; dy++) {
      for (let dx = -1; dx <= 1; dx++) {
        const x = cx + dx;
        const y = cy + dy;
        if (x >= 0 && y >= 0 && x < cols && y < rows && solid[y * cols + x]) return true;
      }
    }
    return false;
  });
}

export function maskFromImage(img: HTMLImageElement): HitMask | null {
  if (!img.naturalWidth) return null;
  const rows = Math.max(1, Math.round((MASK_COLS * img.naturalHeight) / img.naturalWidth));
  const canvas = document.createElement("canvas");
  canvas.width = MASK_COLS;
  canvas.height = rows;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  ctx.drawImage(img, 0, 0, MASK_COLS, rows);
  const data = ctx.getImageData(0, 0, MASK_COLS, rows).data;
  const alpha = new Uint8Array(MASK_COLS * rows);
  for (let i = 0; i < alpha.length; i++) alpha[i] = data[i * 4 + 3];
  return maskFromAlpha(alpha, MASK_COLS, rows);
}

export function rectOf(el: Element | null): Rect | null {
  if (!el || (el instanceof HTMLElement && el.hidden)) return null;
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return null;
  return { x: r.x, y: r.y, w: r.width, h: r.height };
}

export interface Portrait {
  box: Rect | null;
  mask: HitMask | null;
}

/**
 * パネルなどの矩形と立ち絵の形を、Rust へ送る領域にまとめる。立ち絵を隠しているときは portrait に
 * null を渡す。格子も矩形も送らなければ、その場所のクリックは下へ通り、立ち絵の上にいるという
 * 知らせも Rust から来なくなる。格子が無い立ち絵は矩形全体を不透明とみなす。
 */
export function hitRegions(rects: (Rect | null)[], portrait: Portrait | null): HitRegions {
  const all = [...rects];
  let mask: HitRegions["mask"] = null;
  if (portrait?.mask && portrait.box) mask = { ...portrait.box, ...portrait.mask };
  else if (portrait) all.push(portrait.box);
  return { rects: all.filter((r): r is Rect => r !== null), mask };
}

/**
 * クリックを受け取る領域を Rust へ知らせる。窓は透明な部分のクリックを下のウィンドウへ通すので、
 * 見た目が変わるたびに領域を送り直す。同じ内容なら送らない。
 */
export class HitReporter {
  private pending = false;
  private last = "";

  constructor(private readonly collect: () => HitRegions) {}

  /** 次の描画を待たずにすぐ送る。層を見せる前に、その領域を登録し終えておくために使う。 */
  async flush(): Promise<void> {
    const regions = this.collect();
    this.last = JSON.stringify(regions);
    await invoke("set_hit_regions", { regions }).catch((e) => console.error("hit regions", e));
  }

  schedule(): void {
    if (this.pending) return;
    this.pending = true;
    requestAnimationFrame(() => {
      this.pending = false;
      const regions = this.collect();
      const json = JSON.stringify(regions);
      if (json === this.last) return;
      this.last = json;
      void invoke("set_hit_regions", { regions }).catch((e) => console.error("hit regions", e));
    });
  }
}
