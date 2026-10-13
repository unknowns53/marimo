import { describe, expect, it } from "vitest";

import { Acknowledged, triggerKey } from "./acknowledged";
import {
  hasContent,
  type PanelStyle,
  panelView,
  planPanel,
  type RowOrder,
  type TrayList,
  trayList,
} from "./panelModel";
import { session, snap } from "./testFixtures";
import type { SessionState, Snapshot } from "./types";

const ids = (list: { session_id: string }[]) => list.map((s) => s.session_id);

describe("planPanel", () => {
  it("lists every non-idle session and every expired cloud observation as its own row, newest session on top", () => {
    const cloud = { thread_id: "c1", observed_at: 7, expires_at: 8, expired: true };
    const plan = planPanel(
      snap(
        session("w1", "working", 1),
        session("w2", "working", 2),
        session("idle", "idle", 3),
        session("ask", "waiting", 4),
        session("d1", "done", 5),
        session("e1", "error", 6),
        { ...session("cloud-c1", "idle", 7), cloud },
      ),
      new Acknowledged(),
    );
    expect(ids(plan.rows)).toEqual(["cloud-c1", "e1", "d1", "ask", "w2", "w1"]);
  });

  it("keeps rows in started_at order through status changes and puts legacy sessions at the bottom", () => {
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
    // started_at を持たない古いファイルのセッションは、更新が新しくても一番下に置く。
    const legacy = planPanel(
      snap(
        session("old2", "waiting", 90, 100, 0),
        session("old1", "working", 80, 80, 0),
        session("a", "working", 5, 5, 5),
        session("b", "done", 6, 6, 6),
      ),
      new Acknowledged(),
    );
    expect(ids(legacy.rows)).toEqual(["b", "a", "old1", "old2"]);
  });

  it("shows every row in the chosen order, kept read rows in place, ties broken by session key", () => {
    const ack = new Acknowledged();
    const seen = session("c", "done", 70, 70, 7);
    ack.add(triggerKey(seen));
    const sessions = snap(
      session("a", "waiting", 90, 90, 1),
      session("b", "working", 60, 60, 6),
      seen,
      session("d", "done", 30, 30, 3),
      session("e", "error", 20, 20, 2),
      session("g", "working", 50, 50, 5),
      session("f", "working", 50, 50, 5),
      session("h", "waiting", 40, 40, 4),
    );
    const cases: [RowOrder, string[]][] = [
      ["started", ["c", "b", "f", "g", "h", "d", "e", "a"]],
      ["status", ["a", "h", "e", "b", "f", "g", "c", "d"]],
      ["updated", ["a", "c", "b", "f", "g", "h", "d", "e"]],
    ];
    for (const [order, expected] of cases) {
      expect(ids(planPanel(sessions, ack, order, 2000, new Set([triggerKey(seen)])).rows), order).toEqual(expected);
    }
  });
});

describe("acknowledged sessions", () => {
  it("keeps only the read rows in the keep set in place and lists the other read ones finished today, newest first", () => {
    const at = (day: number, hour: number, minute = 0) => new Date(2026, 9, day, hour, minute).getTime();
    const working = session("w", "working", at(13, 9), at(13, 9), at(13, 9));
    const unread = session("unread", "done", at(13, 12), at(13, 12), at(13, 10));
    const kept = session("kept", "done", at(13, 14), at(13, 14), at(13, 8));
    const late = session("late", "done", at(13, 13), at(13, 13), at(13, 7));
    const tie = session("tie", "done", at(13, 13), at(13, 13), at(13, 6));
    const midnight = session("midnight", "done", at(13, 0), at(13, 0), at(13, 5));
    const yesterday = session("yesterday", "done", at(12, 23, 59), at(12, 23, 59), at(12, 4));
    const ack = new Acknowledged();
    for (const s of [kept, late, tie, midnight, yesterday]) ack.add(triggerKey(s));
    const sessions = snap(working, unread, kept, late, tie, midnight, yesterday);
    const cases: [string, number, string[], string[], string[], string[]][] = [
      ["nothing kept", at(13, 15), [], ["unread", "w"], [], ["kept", "late", "tie", "midnight"]],
      ["kept row", at(13, 15), [triggerKey(kept)], ["unread", "w", "kept"], ["kept"], ["late", "tie", "midnight"]],
      ["key of an unread row", at(13, 15), [triggerKey(unread)], ["unread", "w"], [], ["kept", "late", "tie", "midnight"]],
      ["next day", at(14, 0, 30), [], ["unread", "w"], [], []],
    ];
    for (const [label, now, keep, rows, read, today] of cases) {
      const plan = planPanel(sessions, ack, "started", now, new Set(keep));
      expect(ids(plan.rows), label).toEqual(rows);
      expect([...plan.read], label).toEqual(read);
      expect(ids(plan.today), label).toEqual(today);
    }
  });

  it("shows the next completion of the same session as unread", () => {
    const ack = new Acknowledged();
    ack.add(triggerKey(session("a", "done", 10)));
    // 次のプロンプトで作業中を経て、再び完了した。
    const again = session("a", "done", 20);
    ack.prune(snap(again));
    const next = planPanel(snap(again), ack, "started", 1001);
    expect(ids(next.rows)).toEqual(["a"]);
    expect(next.read.has("a")).toBe(false);
  });

  it("does not bring back completions restored from a previous run", () => {
    const ack = new Acknowledged();
    const done = session("a", "done", 10);
    ack.restore([triggerKey(done)]);
    expect(ids(planPanel(snap(done), ack).rows)).toEqual([]);
  });

  it("keeps a Claude Code and a Codex session with the same id apart", () => {
    const ack = new Acknowledged();
    const claudeDone = session("same", "done", 10, 10, 1);
    const codexDone = { ...session("same", "done", 10, 10, 2), provider: "codex" as const };
    // Claude Code の鍵は以前の版と同じなので、保存してある既読の記録がそのまま効く。
    expect([triggerKey(claudeDone), triggerKey(codexDone)]).toEqual(["same:done:10", "codex:same:done:10"]);
    expect(planPanel(snap(claudeDone, codexDone), ack).rows).toHaveLength(2);
    const keep = new Set([triggerKey(claudeDone), triggerKey(codexDone)]);
    ack.add(triggerKey(claudeDone));
    const plan = planPanel(snap(claudeDone, codexDone), ack, "started", 1001, keep);
    expect([...plan.read]).toEqual(["same"]);
    ack.add(triggerKey(codexDone));
    expect([...planPanel(snap(claudeDone, codexDone), ack, "started", 1001, keep).read]).toEqual(["same", "codex:same"]);
  });

  it("keeps a Hermes session apart from the others with the same id", () => {
    const claudeDone = session("same", "done", 10, 10, 1);
    const hermesDone = { ...session("same", "done", 10, 10, 2), provider: "hermes" as const };
    expect(triggerKey(hermesDone)).toBe("hermes:same:done:10");
    const ack = new Acknowledged();
    ack.add(triggerKey(hermesDone));
    const keep = new Set([triggerKey(claudeDone), triggerKey(hermesDone)]);
    expect([...planPanel(snap(claudeDone, hermesDone), ack, "started", 1001, keep).read]).toEqual(["hermes:same"]);
  });

  it("keeps waiting and error rows even when seen", () => {
    const ack = new Acknowledged();
    const waiting = session("w", "waiting", 1);
    const error = session("e", "error", 2);
    ack.add(triggerKey(waiting));
    ack.add(triggerKey(error));
    expect(ids(planPanel(snap(waiting, error), ack).rows)).toEqual(["e", "w"]);
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

  it("decides per style whether there are rows to show instead of the empty line", () => {
    const idle = snap(session("i", "idle", 1));
    // その場所に残した既読の完了は詳細では薄く出るが、件数には数えない。
    const ack = new Acknowledged();
    const seen = session("seen", "done", 10);
    ack.add(triggerKey(seen));
    const keep = new Set([triggerKey(seen)]);
    const cases: [string, Snapshot | null, Acknowledged, PanelStyle, boolean][] = [
      ["detail with sessions", sessions(), new Acknowledged(), "detail", true],
      ["counts with sessions", sessions(), new Acknowledged(), "counts", true],
      ["detail with only idle sessions", idle, new Acknowledged(), "detail", false],
      ["counts with only idle sessions", idle, new Acknowledged(), "counts", false],
      ["detail without a snapshot", null, new Acknowledged(), "detail", false],
      ["detail with a kept seen completion", snap(seen), ack, "detail", true],
      ["counts with a kept seen completion", snap(seen), ack, "counts", false],
    ];
    for (const [label, snapshot, acks, style, expected] of cases) {
      expect(hasContent(panelView(snapshot, acks, style, "started", Date.now(), keep)), label).toBe(expected);
    }
  });

  it("targets the highest-priority session needing attention, or nothing", () => {
    const byPriority = snap(
      session("ask", "waiting", 50, 50, 1),
      session("e", "error", 40, 40, 2),
      session("d", "done", 30, 30, 3),
      session("w", "working", 20, 20, 4),
    );
    const cases: [string, Snapshot, string | null][] = [
      ["done is the only one needing attention", snap(session("w", "working", 5), session("d", "done", 1)), "d"],
      ["priority wins over started_at row order", byPriority, "ask"],
      ["nothing needs attention", snap(session("w", "working", 1)), null],
    ];
    for (const [label, snapshot, expected] of cases) {
      expect(panelView(snapshot, new Acknowledged(), "counts").target?.session_id ?? null, label).toBe(expected);
    }
    expect(ids(panelView(byPriority, new Acknowledged(), "detail").plan.rows)).toEqual(["w", "d", "e", "ask"]);
    expect(panelView(snap(session("w", "working", 1)), new Acknowledged(), "counts").counts).toEqual([
      { status: "working", count: 1 },
    ]);
  });
});

describe("trayList", () => {
  it("lists waiting, error and unread done sessions, newest first within a status, with clipped titles and an overflow count", () => {
    const seen = session("seen", "done", 50);
    const ack = new Acknowledged();
    ack.add(triggerKey(seen));
    const longName = "n".repeat(36);
    const cases: [string, SessionState[], TrayList][] = [
      [
        "order and unread filter",
        [
          session("w1", "waiting", 10),
          session("w2", "waiting", 20),
          session("e1", "error", 30),
          session("d1", "done", 40),
          seen,
          session("busy", "working", 60),
          session("idle", "idle", 70),
        ],
        {
          items: [
            { key: "w2", label: "承認待ち  w2" },
            { key: "w1", label: "承認待ち  w1" },
            { key: "e1", label: "エラー  e1" },
            { key: "d1", label: "完了  d1" },
          ],
          more: 0,
        },
      ],
      [
        "titles, clipped to about 40 characters",
        [
          { ...session("short", "done", 1), title: "短い題名" },
          { ...session("long", "error", 2), title: "あ".repeat(50) },
          { ...session(longName, "error", 1), title: "い".repeat(20) },
          { ...session("chat", "waiting", 3), provider: "hermes", cwd: null, title: "#general" },
        ],
        {
          items: [
            { key: "hermes:chat", label: "承認待ち  #general" },
            { key: "long", label: `エラー  long  ${"あ".repeat(28)}…` },
            { key: longName, label: `エラー  ${longName}  ${"い".repeat(9)}…` },
            { key: "short", label: "完了  short  短い題名" },
          ],
          more: 0,
        },
      ],
      [
        "overflow",
        Array.from({ length: 12 }, (_, i) => session(`w${String(i).padStart(2, "0")}`, "waiting", i)),
        {
          items: Array.from({ length: 10 }, (_, i) => {
            const id = `w${String(11 - i).padStart(2, "0")}`;
            return { key: id, label: `承認待ち  ${id}` };
          }),
          more: 2,
        },
      ],
    ];
    for (const [name, sessions, expected] of cases) {
      expect(trayList(snap(...sessions), ack), name).toEqual(expected);
    }
    expect(trayList(null, ack)).toEqual({ items: [], more: 0 });
  });
});
