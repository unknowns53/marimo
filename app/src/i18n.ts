// 言語は Rust が display.json の language と OS の言語から決め、get_language と LANGUAGE_EVENT で届く。
// フックと CLI も同じ規則で決めるので、パネルの要約と画面の文言が食い違わない。

export type Lang = "ja" | "en";
export type LangSetting = "auto" | "ja" | "en";

/** main.rs の LanguageState と同じ形。 */
export interface LanguageState {
  setting: LangSetting;
  lang: Lang;
}

/** 言語を変えたときに、Rust から全部のウィンドウへ届く。 */
export const LANGUAGE_EVENT = "language";

// 設定で選べるのは二つの言語だけにする。display.json に language が無いあいだは OS の言語に従う。
export const LANGUAGES: readonly Lang[] = ["ja", "en"];

// 英語版より前のテストは日本語の文言を前提にしているので、届くまでは日本語にしておく。
let current: Lang = "ja";

export function lang(): Lang {
  return current;
}

export function setLang(next: Lang): void {
  current = next;
  if (typeof document !== "undefined") document.documentElement.lang = next;
}

export function t(ja: string, en: string): string {
  return current === "ja" ? ja : en;
}

/** 英語の数え方。1 のときだけ単数形にする。 */
export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/**
 * HTML の地の文は日本語で書き、英語を data-en に持たせる。最初に切り替えるときに日本語を data-ja へ残し、
 * 日本語へ戻すときに使う。data-en-title、data-en-aria-label は同じ名前の属性を切り替える。
 */
export function translateDocument(root: ParentNode = document): void {
  for (const el of root.querySelectorAll<HTMLElement>("[data-en]")) {
    if (el.dataset.ja === undefined) el.dataset.ja = el.textContent ?? "";
    el.textContent = t(el.dataset.ja, el.dataset.en ?? "");
  }
  for (const attr of ["title", "aria-label"] as const) {
    const key = `en-${attr}`;
    for (const el of root.querySelectorAll<HTMLElement>(`[data-${key}]`)) {
      const jaKey = `data-ja-${attr}`;
      if (!el.hasAttribute(jaKey)) el.setAttribute(jaKey, el.getAttribute(attr) ?? "");
      el.setAttribute(attr, t(el.getAttribute(jaKey) ?? "", el.getAttribute(`data-${key}`) ?? ""));
    }
  }
}
