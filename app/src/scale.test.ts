import { describe, expect, it } from "vitest";

import { nearestPreset, SCALE_MAX, SCALE_MIN } from "./scale";

describe("nearestPreset", () => {
  it("snaps to the closest preset, with the smaller one winning an exact tie", () => {
    const cases: [number, number][] = [
      [0.6, 0.6],
      [1.1, 1.0],
      [1.4, 1.5],
      [1.9, 2.0],
      // 2 つの刻みのちょうど中間（2 進数で誤差の出ない値）。
      [1.25, 1.0],
      [1.75, 1.5],
    ];
    for (const [input, expected] of cases) {
      expect(nearestPreset(input), String(input)).toBe(expected);
    }
  });

  it("returns the end presets for values outside the allowed range", () => {
    expect(nearestPreset(0)).toBe(SCALE_MIN);
    expect(nearestPreset(-5)).toBe(SCALE_MIN);
    expect(nearestPreset(10)).toBe(SCALE_MAX);
  });
});
