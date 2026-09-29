import { describe, expect, it } from "vitest";

import { opacityPercent, placePopover } from "./opacityPopover";

describe("opacityPercent", () => {
  it("rounds to a whole percent and keeps it in range", () => {
    const cases: [number, number][] = [
      [0.8, 80],
      [0.35000000000000003, 35],
      [1, 100],
      [0.05, 20],
      [-1, 20],
      [3, 100],
      [Number.NaN, 80],
      [Number.POSITIVE_INFINITY, 80],
    ];
    for (const [opacity, percent] of cases) {
      expect(opacityPercent(opacity), String(opacity)).toBe(percent);
    }
  });
});

describe("placePopover", () => {
  it("sits above the tab row, or below it when the window top is too close, always inside the window", () => {
    const size = { w: 220, h: 30 };
    const view = { w: 516, h: 398 };
    const cases: [string, { x: number; y: number; w: number; h: number }, { x: number; y: number }][] = [
      // パネルの幅 312 の左右の中央に揃え、タブの列の 4px 上に置く。
      ["above the tab row", { x: 16, y: 300, w: 296, h: 20 }, { x: 54, y: 266 }],
      ["just enough room above", { x: 16, y: 42, w: 296, h: 20 }, { x: 54, y: 8 }],
      // 立ち絵を隠してパネルが窓の上端まで伸びたときは、タブの列の下に重ねる。
      ["below when the top is too close", { x: 16, y: 8, w: 296, h: 20 }, { x: 54, y: 32 }],
      ["kept inside the left edge", { x: 0, y: 300, w: 40, h: 20 }, { x: 8, y: 266 }],
      ["kept inside the right edge", { x: 480, y: 300, w: 36, h: 20 }, { x: 288, y: 266 }],
      ["kept inside the bottom edge", { x: 16, y: 20, w: 296, h: 380 }, { x: 54, y: 360 }],
    ];
    for (const [label, anchor, expected] of cases) {
      expect(placePopover(anchor, size, view), label).toEqual(expected);
    }
  });
});
