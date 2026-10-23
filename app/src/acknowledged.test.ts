import { describe, expect, it } from "vitest";
import { Acknowledged, triggerKey, unreadMarkable, withAcknowledged } from "./acknowledged";
import { session, snap } from "./testFixtures";
import type { SessionState } from "./types";

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

  it("lowers acknowledged errors like done but never waiting", () => {
    const w = session("a", "waiting", 10);
    const e = session("b", "error", 20);
    const ack = new Acknowledged();
    ack.add(triggerKey(w));
    ack.add(triggerKey(e));
    expect(withAcknowledged(snap(w, e), ack).aggregate).toBe("waiting");
    expect(withAcknowledged(snap(e, session("c", "working", 30)), ack).aggregate).toBe("working");
  });
});

// Rust のメニューバーのアイコンも同じキーで既読を見るので、app/src-tauri/src/acknowledged.rs の
// trigger_keys_match_the_frontend と同じ文字列であることを確かめる。
describe("triggerKey", () => {
  it("matches the keys the Rust side builds", () => {
    const cases: [SessionState, string][] = [
      [session("abc", "done", 5), "abc:done:5"],
      [{ ...session("ses_x", "error", 1792165830895), provider: "opencode" }, "opencode:ses_x:error:1792165830895"],
    ];
    for (const [s, key] of cases) expect(triggerKey(s)).toBe(key);
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

describe("unreadMarkable", () => {
  it("lists only done and error sessions that are not acknowledged", () => {
    const read = session("a", "done", 10);
    const readError = session("e", "error", 50);
    const s = snap(read, session("b", "done", 20), session("c", "waiting", 30), session("d", "error", 40), readError);
    const ack = new Acknowledged();
    ack.add(triggerKey(read));
    ack.add(triggerKey(readError));
    expect(unreadMarkable(s, ack).map((x) => x.session_id)).toEqual(["d", "b"]);
    expect(unreadMarkable(null, ack)).toEqual([]);
  });
});
