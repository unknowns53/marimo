import { describe, expect, it } from "vitest";

import { categoryFor, fillTemplate, formatDuration, linesFor, mergeDialogue } from "./dialogue";
import { session } from "./testFixtures";
import type { SessionState, Status } from "./types";

const MIN = 60_000;

function s(status: Status, extra: Partial<SessionState> = {}): SessionState {
  return { ...session("marimo", status, 100 * MIN), ...extra };
}

describe("categoryFor", () => {
  it("picks the category from the reason and the turn duration", () => {
    const done = (minutes: number) => s("done", { turn_started_at: (100 - minutes) * MIN });
    const cases: [string, SessionState, string][] = [
      ["permission", s("waiting", { status_reason: "permission" }), "waiting.permission"],
      ["question", s("waiting", { status_reason: "question" }), "waiting.question"],
      ["plan", s("waiting", { status_reason: "plan" }), "waiting.plan"],
      ["waiting without a reason", s("waiting"), "waiting"],
      ["done in 1 min", done(1), "done.short"],
      ["done in 2 min", done(2), "done"],
      ["done in 14 min", done(14), "done"],
      ["done in 15 min", done(15), "done.long"],
      ["done without a turn start", s("done"), "done"],
      ["done with a turn start after the status", s("done", { turn_started_at: 200 * MIN }), "done"],
      // StopFailure の error のうち利用制限を表すのは rate_limit だけである。
      ["rate_limit", s("error", { status_reason: "rate_limit" }), "error.rate_limit"],
      ["overloaded", s("error", { status_reason: "overloaded" }), "error"],
      ["error without a reason", s("error"), "error"],
    ];
    for (const [label, state, expected] of cases) {
      expect(categoryFor(state), label).toBe(expected);
    }
  });
});

describe("linesFor", () => {
  it("falls back to parent categories", () => {
    const d = { waiting: ["w"], "done.long": ["long"], done: ["d"] };
    expect(linesFor(d, "waiting.permission")).toEqual(["w"]);
    expect(linesFor(d, "done.long")).toEqual(["long"]);
    expect(linesFor(d, "done.short")).toEqual(["d"]);
    expect(linesFor({ "a.b": [] , a: ["x"] }, "a.b.c")).toEqual(["x"]);
    expect(linesFor(d, "error.rate_limit")).toEqual([]);
  });
});

describe("fillTemplate", () => {
  it("fills folder and duration in Japanese", () => {
    const durations: [number, string][] = [
      [30_000, "1 分足らず"],
      [25 * MIN + 59_000, "25 分"],
      [60 * MIN, "1 時間"],
      [70 * MIN, "1 時間 10 分"],
    ];
    for (const [ms, expected] of durations) {
      expect(formatDuration(ms), `${ms} ms`).toBe(expected);
    }
    const done = s("done", { turn_started_at: 75 * MIN });
    expect(fillTemplate("{folder}、{duration}かかったけど終わったよ。", done)).toBe(
      "marimo、25 分かかったけど終わったよ。",
    );
    expect(fillTemplate("{folder} だよ {duration}", s("done"))).toBe("marimo だよ ");
    expect(fillTemplate("呼んだ？", null)).toBe("呼んだ？");
  });
});

describe("mergeDialogue", () => {
  it("overrides only the valid categories the user wrote", () => {
    const defaults = { waiting: ["既定"], done: ["既定の完了"], reaction: ["ん？"] };
    expect(mergeDialogue(defaults, { done: ["自分の完了"] })).toEqual({
      waiting: ["既定"],
      done: ["自分の完了"],
      reaction: ["ん？"],
    });
    expect(mergeDialogue(defaults, null)).toEqual(defaults);
    expect(mergeDialogue(defaults, [1, 2])).toEqual(defaults);
    expect(mergeDialogue(defaults, { done: [], waiting: "x", reaction: [1] })).toEqual(defaults);
  });
});
