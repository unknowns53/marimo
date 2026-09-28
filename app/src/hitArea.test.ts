import { describe, expect, it } from "vitest";

import { type HitRegions, hitRegions, maskFromAlpha, type Portrait } from "./hitArea";
import { showsCharacter } from "./panelModel";

describe("hit mask", () => {
  it("marks cells above the alpha threshold and widens them by one cell within each row", () => {
    const cases: [string, number[], number, number, string][] = [
      // 5×3 の格子で、中央の 1 マスだけが不透明。
      ["widens by one cell", [0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0], 5, 3, "011100111001110"],
      ["ignores faint edges up to the threshold", [10, 20, 24, 0], 4, 1, "0000"],
      ["does not wrap around rows", [0, 0, 255, 0, 0, 0], 3, 2, "011011"],
    ];
    for (const [label, alpha, cols, rows, bits] of cases) {
      expect(maskFromAlpha(alpha, cols, rows).bits, label).toBe(bits);
    }
  });
});

describe("hit regions", () => {
  it("sends the panel and the portrait shape, or leaves a hidden portrait out", () => {
    const panel = { x: 8, y: 200, w: 312, h: 90 };
    const box = { x: 328, y: 120, w: 180, h: 270 };
    const mask = { cols: 2, rows: 1, bits: "10" };
    const portrait: Portrait = { box, mask };
    const cases: [string, HitRegions, HitRegions][] = [
      [
        "portrait as a mask next to the panel",
        hitRegions([panel, null], portrait),
        { rects: [panel], mask: { ...box, ...mask } },
      ],
      ["whole portrait box without a mask", hitRegions([panel], { box, mask: null }), { rects: [panel, box], mask: null }],
      // リストだけの段階では立ち絵を送らず、その場所のクリックを下のウィンドウへ通す。
      [
        "portrait left out in list mode",
        hitRegions([panel], showsCharacter("list") ? portrait : null),
        { rects: [panel], mask: null },
      ],
    ];
    for (const [label, actual, expected] of cases) {
      expect(actual, label).toEqual(expected);
    }
    for (const mode of ["detail", "counts", "picture"] as const) {
      expect(hitRegions([panel], showsCharacter(mode) ? portrait : null).mask, mode).not.toBeNull();
    }
  });
});
