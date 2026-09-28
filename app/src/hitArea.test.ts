import { describe, expect, it } from "vitest";

import { dilate, hitRegions, maskFromAlpha, type Portrait } from "./hitArea";
import { showsCharacter } from "./panelModel";

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

describe("hit regions", () => {
  const panel = { x: 8, y: 200, w: 312, h: 90 };
  const box = { x: 328, y: 120, w: 180, h: 270 };
  const mask = { cols: 2, rows: 1, bits: "10" };

  it("sends the portrait as a mask next to the panel", () => {
    expect(hitRegions([panel, null], { box, mask })).toEqual({ rects: [panel], mask: { ...box, ...mask } });
  });

  it("uses the whole portrait box when there is no mask", () => {
    expect(hitRegions([panel], { box, mask: null })).toEqual({ rects: [panel, box], mask: null });
  });

  it("leaves the portrait out in list mode so clicks there pass through", () => {
    const portrait: Portrait = { box, mask };
    const regions = hitRegions([panel], showsCharacter("list") ? portrait : null);
    expect(regions).toEqual({ rects: [panel], mask: null });
    for (const mode of ["detail", "counts", "picture"] as const) {
      expect(hitRegions([panel], showsCharacter(mode) ? portrait : null).mask).not.toBeNull();
    }
  });
});
