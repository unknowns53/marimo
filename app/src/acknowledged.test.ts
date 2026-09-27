import { describe, expect, it } from "vitest";
import { Acknowledged, triggerKey, withAcknowledged } from "./acknowledged";
import { session, snap } from "./testFixtures";

describe("withAcknowledged", () => {
  it("falls back to working once the only done session has been acknowledged", () => {
    const done = session("a", "done", 10);
    const s = snap(done, session("b", "working", 20));
    const ack = new Acknowledged();
    expect(withAcknowledged(s, ack).aggregate).toBe("done");
    ack.add(triggerKey(done));
    expect(withAcknowledged(s, ack).aggregate).toBe("working");
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
