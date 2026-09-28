import { describe, expect, it } from "vitest";

import { ExpressionDirector } from "./expression";
import { normalize, type StandingManifest } from "./manifest";

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
      { tools: ["WebSearch", "WebFetch"], expression: "working_curious" },
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
