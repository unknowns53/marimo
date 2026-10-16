/// <reference types="vite/client" />
import { describe, expect, it } from "vitest";

import stateRs from "../../crates/marimo-core/src/state.rs?raw";
import scaleRs from "../src-tauri/src/scale.rs?raw";
import { STATUS_PRIORITY } from "./acknowledged";
import { MAX_BUBBLE_SECONDS } from "./bubbleSettings";
import { DEFAULT_ASPECT } from "./manifest";
import { DEFAULT_PANEL_OPACITY, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN } from "./panelModel";
import { SCALE_MAX, SCALE_MIN } from "./scale";

// Rust はこれらの値で保存する値を収め直し、フロントエンドは同じ値で操作の範囲を決めて描く。
// 片方だけ変えると表示と保存がずれるので、Rust のソースを読んで値を突き合わせる。
// style.css の寸法は vitest が CSS を空で読むので、scale.rs のテストが照合する。
function rustConst(source: string, name: string): number {
  const m = source.match(new RegExp(`const ${name}: \\w+ = ([\\d.]+);`));
  if (!m) throw new Error(`${name} not found`);
  return Number(m[1]);
}

describe("constants shared with Rust", () => {
  it("match the values in scale.rs", () => {
    const cases: [string, number][] = [
      ["MIN", SCALE_MIN],
      ["MAX", SCALE_MAX],
      ["OPACITY_MIN", PANEL_OPACITY_MIN],
      ["OPACITY_MAX", PANEL_OPACITY_MAX],
      ["DEFAULT_OPACITY", DEFAULT_PANEL_OPACITY],
      ["MAX_BUBBLE_SECONDS", MAX_BUBBLE_SECONDS],
      ["DEFAULT_ASPECT", DEFAULT_ASPECT],
    ];
    for (const [name, value] of cases) expect(value, name).toBe(rustConst(scaleRs, name));
  });

  it("orders statuses as Status::priority does", () => {
    const body = stateRs.slice(stateRs.indexOf("fn priority"));
    const rank = (status: string) => {
      const m = body.match(new RegExp(`Status::${status[0].toUpperCase()}${status.slice(1)} => (\\d+)`));
      if (!m) throw new Error(`${status} not found`);
      return Number(m[1]);
    };
    const fromRust = [...STATUS_PRIORITY].sort((a, b) => rank(b) - rank(a));
    expect(STATUS_PRIORITY).toEqual(fromRust);
  });
});
