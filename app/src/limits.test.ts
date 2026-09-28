import { describe, expect, it } from "vitest";

import { formatClock } from "./format";
import { limitLine, RATE_STALE_MS, usageNote, windowLabel } from "./limits";
import type { CodexRateLimits, RateLimits, UsageStatus } from "./types";

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
    expect(limitLine(claude(NOW - 5), null, NOW)).toEqual({
      groups: [{ provider: "claude", text: "5h 12% · 7d 30%", updatedAt: NOW - 5, stale: false }],
      marked: false,
      updatedAt: NOW - 5,
    });
  });

  it("marks each group once Codex data exists, dimming and dating each group on its own", () => {
    const staleCodex = codex(
      [
        { window_minutes: 300, used_percentage: 99, resets_at: past },
        { window_minutes: 10080, used_percentage: 13, resets_at: future },
        { window_minutes: null, used_percentage: 4, resets_at: null },
      ],
      NOW - RATE_STALE_MS - 1,
    );
    const both = limitLine(claude(), staleCodex, NOW);
    expect(both?.marked).toBe(true);
    expect(both?.groups.map((g) => [g.provider, g.text, g.stale])).toEqual([
      ["claude", "5h 12% · 7d 30%", false],
      ["codex", "7d 13% · 4%", true],
    ]);
    expect(both?.updatedAt).toBe(NOW);

    // Codex の窓がどれも出せなくても、Codex の値がある以上は印を付けたまま Claude Code の組だけを出す。
    const expired = codex([{ window_minutes: 300, used_percentage: 50, resets_at: past }]);
    const onlyClaude = limitLine(claude(), expired, NOW);
    expect([onlyClaude?.marked, onlyClaude?.groups.map((g) => g.provider)]).toEqual([true, ["claude"]]);
    const onlyCodex = limitLine(null, codex([{ window_minutes: 10080, used_percentage: 13, resets_at: null }]), NOW);
    expect(onlyCodex?.groups.map((g) => [g.provider, g.text])).toEqual([["codex", "7d 13%"]]);
    expect(limitLine(null, expired, NOW)).toBeNull();
  });
});

describe("usageNote", () => {
  it("gives a short reason and an explanatory tooltip only when the API polling is on and failing", () => {
    const at = formatClock(NOW);
    const cases: [UsageStatus | null, string | null, string | null][] = [
      [null, null, null],
      [{ kind: "pending" }, null, null],
      [{ kind: "disabled" }, null, null],
      [{ kind: "ok" }, null, null],
      [{ kind: "token_expired" }, "トークン期限切れ", "ターミナルで claude を一度起動"],
      [{ kind: "missing_scope" }, "トークンの権限不足", "user:profile"],
      [{ kind: "not_found" }, "ログイン情報なし", "ログインすると"],
      [{ kind: "keychain_denied" }, "キーチェーンで拒否", "「利用制限を API から取得」を一度無効に"],
      [{ kind: "rate_limited", retry_at: NOW }, `API 混雑 ${at} に再試行`, `HTTP 429）。${at} に`],
      [{ kind: "rejected", code: 403 }, "トークン拒否", "HTTP 403"],
      [{ kind: "failed", detail: "timeout", retry_at: NOW }, `取得失敗 ${at} に再試行`, `（timeout）。${at} に`],
    ];
    for (const [status, text, hint] of cases) {
      const note = usageNote(status);
      expect(note?.text ?? null, status?.kind).toBe(text);
      if (hint === null) expect(note).toBeNull();
      else expect(note?.title, status?.kind).toContain(hint);
    }
  });
});
