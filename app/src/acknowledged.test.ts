import { describe, expect, it } from "vitest";
import { Acknowledged, triggerKey, withAcknowledged } from "./acknowledged";
import { session, snap } from "./testFixtures";

describe("withAcknowledged", () => {
  it("keeps working above an unread done session", () => {
    const s = snap(session("a", "done", 10), session("b", "working", 20));
    expect(withAcknowledged(s, new Acknowledged()).aggregate).toBe("working");
  });

  it("stops letting acknowledged done sessions drive the aggregate", () => {
    const done = session("a", "done", 10);
    const s = snap(done, session("b", "idle", 20));
    const ack = new Acknowledged();
    expect(withAcknowledged(s, ack).aggregate).toBe("done");
    ack.add(triggerKey(done));
    expect(withAcknowledged(s, ack).aggregate).toBe("idle");
    // 読んでいない完了が他に残っていれば、完了のままにする。
    expect(withAcknowledged(snap(done, session("c", "done", 20)), ack).aggregate).toBe("done");
  });

  it("does not lower waiting or error even if they were acknowledged", () => {
    const w = session("a", "waiting", 10);
    const e = session("b", "error", 20);
    const ack = new Acknowledged();
    ack.add(triggerKey(w));
    ack.add(triggerKey(e));
    expect(withAcknowledged(snap(w, e), ack).aggregate).toBe("waiting");
    expect(withAcknowledged(snap(e), ack).aggregate).toBe("error");
  });
});

describe("Acknowledged persistence", () => {
  it("reports additions and pruning but not restores or repeats", () => {
    const saved: string[][] = [];
    const ack = new Acknowledged((keys) => saved.push(keys));
    const a = session("a", "done", 10);
    const b = session("b", "done", 20);
    ack.restore([triggerKey(a), triggerKey(b)]);
    expect(saved).toEqual([]);
    ack.add(triggerKey(a));
    expect(saved).toEqual([]);
    ack.prune(snap(a));
    expect(saved).toEqual([[triggerKey(a)]]);
    ack.prune(snap(a));
    expect(saved).toHaveLength(1);
  });
});
