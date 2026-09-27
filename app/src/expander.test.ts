import { describe, expect, it } from "vitest";

import { Expander, type ExpandTarget, type Point } from "./expander";

// 件数の行（y 300〜320）と、開いたときにその上へ伸びる層（y 200〜316）。層の下端と行の間に
// 隙間はないが、層の左右は行より 4 px 狭い。
const counts: ExpandTarget = {
  id: "counts",
  anchor: { x: 10, y: 300, w: 260, h: 20 },
  layer: { x: 14, y: 200, w: 252, h: 116 },
};
// 隙間のある作業中の要約（層が行から 8 px 離れて上にある）。
const working: ExpandTarget = {
  id: "working",
  anchor: { x: 10, y: 330, w: 260, h: 20 },
  layer: { x: 10, y: 250, w: 260, h: 72 },
};

type Step = [t: number, cursor: Point | null];

function run(steps: Step[], targets: ExpandTarget[] = [counts]): (string | null)[] {
  const e = new Expander({ openDelayMs: 100, closeDelayMs: 300 });
  return steps.map(([t, c]) => e.update(t, c, targets));
}

const onRow = { x: 100, y: 310 };
const onLayer = { x: 100, y: 220 };
const outside = { x: 100, y: 100 };

describe("Expander", () => {
  it("opens after resting on the row and stays open while the cursor is there", () => {
    expect(run([[0, onRow], [40, onRow], [99, onRow], [100, onRow], [140, onRow], [2000, onRow]])).toEqual([
      null,
      null,
      null,
      "counts",
      "counts",
      "counts",
    ]);
  });

  it("stays open while moving from the row into the layer", () => {
    expect(
      run([
        [0, onRow],
        [100, onRow],
        [140, { x: 100, y: 299 }],
        [180, { x: 100, y: 260 }],
        [220, onLayer],
        [1000, onLayer],
      ]),
    ).toEqual([null, "counts", "counts", "counts", "counts", "counts"]);
  });

  it("keeps a target open across the gap between the row and its layer", () => {
    const gap = { x: 100, y: 326 };
    expect(
      run(
        [
          [0, { x: 100, y: 340 }],
          [100, { x: 100, y: 340 }],
          [140, gap],
          [180, { x: 100, y: 300 }],
          [600, { x: 100, y: 300 }],
        ],
        [working],
      ),
    ).toEqual([null, "working", "working", "working", "working"]);
  });

  it("measures the close delay from the first time the cursor is seen outside", () => {
    // 行の上で長く止まった後に外へ出ても、出てから 300 ms は開いたままにする。
    expect(run([[0, onRow], [100, onRow], [5000, outside], [5299, outside], [5300, outside]])).toEqual([
      null,
      "counts",
      "counts",
      "counts",
      null,
    ]);
  });

  it("does not close when the cursor leaves and comes back quickly", () => {
    expect(
      run([
        [0, onRow],
        [100, onRow],
        [140, outside],
        [300, outside],
        [420, onLayer],
        [1000, onLayer],
      ]),
    ).toEqual([null, "counts", "counts", "counts", "counts", "counts"]);
  });

  it("closes only after the cursor has been outside for the close delay", () => {
    // 範囲の外で最初に見えたのが 140 ms なので、440 ms で閉じる。
    expect(run([[0, onRow], [100, onRow], [140, outside], [439, outside], [440, outside], [500, null]])).toEqual([
      null,
      "counts",
      "counts",
      "counts",
      null,
      null,
    ]);
  });

  it("does not open when the cursor merely crosses the row", () => {
    expect(run([[0, { x: 100, y: 295 }], [40, onRow], [80, { x: 100, y: 330 }], [120, { x: 100, y: 360 }]])).toEqual([
      null,
      null,
      null,
      null,
    ]);
  });

  it("switches to another row only after the first one has closed", () => {
    const e = new Expander({ openDelayMs: 100, closeDelayMs: 300 });
    const targets = [counts, working];
    e.update(0, onRow, targets);
    expect(e.update(100, onRow, targets)).toBe("counts");
    expect(e.update(140, { x: 100, y: 340 }, targets)).toBe("counts");
    expect(e.update(440, { x: 100, y: 340 }, targets)).toBeNull();
    expect(e.update(540, { x: 100, y: 340 }, targets)).toBe("working");
  });

  it("keeps the parent open while the cursor is on a child layer that sticks out", () => {
    const e = new Expander({ openDelayMs: 100, closeDelayMs: 300 });
    e.update(0, onRow, [counts]);
    e.update(100, onRow, [counts]);
    const childLayer = { x: 14, y: 150, w: 252, h: 60 };
    const aboveParent = { x: 100, y: 160 };
    expect(e.update(500, aboveParent, [counts], childLayer)).toBe("counts");
    expect(e.update(1000, aboveParent, [counts], childLayer)).toBe("counts");
  });

  it("reports when it needs to be asked again", () => {
    const e = new Expander({ openDelayMs: 100, closeDelayMs: 300 });
    expect(e.nextDeadline()).toBeNull();
    e.update(10, onRow, [counts]);
    expect(e.nextDeadline()).toBe(110);
    e.update(110, onRow, [counts]);
    expect(e.nextDeadline()).toBeNull();
    e.update(150, outside, [counts]);
    expect(e.nextDeadline()).toBe(450);
  });

  it("closes when the open target disappears from the panel", () => {
    const e = new Expander({ openDelayMs: 100, closeDelayMs: 300 });
    e.update(0, onRow, [counts]);
    e.update(100, onRow, [counts]);
    expect(e.update(120, onRow, [])).toBeNull();
  });
});
