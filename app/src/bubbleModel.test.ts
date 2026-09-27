import { describe, expect, it } from "vitest";

import { Acknowledged } from "./acknowledged";
import { BubbleModel, fillFolder } from "./bubbleModel";
import { session, snap } from "./testFixtures";
import type { Dialogue } from "./types";

const DIALOGUE: Dialogue = {
  waiting: ["{folder} で確認をお願いしたいことがあるの", "もう一つの言い方"],
  done: ["{folder} の作業が終わったわ"],
  error: ["{folder} で困ったことになったわ"],
};

function model(): BubbleModel {
  // テストでは常に最初のセリフを選び、結果を決まったものにする。
  return new BubbleModel(new Acknowledged(), (lines) => lines[0]);
}

describe("BubbleModel", () => {
  it("keeps the bubble while the triggering state lasts", () => {
    const m = model();
    expect(m.update(snap(session("marimo", "working", 1)), DIALOGUE)).toBeNull();
    const shown = m.update(snap(session("marimo", "waiting", 2)), DIALOGUE);
    expect(shown?.text).toBe("marimo で確認をお願いしたいことがあるの");
    // 同じきっかけの間は、後から届いた更新でも同じ吹き出しを保つ。
    expect(m.update(snap(session("marimo", "waiting", 2, 9)), DIALOGUE)).toBe(shown);
    // 承認されて作業中へ戻ったら消える。
    expect(m.update(snap(session("marimo", "working", 10)), DIALOGUE)).toBeNull();
  });

  it("keeps done until the next prompt and error until the state changes", () => {
    const m = model();
    expect(m.update(snap(session("a", "done", 5)), DIALOGUE)?.text).toBe("a の作業が終わったわ");
    expect(m.update(snap(session("a", "done", 5, 50)), DIALOGUE)?.text).toBe("a の作業が終わったわ");
    expect(m.update(snap(session("a", "working", 60)), DIALOGUE)).toBeNull();
    expect(m.update(snap(session("a", "error", 70)), DIALOGUE)?.text).toBe("a で困ったことになったわ");
    expect(m.update(snap(session("a", "idle", 80)), DIALOGUE)).toBeNull();
  });

  it("does not bring back a dismissed bubble for the same trigger", () => {
    const m = model();
    m.update(snap(session("a", "waiting", 2)), DIALOGUE);
    m.dismiss();
    expect(m.view).toBeNull();
    expect(m.needsText(snap(session("a", "waiting", 2, 5)))).toBe(false);
    expect(m.update(snap(session("a", "waiting", 2, 5)), DIALOGUE)).toBeNull();
    // 一度別の状態を経て再び承認待ちになれば、新しいきっかけとして出す。
    m.update(snap(session("a", "working", 6)), DIALOGUE);
    expect(m.update(snap(session("a", "waiting", 7)), DIALOGUE)?.sessionId).toBe("a");
  });

  it("shows another session that becomes waiting after one was dismissed", () => {
    const m = model();
    m.update(snap(session("a", "waiting", 2)), DIALOGUE);
    m.dismiss();
    const view = m.update(snap(session("a", "waiting", 2), session("b", "waiting", 3)), DIALOGUE);
    expect(view?.text).toBe("b で確認をお願いしたいことがあるの");
  });

  it("switches to the next waiting session when the current one resolves", () => {
    const m = model();
    const first = m.update(snap(session("a", "waiting", 2), session("b", "waiting", 3)), DIALOGUE);
    expect(first?.sessionId).toBe("b");
    // a の更新が新しくなって並びが入れ替わっても、b の吹き出しを保つ。
    expect(
      m.update(snap(session("a", "waiting", 2, 20), session("b", "waiting", 3)), DIALOGUE)?.sessionId,
    ).toBe("b");
    const next = m.update(snap(session("a", "waiting", 2, 20), session("b", "working", 30)), DIALOGUE);
    expect(next?.sessionId).toBe("a");
  });

  it("follows the aggregate across different states", () => {
    const m = model();
    expect(m.update(snap(session("a", "done", 1), session("b", "error", 2)), DIALOGUE)?.sessionId).toBe("b");
    expect(m.update(snap(session("a", "done", 1), session("b", "working", 3)), DIALOGUE)?.sessionId).toBe("a");
  });

  it("keeps the same line while the trigger lasts even with random picks", () => {
    let calls = 0;
    const m = new BubbleModel(new Acknowledged(), (lines) => lines[calls++ % lines.length]);
    const a = m.update(snap(session("a", "waiting", 1)), DIALOGUE)?.text;
    for (let i = 2; i < 6; i++) {
      expect(m.update(snap(session("a", "waiting", 1, i)), DIALOGUE)?.text).toBe(a);
    }
    expect(calls).toBe(1);
  });

  it("works with dialogue lacking {folder} or lines for a state", () => {
    const m = model();
    expect(m.update(snap(session("a", "waiting", 1)), { waiting: ["確認してね"] })?.text).toBe("確認してね");
    const n = model();
    expect(n.update(snap(session("a", "done", 1)), { waiting: ["x"] })).toBeNull();
    expect(fillFolder("{folder} と {folder}", "m")).toBe("m と m");
  });
});

describe("Speech", () => {
  it("lets notification bubbles win over the touch reaction", async () => {
    const { Speech } = await import("./speech");
    const s = new Speech();
    const status = { key: "k", sessionId: "a", status: "waiting" as const, text: "確認してね" };
    expect(s.react("ふふ", 0, 2500, status)).toBe(false);
    expect(s.current(status, 100)).toBe("確認してね");
    expect(s.react("ふふ", 0, 2500, null)).toBe(true);
    expect(s.current(null, 2499)).toBe("ふふ");
    expect(s.current(null, 2500)).toBeNull();
    // ひとことの途中で知らせが来たら、知らせに切り替えて、ひとことは戻さない。
    s.react("ふふ", 3000, 2500, null);
    expect(s.current(status, 3100)).toBe("確認してね");
    expect(s.current(null, 3200)).toBeNull();
  });
});
