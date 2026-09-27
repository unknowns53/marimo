import { describe, expect, it } from "vitest";

import {
  categoryFor,
  fillTemplate,
  formatDuration,
  linesFor,
  mergeDialogue,
  reactionCategory,
} from "./dialogue";
import { session } from "./testFixtures";
import type { SessionState, Status } from "./types";

const MIN = 60_000;

function s(status: Status, extra: Partial<SessionState> = {}): SessionState {
  return { ...session("marimo", status, 100 * MIN), ...extra };
}

describe("categoryFor", () => {
  it("picks the waiting category from the reason", () => {
    expect(categoryFor(s("waiting", { status_reason: "permission" }))).toBe("waiting.permission");
    expect(categoryFor(s("waiting", { status_reason: "question" }))).toBe("waiting.question");
    expect(categoryFor(s("waiting", { status_reason: "plan" }))).toBe("waiting.plan");
    expect(categoryFor(s("waiting"))).toBe("waiting");
  });

  it("picks the done category from the turn duration", () => {
    const at = (minutes: number) => s("done", { turn_started_at: (100 - minutes) * MIN });
    expect(categoryFor(at(1))).toBe("done.short");
    expect(categoryFor(at(2))).toBe("done");
    expect(categoryFor(at(14))).toBe("done");
    expect(categoryFor(at(15))).toBe("done.long");
    expect(categoryFor(s("done"))).toBe("done");
    expect(categoryFor(s("done", { turn_started_at: 200 * MIN }))).toBe("done");
  });

  it("maps only rate_limit to the rate limit error", () => {
    expect(categoryFor(s("error", { status_reason: "rate_limit" }))).toBe("error.rate_limit");
    expect(categoryFor(s("error", { status_reason: "overloaded" }))).toBe("error");
    expect(categoryFor(s("error"))).toBe("error");
  });

  it("chooses the reaction category from the aggregate", () => {
    expect(reactionCategory("working")).toBe("reaction.working");
    expect(reactionCategory("idle")).toBe("reaction");
    expect(reactionCategory("done")).toBe("reaction");
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

describe("formatDuration and fillTemplate", () => {
  it("formats durations in Japanese", () => {
    expect(formatDuration(30_000)).toBe("1 分足らず");
    expect(formatDuration(25 * MIN + 59_000)).toBe("25 分");
    expect(formatDuration(60 * MIN)).toBe("1 時間");
    expect(formatDuration(70 * MIN)).toBe("1 時間 10 分");
  });

  it("fills folder and duration", () => {
    const done = s("done", { turn_started_at: 75 * MIN });
    expect(fillTemplate("{folder}、{duration}かかったけど終わったよ。", done)).toBe(
      "marimo、25 分かかったけど終わったよ。",
    );
    expect(fillTemplate("{folder} だよ {duration}", s("done"))).toBe("marimo だよ ");
    expect(fillTemplate("呼んだ？", null)).toBe("呼んだ？");
  });
});

describe("mergeDialogue", () => {
  const defaults = { waiting: ["既定"], done: ["既定の完了"], reaction: ["ん？"] };

  it("overrides only the categories the user wrote", () => {
    expect(mergeDialogue(defaults, { done: ["自分の完了"] })).toEqual({
      waiting: ["既定"],
      done: ["自分の完了"],
      reaction: ["ん？"],
    });
  });

  it("ignores invalid user files and entries", () => {
    expect(mergeDialogue(defaults, null)).toEqual(defaults);
    expect(mergeDialogue(defaults, [1, 2])).toEqual(defaults);
    expect(mergeDialogue(defaults, { done: [], waiting: "x", reaction: [1] })).toEqual(defaults);
  });
});
