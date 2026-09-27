import { describe, expect, it } from "vitest";
import { phrases } from "./bubble";

describe("phrases", () => {
  it("breaks after hiragana before other scripts and after punctuation", () => {
    expect(phrases("gaussian-scan が許可を待ってるよ。中身を見てあげて。")).toEqual([
      "gaussian-scan が",
      "許可を",
      "待ってるよ。",
      "中身を",
      "見てあげて。",
    ]);
  });

  it("keeps a duration together by turning spaces next to digits into no-break spaces", () => {
    const p = phrases("md、1 時間 10 分かかったけど終わったよ。");
    expect(p.some((s) => s.includes("1 時間 10 分"))).toBe(true);
  });

  it("does not split runs of punctuation or leave a lone closing mark", () => {
    expect(phrases("ん？ どうしたの？")).toEqual(["ん？", " どうしたの？"]);
  });

  it("returns short text as a single phrase", () => {
    expect(phrases("呼んだ？")).toEqual(["呼んだ？"]);
  });
});
