import type { CharacterRenderer, StandingManifest, StandingStateAssets } from "./renderer";
import type { Status } from "./types";

const BREATHING: ReadonlySet<Status> = new Set(["idle", "working"]);
const BLINK_MIN_MS = 3000;
const BLINK_MAX_MS = 6000;
const BLINK_HOLD_MS = 120;

export class StandingRenderer implements CharacterRenderer {
  private readonly root = document.createElement("div");
  private readonly base = document.createElement("img");
  private readonly blink = document.createElement("img");
  private status: Status = "idle";
  private blinkTimer: number | undefined;

  constructor(
    private readonly manifest: StandingManifest,
    private readonly baseUrl: string,
  ) {}

  async mount(container: HTMLElement): Promise<void> {
    const { width, height } = this.manifest.canvas;
    this.root.className = "character";
    this.root.style.aspectRatio = `${width} / ${height}`;
    for (const img of [this.base, this.blink]) {
      img.alt = "";
      img.draggable = false;
      this.root.append(img);
    }
    this.blink.hidden = true;
    // 切り替えの瞬間に読み込みで一瞬消えないよう、全差分を先にデコードしておく。
    await Promise.all(this.allFiles().map((file) => preload(this.url(file))));
    container.prepend(this.root);
    this.apply();
  }

  setStatus(status: Status): void {
    if (status === this.status) return;
    this.status = status;
    this.apply();
  }

  destroy(): void {
    window.clearTimeout(this.blinkTimer);
    this.root.remove();
  }

  private assets(): StandingStateAssets {
    return this.manifest.states[this.status] ?? this.manifest.states.idle;
  }

  private apply(): void {
    const assets = this.assets();
    this.base.src = this.url(assets.image);
    this.base.hidden = false;
    this.blink.hidden = true;
    if (assets.blink) this.blink.src = this.url(assets.blink);
    this.root.classList.toggle("breathing", BREATHING.has(this.status));
    this.scheduleBlink();
  }

  // 瞬きの差分を持つ状態だけ瞬きさせる。どの状態に差分を付けるかは manifest に任せる。
  private scheduleBlink(): void {
    window.clearTimeout(this.blinkTimer);
    if (!this.assets().blink) return;
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

  private allFiles(): string[] {
    return Object.values(this.manifest.states).flatMap((s) =>
      s ? [s.image, ...(s.blink ? [s.blink] : [])] : [],
    );
  }

  private url(file: string): string {
    return new URL(file, this.baseUrl).href;
  }
}

async function preload(src: string): Promise<void> {
  const img = new Image();
  img.src = src;
  try {
    await img.decode();
  } catch {
    // 読めない差分があっても、他の状態の表示は続ける。
  }
}
