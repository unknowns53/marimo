import { ExpressionDirector } from "./expression";
import { maskFromImage, type HitMask } from "./hitArea";
import { normalize, type Character, type ExpressionAssets, type StandingManifest } from "./manifest";
import type { CharacterRenderer, RendererInput } from "./renderer";
import type { Status } from "./types";

const BREATHING: ReadonlySet<Status> = new Set(["idle", "working"]);
const BLINK_MIN_MS = 3000;
const BLINK_MAX_MS = 6000;
const BLINK_HOLD_MS = 120;
const CROSSFADE_MS = 150;
// 仕草の開始と終了、反応やマウスを載せたときの表情の戻りは時刻で決まるので、この周期で確かめる。
const TICK_MS = 250;

export class StandingRenderer implements CharacterRenderer {
  onShapeChange: (() => void) | null = null;
  private readonly root = document.createElement("div");
  private readonly base = document.createElement("img");
  private readonly blink = document.createElement("img");
  private readonly fading = document.createElement("img");
  private readonly character: Character;
  private readonly canvas: { width: number; height: number };
  private readonly loaded = new Set<string>();
  private readonly masks = new Map<string, HitMask | null>();
  private readonly director: ExpressionDirector;
  private status: Status = "idle";
  private expression: string | null = null;
  private blinkTimer: number | undefined;
  private tickTimer: number | undefined;

  constructor(
    manifest: StandingManifest,
    private readonly baseUrl: string,
  ) {
    this.character = normalize(manifest);
    this.canvas = manifest.canvas;
    this.director = new ExpressionDirector(this.character.rules, (name) => this.loaded.has(name));
  }

  async mount(container: HTMLElement): Promise<void> {
    const { width, height } = this.canvas;
    this.root.className = "character";
    this.root.style.aspectRatio = `${width} / ${height}`;
    for (const img of [this.base, this.blink, this.fading]) {
      img.alt = "";
      img.draggable = false;
      this.root.append(img);
    }
    this.blink.hidden = true;
    this.fading.hidden = true;
    this.fading.className = "fading";
    // 切り替えの瞬間に読み込みで一瞬消えないよう、全差分を先にデコードしておく。読めなかった表情は
    // 使える表情に数えず、状態の基本の表情で代用させる。クリックを通す領域の格子も画像ごとに作る。
    await Promise.all(
      Object.entries(this.character.expressions).map(async ([name, assets]) => {
        const img = await preload(this.url(assets.image));
        if (!img) return;
        this.masks.set(assets.image, maskFromImage(img));
        if (assets.blink && !(await preload(this.url(assets.blink)))) delete assets.blink;
        this.loaded.add(name);
      }),
    );
    container.prepend(this.root);
    this.refresh(true);
    this.tickTimer = window.setInterval(() => this.refresh(false), TICK_MS);
  }

  update(input: RendererInput): void {
    this.status = input.status;
    this.director.setInput(input);
    this.refresh(false);
  }

  setHover(on: boolean): void {
    this.director.setHover(on, performance.now());
    this.refresh(false);
  }

  react(): boolean {
    const ok = this.director.react(performance.now());
    this.refresh(false);
    return ok;
  }

  // 瞬きの差分は顔だけが違い、輪郭は基本の画像と同じなので、基本の画像の形で判定する。
  hitArea(): { element: HTMLElement; mask: HitMask | null } {
    const assets = this.assets();
    return { element: this.root, mask: assets ? (this.masks.get(assets.image) ?? null) : null };
  }

  destroy(): void {
    window.clearTimeout(this.blinkTimer);
    window.clearInterval(this.tickTimer);
    this.root.remove();
  }

  private assets(): ExpressionAssets | null {
    return this.expression ? (this.character.expressions[this.expression] ?? null) : null;
  }

  private refresh(first: boolean): void {
    this.root.classList.toggle("breathing", BREATHING.has(this.status));
    const next = this.director.current(performance.now());
    if (next === this.expression && !first) return;
    if (!first) this.crossfadeFrom(this.blink.hidden ? this.base.src : this.blink.src);
    this.expression = next;
    const assets = this.assets();
    if (assets) {
      this.base.src = this.url(assets.image);
      if (assets.blink) this.blink.src = this.url(assets.blink);
    }
    this.base.hidden = false;
    this.blink.hidden = true;
    this.scheduleBlink();
    this.onShapeChange?.();
  }

  // 承認待ちや仕草の差分は腕や髪まで描き直しており、線もわずかに変わる。前の画像を上に重ねて
  // 薄くしていき、切り替えの瞬間に髪が跳ねて見えないようにする。瞬きだけはこれを通さない。
  private crossfadeFrom(src: string): void {
    const img = this.fading;
    img.getAnimations().forEach((a) => a.cancel());
    img.src = src;
    img.hidden = false;
    img
      .animate([{ opacity: 1 }, { opacity: 0 }], { duration: CROSSFADE_MS, easing: "ease-out" })
      .finished.then(
        () => (img.hidden = true),
        () => {},
      );
  }

  // 瞬きの差分を持つ表情だけ瞬きさせる。仕草の差分は瞬きの画像を持たないので、その間は瞬きしない。
  private scheduleBlink(): void {
    window.clearTimeout(this.blinkTimer);
    if (!this.assets()?.blink) return;
    const wait = BLINK_MIN_MS + Math.random() * (BLINK_MAX_MS - BLINK_MIN_MS);
    this.blinkTimer = window.setTimeout(() => {
      this.base.hidden = true;
      this.blink.hidden = false;
      this.blinkTimer = window.setTimeout(() => {
        this.blink.hidden = true;
        this.base.hidden = false;
        this.scheduleBlink();
      }, BLINK_HOLD_MS);
    }, wait);
  }

  private url(file: string): string {
    return new URL(file, this.baseUrl).href;
  }
}

async function preload(src: string): Promise<HTMLImageElement | null> {
  const img = new Image();
  img.src = src;
  try {
    await img.decode();
    return img;
  } catch {
    // 読めない差分があっても、他の表情の表示は続ける。
    return null;
  }
}
