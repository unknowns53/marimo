import { describe, expect, it } from "vitest";
import { Acknowledged, triggerKey, withAcknowledged } from "./acknowledged";
import { session, snap } from "./testFixtures";

describe("withAcknowledged", () => {
  it("keeps working above an unread done session", () => {
    const s = snap(session("a", "done", 10), session("b", "working", 20));
    expect(withAcknowledged(s, new Acknowledged()).aggregate).toBe("working");
  });

  it("falls back to idle once the only done session has been acknowledged", () => {
    const done = session("a", "done", 10);
    const s = snap(done, session("b", "idle", 20));
    const ack = new Acknowledged();
    expect(withAcknowledged(s, ack).aggregate).toBe("done");
    ack.add(triggerKey(done));
    expect(withAcknowledged(s, ack).aggregate).toBe("idle");
  });

  it("stays done while another done session is still unread", () => {
    const a = session("a", "done", 10);
    const s = snap(a, session("b", "done", 20));
    const ack = new Acknowledged();
    ack.add(triggerKey(a));
    expect(withAcknowledged(s, ack).aggregate).toBe("done");
  });

  it("becomes idle when every session is an acknowledged done", () => {
    const a = session("a", "done", 10);
    const ack = new Acknowledged();
    ack.add(triggerKey(a));
    expect(withAcknowledged(snap(a), ack).aggregate).toBe("idle");
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

  it("treats a new completion of the same session as unread", () => {
    const first = session("a", "done", 10);
    const ack = new Acknowledged();
    ack.add(triggerKey(first));
    expect(withAcknowledged(snap(session("a", "done", 99)), ack).aggregate).toBe("done");
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

  it("keeps a restored completion acknowledged after a restart", () => {
    const done = session("a", "done", 10);
    const ack = new Acknowledged();
    ack.restore([triggerKey(done)]);
    expect(withAcknowledged(snap(done), ack).aggregate).toBe("idle");
  });
});
