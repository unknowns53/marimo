import { afterEach, describe, expect, it } from "vitest";

import { LANG_SETTINGS, langSettingLabel, plural, setLang, translateDocument } from "./i18n";

afterEach(() => setLang("ja"));

// vitest は DOM を持たないので、translateDocument が触る分だけを持つ要素で代える。
class FakeElement {
  constructor(
    public textContent: string,
    public dataset: Record<string, string | undefined> = {},
    private readonly attrs = new Map<string, string>(),
  ) {}

  hasAttribute(name: string): boolean {
    return this.attrs.has(name);
  }

  getAttribute(name: string): string | null {
    return this.attrs.get(name) ?? null;
  }

  setAttribute(name: string, value: string): void {
    this.attrs.set(name, value);
  }
}

function fakeRoot(elements: FakeElement[]): ParentNode {
  const matches = (el: FakeElement, selector: string) =>
    selector === "[data-en]" ? el.dataset.en !== undefined : el.hasAttribute(selector.slice(1, -1));
  return { querySelectorAll: (selector: string) => elements.filter((el) => matches(el, selector)) } as unknown as ParentNode;
}

describe("translateDocument", () => {
  it("switches text and attributes to English and back to the Japanese written in the HTML", () => {
    const heading = new FakeElement("表示", { en: "Display" });
    const nav = new FakeElement(
      "",
      {},
      new Map([
        ["aria-label", "設定の項目"],
        ["data-en-aria-label", "Settings sections"],
        ["title", "閉じる"],
        ["data-en-title", "Close"],
      ]),
    );
    const plain = new FakeElement("Claude Code");
    const root = fakeRoot([heading, nav, plain]);
    const read = () => [heading.textContent, nav.getAttribute("aria-label"), nav.getAttribute("title"), plain.textContent];

    translateDocument(root);
    expect(read()).toEqual(["表示", "設定の項目", "閉じる", "Claude Code"]);
    setLang("en");
    translateDocument(root);
    expect(read()).toEqual(["Display", "Settings sections", "Close", "Claude Code"]);
    setLang("ja");
    translateDocument(root);
    expect(read()).toEqual(["表示", "設定の項目", "閉じる", "Claude Code"]);
  });
});

describe("langSettingLabel", () => {
  it("names each language in itself and the automatic choice in the current language", () => {
    expect(LANG_SETTINGS.map(langSettingLabel)).toEqual(["自動（システムの言語）", "日本語", "English"]);
    setLang("en");
    expect(LANG_SETTINGS.map(langSettingLabel)).toEqual(["Automatic (system language)", "日本語", "English"]);
  });
});

describe("plural", () => {
  it("uses the singular only for one", () => {
    expect([plural(1, "session", "sessions"), plural(2, "session", "sessions"), plural(0, "item", "items")]).toEqual([
      "1 session",
      "2 sessions",
      "0 items",
    ]);
  });
});
