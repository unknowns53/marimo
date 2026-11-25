import { afterEach, describe, expect, it } from "vitest";

import { setLang } from "./i18n";
import { limitLine, limitTooltip, RATE_STALE_MS, windowLabel } from "./limits";
import type { CodexRateLimits, RateLimits } from "./types";

afterEach(() => setLang("ja"));

const NOW = 1_800_000_000_000;
const future = NOW / 1000 + 3600;
const past = NOW / 1000 - 1;

const claude = (updatedAt = NOW): RateLimits => ({
  five_hour: { used_percentage: 12.4, resets_at: future },
  seven_day: { used_percentage: 30, resets_at: future },
  updated_at: updatedAt,
});

const codex = (windows: CodexRateLimits["windows"], observedAt = NOW): CodexRateLimits => ({
  windows,
  plan_type: "plus",
  observed_at: observedAt,
  updated_at: NOW,
});

describe("windowLabel", () => {
  it("names whole days, whole hours, minutes, and leaves unknown lengths unnamed", () => {
    const cases: [number | null, string | null][] = [
      [300, "5h"],
      [10080, "7d"],
      [1440, "1d"],
      [60, "1h"],
      [90, "90m"],
      [null, null],
      [0, null],
    ];
    for (const [minutes, label] of cases) expect(windowLabel(minutes), String(minutes)).toBe(label);
  });
});

describe("limitLine", () => {
  it("keeps the Claude-only line unmarked when there is no Codex data", () => {
    expect(limitLine({ claude: claude(NOW - 5), codex: null, antigravity: null }, NOW)).toEqual({
      groups: [
        {
          provider: "claude",
          text: "5h 12% · 7d 30%",
          used: [12.4, 30],
          updatedAt: NOW - 5,
          stale: false,
          resets: [
            { label: "5h", at: future },
            { label: "7d", at: future },
          ],
        },
      ],
      marked: false,
    });
  });

  it("marks each group once Codex data exists, dimming each group on its own", () => {
    const staleCodex = codex(
      [
        { window_minutes: 300, used_percentage: 99, resets_at: past },
        { window_minutes: 10080, used_percentage: 13, resets_at: future },
        { window_minutes: null, used_percentage: 4, resets_at: null },
      ],
      NOW - RATE_STALE_MS - 1,
    );
    const both = limitLine({ claude: claude(), codex: staleCodex, antigravity: null }, NOW);
    expect(both?.marked).toBe(true);
    expect(both?.groups.map((g) => [g.provider, g.text, g.stale])).toEqual([
      ["claude", "5h 12% · 7d 30%", false],
      ["codex", "7d 13% · 4%", true],
    ]);

    // Codex のウィンドウがどれも出せなくても、Codex の値がある以上はマークを付けたまま Claude Code のグループだけを出す。
    const expired = codex([{ window_minutes: 300, used_percentage: 50, resets_at: past }]);
    const onlyClaude = limitLine({ claude: claude(), codex: expired, antigravity: null }, NOW);
    expect([onlyClaude?.marked, onlyClaude?.groups.map((g) => g.provider)]).toEqual([true, ["claude"]]);
    const onlyCodex = limitLine(
      { claude: null, codex: codex([{ window_minutes: 10080, used_percentage: 13, resets_at: null }]), antigravity: null },
      NOW,
    );
    expect(onlyCodex?.groups.map((g) => [g.provider, g.text])).toEqual([["codex", "7d 13%"]]);
    expect(limitLine({ claude: null, codex: expired, antigravity: null }, NOW)).toBeNull();

    // Antigravity は Claude Code と同じ 5h と 7d の名前で、Codex の後ろに並ぶ。Antigravity だけでもマークを付ける。
    const antigravity: RateLimits = {
      five_hour: { used_percentage: 9.9, resets_at: future },
      seven_day: { used_percentage: 3, resets_at: future },
      updated_at: NOW,
    };
    const all = limitLine({ claude: claude(), codex: staleCodex, antigravity }, NOW);
    expect(all?.groups.map((g) => [g.provider, g.text])).toEqual([
      ["claude", "5h 12% · 7d 30%"],
      ["codex", "7d 13% · 4%"],
      ["antigravity", "5h 10% · 7d 3%"],
    ]);
    const onlyAntigravity = limitLine({ claude: null, codex: null, antigravity }, NOW);
    expect([onlyAntigravity?.marked, onlyAntigravity?.groups.map((g) => g.provider)]).toEqual([true, ["antigravity"]]);
  });

  it("includes the labeled values, reset times and update time in the tooltip", () => {
    const now = new Date(2026, 10, 21, 14, 5).getTime();
    const sameDay = Math.floor(new Date(2026, 10, 21, 19, 0).getTime() / 1000);
    const nextDay = Math.floor(new Date(2026, 10, 24, 9, 30).getTime() / 1000);
    const group = {
      provider: "claude" as const,
      text: "5h 12% · 7d 30%",
      used: [12, 30],
      updatedAt: now,
      stale: false,
      resets: [
        { label: "5h", at: sameDay },
        { label: "7d", at: nextDay },
      ],
    };
    expect(limitTooltip(group, "Claude Code", now)).toBe(
      "Claude Code  5h 12% · 7d 30%\n5h リセット 19:00 · 7d リセット 11/24 09:30\n14:05 更新",
    );
    expect(limitTooltip({ ...group, resets: [] }, "Claude Code", now)).toBe("Claude Code  5h 12% · 7d 30%\n14:05 更新");
    setLang("en");
    expect(limitTooltip(group, "Claude Code", now)).toBe(
      "Claude Code  5h 12% · 7d 30%\n5h resets 19:00 · 7d resets Nov 24, 09:30\nUpdated 14:05",
    );
  });
});
