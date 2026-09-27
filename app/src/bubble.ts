export class Bubble {
  constructor(
    private readonly node: HTMLElement,
    onDismiss: () => void,
  ) {
    node.addEventListener("click", onDismiss);
  }

  show(text: string): void {
    renderPhrases(this.node, text);
    this.node.title = "押すと閉じます";
    fitToLines(this.node);
    this.node.classList.add("show");
  }

  hide(): void {
    this.node.classList.remove("show");
  }
}

const HIRAGANA = /[ぁ-ゟ]/;
const BREAK_AFTER = /[、。！？!?]/;

/**
 * 日本語は文字の間のどこでも改行できるので、そのままでは「中｜身」のように単語の途中で切れる。
 * 文節の切れ目に近い位置として、ひらがなの次にひらがな以外が来るところと、句読点の後ろだけで
 * 区切る。数字に接する空白は改行しない空白にして、「1 時間 10 分」を一続きのまま残す。
 */
export function phrases(text: string): string[] {
  const chars = Array.from(text.replace(/(?<=\d) | (?=\d)/g, " "));
  const out: string[] = [];
  let current = "";
  chars.forEach((ch, i) => {
    current += ch;
    const next = chars[i + 1];
    if (next === undefined) return;
    const afterKana = HIRAGANA.test(ch) && !HIRAGANA.test(next) && !BREAK_AFTER.test(next) && !/\s/.test(next);
    if (afterKana || (BREAK_AFTER.test(ch) && !BREAK_AFTER.test(next))) {
      out.push(current);
      current = "";
    }
  });
  if (current) out.push(current);
  return out;
}

// 区切りの間にだけ <wbr> を置き、CSS の word-break: keep-all でそれ以外の位置の改行を止める。
function renderPhrases(node: HTMLElement, text: string): void {
  const parts: Node[] = [];
  phrases(text).forEach((p, i) => {
    if (i > 0) parts.push(document.createElement("wbr"));
    parts.push(document.createTextNode(p));
  });
  node.replaceChildren(...parts);
}

/**
 * 折り返した文章の枠は CSS だけでは最大の幅のまま残り、行が短いと右に大きな余白ができる。
 * 描いた行の幅を測り、いちばん長い行に枠を合わせる。
 */
export function fitToLines(node: HTMLElement): void {
  node.style.width = "";
  const range = document.createRange();
  range.selectNodeContents(node);
  const rects = Array.from(range.getClientRects()).filter((r) => r.width > 0);
  const lines = new Set(rects.map((r) => Math.round(r.top)));
  if (lines.size < 2) return;
  const left = Math.min(...rects.map((r) => r.left));
  const right = Math.max(...rects.map((r) => r.right));
  node.style.width = `${Math.ceil(right - left)}px`;
}
