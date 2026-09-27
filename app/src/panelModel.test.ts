import { describe, expect, it } from "vitest";

import { Acknowledged, triggerKey } from "./acknowledged";
import { isEmpty, planPanel } from "./panelModel";
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
