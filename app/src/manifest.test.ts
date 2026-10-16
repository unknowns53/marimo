import { describe, expect, it } from "vitest";

import {
  characterCandidates,
  characterInfo,
  DEFAULT_ASPECT,
  DEFAULT_CHARACTER,
  normalize,
  type StandingManifest,
} from "./manifest";
import type { SpriteManifest } from "./renderer";

const asset = (image: string) => ({ image });
const canvas = { width: 100, height: 150 };

describe("normalize", () => {
  it("reads a states-only manifest as expressions named after the statuses, and ignores states once expressions exist", () => {
    const legacy: StandingManifest = {
      mode: "standing",
      canvas,
      states: { idle: asset("i.png"), working: asset("w.png") },
    };
    const old = normalize(legacy);
    expect(old.expressions).toEqual({ idle: asset("i.png"), working: asset("w.png") });
    // 状態の表情が無い状態は rules.status に載せず、描画側の既定へ任せる。
    expect(old.rules.status).toEqual({ idle: "idle", working: "working" });

    const modern = normalize({ ...legacy, expressions: { smile: asset("s.png") } });
    expect(Object.keys(modern.expressions)).toEqual(["smile"]);
    expect(modern.rules.status).toEqual({});
  });

  it("lets rules.status override the same-named expression", () => {
    const c = normalize({
      mode: "standing",
      canvas,
      expressions: { idle: asset("i.png"), happy: asset("h.png") },
      rules: { status: { done: "happy", idle: "happy" } },
    });
    expect(c.rules.status).toEqual({ idle: "happy", done: "happy" });
  });

  it("splits working_tools into exact names and trailing-* prefixes, keeping max_ms per expression", () => {
    const c = normalize({
      mode: "standing",
      canvas,
      expressions: {},
      rules: {
        working_tools: [
          { tools: ["Read", "mcp__github__*"], expression: "read", max_ms: 3000 },
          { tools: ["Bash"], expression: "type" },
        ],
      },
    });
    expect([...c.rules.workingTools]).toEqual([
      ["Read", "read"],
      ["Bash", "type"],
    ]);
    expect(c.rules.workingToolPrefixes).toEqual([["mcp__github__", "read"]]);
    expect([...c.rules.workingMaxMs]).toEqual([["read", 3000]]);
  });
});

describe("characterCandidates", () => {
  it("tries the saved character first when listed and always ends with the first listed one", () => {
    const cases: [string | null, unknown, string[]][] = [
      ["b", ["a", "b"], ["b", "a"]],
      ["a", ["a", "b"], ["a"]],
      // 一覧に無い保存値は捨てて、先頭だけを試す。
      ["gone", ["a", "b"], ["a"]],
      [null, ["a", "b"], ["a"]],
      // 文字列以外の要素は一覧から外す。
      ["b", [1, "a", null, "b"], ["b", "a"]],
      // 一覧が読めないときは既定のキャラクターへ戻す。
      ["b", { not: "array" }, [DEFAULT_CHARACTER]],
      [null, [], [DEFAULT_CHARACTER]],
    ];
    for (const [saved, index, expected] of cases) {
      expect(characterCandidates(saved, index), JSON.stringify([saved, index])).toEqual(expected);
    }
  });
});

describe("characterInfo", () => {
  it("takes the aspect as height over width from canvas or sprite frame, falling back to the default for unusable sizes", () => {
    const standing = (width: number, height: number): StandingManifest => ({
      mode: "standing",
      canvas: { width, height },
    });
    const sprite: SpriteManifest = {
      mode: "sprite",
      sheet: "s.png",
      columns: 1,
      frame: { width: 200, height: 100 },
      states: {},
    };
    expect(characterInfo("a", standing(100, 200)).aspect).toBe(2);
    expect(characterInfo("a", sprite).aspect).toBe(0.5);
    expect(characterInfo("a", standing(0, 100)).aspect).toBe(DEFAULT_ASPECT);
    expect(characterInfo("a", standing(100, -1)).aspect).toBe(DEFAULT_ASPECT);
  });

  it("names the character by display_name, then name, then id, ignoring blank names", () => {
    const base = { mode: "standing", canvas } as const;
    const cases: [Partial<StandingManifest>, string][] = [
      [{ display_name: " Koharu ", name: "k" }, "Koharu"],
      [{ display_name: "   ", name: " k " }, "k"],
      [{ display_name: "", name: "" }, "id1"],
      [{}, "id1"],
    ];
    for (const [extra, expected] of cases) {
      expect(characterInfo("id1", { ...base, ...extra }).displayName).toBe(expected);
    }
  });
});
