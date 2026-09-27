import type { BubbleView } from "./bubbleModel";

/**
 * 吹き出しに今どのセリフを出すかを決める。承認待ちや完了の吹き出しは知らせるためのものなので、
 * 立ち絵を押したときのひとことより常に優先し、上書きしない。
 */
export class Speech {
  private reaction: { text: string; until: number } | null = null;

  /** ひとことを出せたら true を返す。知らせの吹き出しが出ている間は出さない。 */
  react(text: string | undefined, now: number, durationMs: number, status: BubbleView | null): boolean {
    if (status || !text) return false;
    this.reaction = { text, until: now + durationMs };
    return true;
  }

  current(status: BubbleView | null, now: number): string | null {
    if (status) {
      this.reaction = null;
      return status.text;
    }
    if (this.reaction && now < this.reaction.until) return this.reaction.text;
    this.reaction = null;
    return null;
  }

  clearReaction(): void {
    this.reaction = null;
  }
}
