import { describe, expect, it } from "vitest";

import { Acknowledged, triggerKey } from "./acknowledged";
import { hasContent, isEmpty, PANEL_MODES, panelView, planPanel, READ_LINGER_MS, showsCharacter } from "./panelModel";
import { session, snap } from "./testFixtures";

const ids = (list: { session_id: string }[]) => list.map((s) => s.session_id);

describe("planPanel", () => {
  it("lists every non-idle session as its own row, newest session on top", () => {
    const plan = planPanel(
      snap(
        session("w1", "working", 1),
        session("w2", "working", 2),
        session("idle", "idle", 3),
        session("ask", "waiting", 4),
        session("d1", "done", 5),
        session("e1", "error", 6),
      ),
      new Acknowledged(),
    );
    expect(ids(plan.rows)).toEqual(["e1", "d1", "ask", "w2", "w1"]);
    expect(plan.moreRows).toBe(0);
  });

  it("keeps each row in place when statuses change", () => {
    const before = planPanel(
      snap(
        session("a", "working", 10, 10, 1),
        session("b", "working", 10, 10, 2),
        session("c", "working", 10, 10, 3),
      ),
      new Acknowledged(),
    );
    expect(ids(before.rows)).toEqual(["c", "b", "a"]);
    const after = planPanel(
      snap(
        session("a", "waiting", 20, 50, 1),
        session("b", "error", 30, 40, 2),
        session("c", "done", 40, 30, 3),
      ),
      new Acknowledged(),
    );
    expect(ids(after.rows)).toEqual(["c", "b", "a"]);
  });

  it("adds a new session on top without moving the others", () => {
    const old = [session("a", "waiting", 10, 10, 1), session("b", "working", 10, 10, 2)];
    const plan = planPanel(snap(...old, session("new", "working", 30, 30, 3)), new Acknowledged());
    expect(ids(plan.rows)).toEqual(["new", "b", "a"]);
  });

  it("puts sessions without started_at at the bottom regardless of updates", () => {
    const plan = planPanel(
      snap(
        session("old2", "waiting", 90, 100, 0),
        session("old1", "working", 80, 80, 0),
        session("a", "working", 5, 5, 5),
        session("b", "done", 6, 6, 6),
      ),
      new Acknowledged(),
    );
    expect(ids(plan.rows)).toEqual(["b", "a", "old1", "old2"]);
  });

  it("caps the rows at five and counts the rest", () => {
    const working = Array.from({ length: 6 }, (_, i) => session(`w${i}`, "working", i));
    const plan = planPanel(snap(session("a", "waiting", 10), ...working), new Acknowledged());
    expect(ids(plan.rows)).toEqual(["a", "w5", "w4", "w3", "w2"]);
    expect(plan.moreRows).toBe(2);
  });

  it("keeps an old waiting session when capping, still in started_at order", () => {
    const working = Array.from({ length: 6 }, (_, i) => session(`w${i}`, "working", 10 + i, 10 + i, 10 + i));
    const plan = planPanel(snap(session("ask", "waiting", 50, 50, 1), ...working), new Acknowledged());
    expect(ids(plan.rows)).toEqual(["w5", "w4", "w3", "w2", "ask"]);
    expect(plan.moreRows).toBe(2);
  });

  it("is empty when only idle sessions exist", () => {
    expect(isEmpty(planPanel(snap(session("i", "idle", 1)), new Acknowledged()))).toBe(true);
    expect(isEmpty(planPanel(null, new Acknowledged()))).toBe(true);
  });
});

describe("acknowledged sessions", () => {
  it("keeps a seen done row dimmed for a while, then folds it", () => {
    const ack = new Acknowledged();
    const done = session("a", "done", 10);
    ack.add(triggerKey(done), 1000);
    const seen = planPanel(snap(done), ack, 1000 + READ_LINGER_MS - 1);
    expect(ids(seen.rows)).toEqual(["a"]);
    expect(seen.read.has("a")).toBe(true);
    expect(ids(planPanel(snap(done), ack, 1000 + READ_LINGER_MS).rows)).toEqual([]);
  });

  it("shows the next completion of the same session as unread", () => {
    const ack = new Acknowledged();
    ack.add(triggerKey(session("a", "done", 10)), 1000);
    // 次のプロンプトで作業中を経て、再び完了した。
    const again = session("a", "done", 20);
    ack.prune(snap(again));
    const next = planPanel(snap(again), ack, 1001);
    expect(ids(next.rows)).toEqual(["a"]);
    expect(next.read.has("a")).toBe(false);
  });

  it("does not bring back completions restored from a previous run", () => {
    const ack = new Acknowledged();
    const done = session("a", "done", 10);
    ack.restore([triggerKey(done)]);
    expect(ids(planPanel(snap(done), ack).rows)).toEqual([]);
  });

  it("keeps a seen done row in its place and dims it", () => {
    const ack = new Acknowledged();
    const sessions = () =>
      snap(
        session("w", "working", 1, 1, 1),
        session("seen", "done", 50, 50, 2),
        session("new", "done", 10, 10, 3),
      );
    expect(ids(planPanel(sessions(), ack, 2000).rows)).toEqual(["new", "seen", "w"]);
    ack.add(triggerKey(session("seen", "done", 50)), 1000);
    const plan = planPanel(sessions(), ack, 2000);
    expect(ids(plan.rows)).toEqual(["new", "seen", "w"]);
    expect([...plan.read]).toEqual(["seen"]);
  });

  it("gives up room for seen done rows first when capping", () => {
    const ack = new Acknowledged();
    const seen = session("seen", "done", 1, 1, 100);
    ack.add(triggerKey(seen), 1000);
    const working = Array.from({ length: 5 }, (_, i) => session(`w${i}`, "working", i));
    const plan = planPanel(snap(seen, ...working), ack, 2000);
    expect(ids(plan.rows)).toEqual(["w4", "w3", "w2", "w1", "w0"]);
    expect(plan.moreRows).toBe(1);
  });

  it("keeps waiting and error rows even when seen", () => {
    const ack = new Acknowledged();
    const waiting = session("w", "waiting", 1);
    const error = session("e", "error", 2);
    ack.add(triggerKey(waiting));
    ack.add(triggerKey(error));
    expect(ids(planPanel(snap(waiting, error), ack).rows)).toEqual(["e", "w"]);
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
      { status: "working", count: 3 },
      { status: "done", count: 2 },
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

  it("targets a done session when it is the only one needing attention", () => {
    const v = panelView(snap(session("w", "working", 5), session("d", "done", 1)), new Acknowledged(), "counts");
    expect(v.target?.session_id).toBe("d");
  });

  it("targets the highest-priority session even when the panel rows are in started_at order", () => {
    const v = panelView(
      snap(
        session("ask", "waiting", 50, 50, 1),
        session("e", "error", 40, 40, 2),
        session("d", "done", 30, 30, 3),
        session("w", "working", 20, 20, 4),
      ),
      new Acknowledged(),
      "detail",
    );
    expect(ids(v.plan.rows)).toEqual(["w", "d", "e", "ask"]);
    expect(v.target?.session_id).toBe("ask");
  });

  it("shows the same rows in list mode as in detail mode", () => {
    const detail = panelView(sessions(), new Acknowledged(), "detail");
    const list = panelView(sessions(), new Acknowledged(), "list");
    expect(ids(list.plan.rows)).toEqual(ids(detail.plan.rows));
    expect(list.plan.moreRows).toBe(detail.plan.moreRows);
    expect(hasContent(list)).toBe(true);
  });

  it("keeps the panel in list mode when there are no rows so it can still be dragged and right-clicked", () => {
    const list = panelView(snap(session("i", "idle", 1)), new Acknowledged(), "list");
    expect(isEmpty(list.plan)).toBe(true);
    expect(hasContent(list)).toBe(true);
    expect(hasContent(panelView(null, new Acknowledged(), "list"))).toBe(true);
  });

  it("hides the character only in list mode, which sits between counts and picture", () => {
    expect(PANEL_MODES).toEqual(["detail", "counts", "list", "picture"]);
    expect(PANEL_MODES.filter((m) => !showsCharacter(m))).toEqual(["list"]);
  });

  it("has no click target when nothing needs attention", () => {
    const v = panelView(snap(session("w", "working", 1)), new Acknowledged(), "counts");
    expect(v.target).toBeNull();
    expect(v.counts).toEqual([{ status: "working", count: 1 }]);
  });
});
