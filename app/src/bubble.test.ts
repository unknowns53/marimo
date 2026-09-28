import { describe, expect, it } from "vitest";
import { phrases } from "./bubble";

describe("phrases", () => {
  it("breaks only at phrase boundaries", () => {
    const cases: [string, string, string[]][] = [
      [
        "after hiragana before other scripts and after punctuation",
        "gaussian-scan が許可を待ってるよ。中身を見てあげて。",
        ["gaussian-scan が", "許可を", "待ってるよ。", "中身を", "見てあげて。"],
      ],
      ["not inside a run of punctuation", "ん？ どうしたの？", ["ん？", " どうしたの？"]],
    ];
    for (const [label, text, expected] of cases) {
      expect(phrases(text), label).toEqual(expected);
    }
    // 数字に接する空白は改行しない空白にして、所要時間を一続きのまま残す。
    const p = phrases("md、1 時間 10 分かかったけど終わったよ。");
    expect(p.some((s) => s.includes("1\u00a0時間\u00a010\u00a0分"))).toBe(true);
  });
});
