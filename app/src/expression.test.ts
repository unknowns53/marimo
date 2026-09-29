import { describe, expect, it } from "vitest";

import { ExpressionDirector } from "./expression";
import { characterCandidates, characterInfo, normalize, type StandingManifest } from "./manifest";

const FULL: StandingManifest = {
  mode: "standing",
  canvas: { width: 800, height: 1200 },
  expressions: Object.fromEntries(
    [
      "idle",
      "working",
      "working_focus",
      "working_curious",
      "working_think",
      "working_delegate",
      "waiting",
      "done",
      "error",
      "idle_look_away",
      "idle_hair",
      "react_shy",
    ].map((n) => [n, { image: `${n}.png` }]),
  ),
  rules: {
    status: { idle: "idle", working: "working", waiting: "waiting", done: "done", error: "error" },
    working_tools: [
      { tools: ["Bash", "Edit"], expression: "working_focus" },
      { tools: ["WebSearch", "WebFetch", "mcp__*"], expression: "working_curious" },
      { tools: ["Agent"], expression: "working_delegate", max_ms: 30000 },
    ],
    working_no_tool: "working_think",
    min_switch_ms: 4000,
    idle_gestures: {
      interval_ms: [20000, 60000],
      gestures: [
        { expression: "idle_look_away", duration_ms: [3000, 5000] },
        { expression: "idle_hair", duration_ms: [3000, 5000] },
      ],
    },
    reaction: { expression: "react_shy", duration_ms: 2500 },
    variants: { "done.long": "idle_hair", "error.rate_limit": "idle_yawn" },
    hover: { working: "idle" },
    hover_release_ms: 600,
  },
};

function director(missing: string[] = [], random = () => 0) {
  const character = normalize(FULL);
  const available = (n: string) => n in character.expressions && !missing.includes(n);
  return new ExpressionDirector(character.rules, available, random);
}

describe("ExpressionDirector", () => {
  it("picks the working expression from the tool", () => {
    const d = director();
    d.setInput({ status: "working", tool: "Bash" });
    expect(d.current(0)).toBe("working_focus");
    const e = director();
    e.setInput({ status: "working", tool: "WebFetch" });
    expect(e.current(0)).toBe("working_curious");
    const m = director();
    m.setInput({ status: "working", tool: "mcp__mashu__memory_list" });
    expect(m.current(0)).toBe("working_curious");
    const f = director();
    f.setInput({ status: "working", tool: "Read" });
    expect(f.current(0)).toBe("working");
    const g = director();
    g.setInput({ status: "working", tool: null });
    expect(g.current(0)).toBe("working_think");
    const h = director(["working_think"]);
    h.setInput({ status: "working", tool: null });
    expect(h.current(0)).toBe("working");
  });

  it("uses the variant for a dialogue category and falls back when it has no image", () => {
    // idle_yawn は FULL に無いので、error.rate_limit は状態の基本の表情になる。
    for (const [status, category, expected] of [
      ["done", "done.long", "idle_hair"],
      ["done", "done.short", "done"],
      ["done", null, "done"],
      ["error", "error.rate_limit", "error"],
    ] as const) {
      const d = director();
      d.setInput({ status, tool: null, category });
      expect(d.current(0), `${status} ${category}`).toBe(expected);
    }
  });

  it("does not switch working expressions more often than the minimum interval", () => {
    const d = director();
    d.setInput({ status: "working", tool: "Bash" });
    expect(d.current(0)).toBe("working_focus");
    d.setInput({ status: "working", tool: "WebSearch" });
    expect(d.current(1000)).toBe("working_focus");
    d.setInput({ status: "working", tool: "Read" });
    expect(d.current(3999)).toBe("working_focus");
    expect(d.current(4000)).toBe("working");
    // 状態が変わったときは下限を待たずに切り替える。
    d.setInput({ status: "done", tool: null });
    expect(d.current(4100)).toBe("done");
  });

  it("returns to the base working expression after a tool's time limit", () => {
    const d = director();
    d.setInput({ status: "working", tool: "Agent" });
    expect(d.current(0)).toBe("working_delegate");
    expect(d.current(29999)).toBe("working_delegate");
    expect(d.current(30000)).toBe("working");
    // 別のツールを挟むか状態が変われば、上限は数え直す。
    d.setInput({ status: "working", tool: "Bash" });
    expect(d.current(35000)).toBe("working_focus");
    d.setInput({ status: "working", tool: "Agent" });
    expect(d.current(40000)).toBe("working_delegate");
    d.setInput({ status: "waiting", tool: "Agent" });
    d.setInput({ status: "working", tool: "Agent" });
    expect(d.current(80000)).toBe("working_delegate");
  });

  it("starts and ends idle gestures within the configured ranges", () => {
    let r = 0;
    const d = director([], () => r);
    d.setInput({ status: "idle", tool: null });
    expect(d.current(0)).toBe("idle"); // 次の仕草は 20 秒後に決まる
    expect(d.current(19_999)).toBe("idle");
    expect(d.current(20_000)).toBe("idle_look_away");
    expect(d.current(22_999)).toBe("idle_look_away");
    r = 0.99;
    expect(d.current(23_000)).toBe("idle"); // 3 秒で戻り、次は約 60 秒後
    expect(d.current(82_000)).toBe("idle");
    expect(d.current(83_000)).toBe("idle_hair");
    // 待機から離れたら、仕草の途中でも止める。
    d.setInput({ status: "working", tool: "Bash" });
    expect(d.current(83_001)).toBe("working_focus");
  });

  it("does not repeat the same gesture twice in a row", () => {
    const d = director([], () => 0);
    d.setInput({ status: "idle", tool: null });
    d.current(0);
    expect(d.current(20_000)).toBe("idle_look_away");
    expect(d.current(23_000)).toBe("idle");
    // 乱数が同じでも、直前の仕草を除いた候補から選ぶ。
    expect(d.current(43_000)).toBe("idle_hair");
    expect(d.current(46_000)).toBe("idle");
    expect(d.current(66_000)).toBe("idle_look_away");
  });

  it("reaction and hover override the base expression for their durations", () => {
    const d = director();
    d.setInput({ status: "working", tool: "Bash" });
    d.setHover(true, 0);
    expect(d.current(0)).toBe("idle");
    expect(d.react(100)).toBe(true);
    expect(d.current(200)).toBe("react_shy");
    expect(d.current(2599)).toBe("react_shy");
    expect(d.current(2600)).toBe("idle");

    // カーソルが離れても、hover_release_ms の間はマウスを載せたときの表情を保つ。
    const e = director();
    e.setInput({ status: "working", tool: "Bash" });
    e.setHover(true, 0);
    expect(e.current(0)).toBe("idle");
    e.setHover(false, 1000);
    expect(e.current(1500)).toBe("idle");
    expect(e.current(1600)).toBe("working_focus");
  });

  it("falls back to the status expression when assets are missing", () => {
    const d = director(["working_focus", "react_shy", "idle_look_away", "idle_hair"]);
    d.setInput({ status: "working", tool: "Bash" });
    expect(d.current(0)).toBe("working");
    expect(d.react(0)).toBe(false);
    const idle = director(["idle_look_away", "idle_hair"]);
    idle.setInput({ status: "idle", tool: null });
    idle.current(0);
    expect(idle.current(100_000)).toBe("idle");
    const bare = director(["waiting"]);
    bare.setInput({ status: "waiting", tool: null });
    expect(bare.current(0)).toBe("idle");
  });
});

describe("normalize", () => {
  it("reads the legacy states-only manifest", () => {
    const c = normalize({
      mode: "standing",
      canvas: { width: 800, height: 1200 },
      states: { idle: { image: "idle.png", blink: "idle_blink.png" }, done: { image: "done.png" } },
    });
    expect(Object.keys(c.expressions)).toEqual(["idle", "done"]);
    expect(c.rules.status).toEqual({ idle: "idle", done: "done" });
    expect(c.rules.idleGestures).toBeNull();
    expect(c.rules.reaction).toBeNull();
    expect(c.rules.minSwitchMs).toBe(4000);
    const d = new ExpressionDirector(c.rules, (n) => n in c.expressions);
    d.setInput({ status: "working", tool: "Bash" });
    expect(d.current(0)).toBe("idle");
  });
});

describe("built-in characters", () => {
  it("reads the menu name, pixel art flag and stage aspect from the manifest", () => {
    const cases: [string, Partial<StandingManifest>, [string, boolean, number]][] = [
      ["display name wins", { name: "koharu", display_name: "小春" }, ["小春", false, 1.5]],
      [
        "falls back to name",
        { name: "clawd", pixelated: true, canvas: { width: 60, height: 30 } },
        ["clawd", true, 0.5],
      ],
      ["falls back to the id", { display_name: " ", pixelated: "yes" as never }, ["x", false, 1.5]],
      ["broken canvas keeps the default aspect", { canvas: { width: 0, height: 30 } }, ["x", false, 1.5]],
    ];
    for (const [label, fields, [name, pixelated, aspect]] of cases) {
      const info = characterInfo("x", { ...FULL, ...fields });
      expect([info.displayName, info.pixelated, info.aspect], label).toEqual([name, pixelated, aspect]);
    }
  });

  it("tries the saved character first and the first listed one as the fallback", () => {
    const index = ["koharu", "clawd"];
    const cases: [string | null, unknown, string[]][] = [
      ["clawd", index, ["clawd", "koharu"]],
      ["koharu", index, ["koharu"]],
      ["removed", index, ["koharu"]],
      [null, index, ["koharu"]],
      ["clawd", "broken", ["koharu"]],
      ["clawd", ["clawd", 3], ["clawd"]],
    ];
    for (const [saved, idx, expected] of cases) {
      expect(characterCandidates(saved, idx), `${saved} ${JSON.stringify(idx)}`).toEqual(expected);
    }
  });
});
