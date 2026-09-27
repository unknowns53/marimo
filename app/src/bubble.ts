import type { Dialogue, Status } from "./types";

const SHOW_MS = 4500;

export class Bubble {
  private timer: number | undefined;
  private last: string | undefined;

  constructor(private readonly node: HTMLElement) {}

  say(status: Status, dialogue: Dialogue): void {
    const text = this.pick(dialogue[status] ?? []);
    if (!text) return;
    window.clearTimeout(this.timer);
    this.node.textContent = text;
    this.node.classList.add("show");
    this.timer = window.setTimeout(() => this.node.classList.remove("show"), SHOW_MS);
  }

  // 候補が複数あるときは、直前と同じセリフが続かないようにする。
  private pick(lines: string[]): string | undefined {
    const pool = lines.length > 1 ? lines.filter((l) => l !== this.last) : lines;
    const text = pool[Math.floor(Math.random() * pool.length)];
    this.last = text;
    return text;
  }
}
