import { describe, expect, it } from "vitest";

import { Acknowledged, triggerKey } from "./acknowledged";
import { hasContent, isEmpty, panelView, planPanel } from "./panelModel";
import { session, snap } from "./testFixtures";

const ids = (list: { session_id: string }[]) => list.map((s) => s.session_id);

describe("planPanel", () => {
  it("shows rows only for sessions that need attention and summarises working ones", () => {
    const plan = planPanel(
      snap(
        session("w1", "working", 1),
        session("w2", "working", 2),
        session("idle", "idle", 3),
        session("ask", "waiting", 4),
      ),
      new Acknowledged(),
    );
    expect(ids(plan.attention)).toEqual(["ask"]);
    expect(ids(plan.working)).toEqual(["w2", "w1"]);
    expect(plan.moreAttention).toBe(0);
  });

  it("keeps the attention order and caps it at three rows", () => {
    const plan = planPanel(
      snap(
        session("d1", "done", 1),
        session("e1", "error", 2),
        session("a1", "waiting", 3),
        session("d2", "done", 4),
        session("a2", "waiting", 5),
      ),
      new Acknowledged(),
    );
    expect(ids(plan.attention)).toEqual(["a2", "a1", "e1"]);
    expect(plan.moreAttention).toBe(2);
  });

  it("is empty when only idle sessions exist", () => {
    expect(isEmpty(planPanel(snap(session("i", "idle", 1)), new Acknowledged()))).toBe(true);
    expect(isEmpty(planPanel(null, new Acknowledged()))).toBe(true);
  });

  it("caps the working list", () => {
    const working = Array.from({ length: 7 }, (_, i) => session(`w${i}`, "working", i));
    const plan = planPanel(snap(...working), new Acknowledged());
    expect(plan.working).toHaveLength(5);
    expect(plan.moreWorking).toBe(2);
  });
});

describe("acknowledged sessions", () => {
  it("folds a done row once seen and shows it again on the next completion", () => {
    const ack = new Acknowledged();
    const done = session("a", "done", 10);
    ack.add(triggerKey(done));
    expect(ids(planPanel(snap(done), ack).attention)).toEqual([]);
    // 次のプロンプトで作業中を経て、再び完了した。
    const again = session("a", "done", 20);
    ack.prune(snap(again));
    expect(ids(planPanel(snap(again), ack).attention)).toEqual(["a"]);
  });

  it("keeps waiting and error rows even when seen", () => {
    const ack = new Acknowledged();
    const waiting = session("w", "waiting", 1);
    const error = session("e", "error", 2);
    ack.add(triggerKey(waiting));
    ack.add(triggerKey(error));
    expect(ids(planPanel(snap(waiting, error), ack).attention)).toEqual(["w", "e"]);
  });

  it("forgets keys whose trigger has ended", () => {
    const ack = new Acknowledged();
    const done = session("a", "done", 10);
    ack.add(triggerKey(done));
    ack.prune(snap(session("a", "working", 15)));
    expect(ack.has(done)).toBe(false);
  });
});

describe("panelView", () => {
  const sessions = () =>
    snap(
      session("ask", "waiting", 5),
      session("d1", "done", 1),
      session("d2", "done", 2),
      session("w1", "working", 3),
      session("w2", "working", 4),
      session("w3", "working", 6),
      session("i", "idle", 7),
    );

  it("counts sessions per status in priority order and skips zero counts", () => {
    const v = panelView(sessions(), new Acknowledged(), "counts");
    expect(v.counts).toEqual([
      { status: "waiting", count: 1 },
      { status: "done", count: 2 },
      { status: "working", count: 3 },
    ]);
    expect(v.target?.session_id).toBe("ask");
    expect(hasContent(v)).toBe(true);
  });

  it("does not count seen done sessions", () => {
    const ack = new Acknowledged();
    const s = sessions();
    for (const x of s.sessions.filter((x) => x.status === "done")) ack.add(triggerKey(x));
    const v = panelView(s, ack, "counts");
    expect(v.counts.find((c) => c.status === "done")).toBeUndefined();
  });

  it("has nothing to show when every count is zero or in picture mode", () => {
    const idle = snap(session("i", "idle", 1));
    expect(hasContent(panelView(idle, new Acknowledged(), "counts"))).toBe(false);
    expect(hasContent(panelView(idle, new Acknowledged(), "detail"))).toBe(false);
    expect(hasContent(panelView(sessions(), new Acknowledged(), "picture"))).toBe(false);
  });

  it("has no click target when nothing needs attention", () => {
    const v = panelView(snap(session("w", "working", 1)), new Acknowledged(), "counts");
    expect(v.target).toBeNull();
    expect(v.counts).toEqual([{ status: "working", count: 1 }]);
  });
});
