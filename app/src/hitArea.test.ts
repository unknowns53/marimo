import { describe, expect, it } from "vitest";

import { dilate, maskFromAlpha } from "./hitArea";

describe("hit mask", () => {
  it("marks cells above the alpha threshold and widens them by one cell", () => {
    // 5×3 の格子で、中央の 1 マスだけが不透明。
    const alpha = [0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0];
    expect(maskFromAlpha(alpha, 5, 3).bits).toBe("011100111001110");
  });

  it("ignores faint edges below the threshold", () => {
    expect(maskFromAlpha([10, 20, 24, 0], 4, 1).bits).toBe("0000");
  });

  it("does not wrap around rows when widening", () => {
    const solid = [false, false, true, false, false, false];
    expect(dilate(solid, 3, 2)).toEqual([false, true, true, false, true, true]);
  });
});
