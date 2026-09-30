import { describe, expect, it } from "vitest";

import { folderName, formatAge, titleIsName } from "./format";

describe("folderName", () => {
  it("prefers the repository, then the cwd folder, then the session id", () => {
    const base = { session_id: "0123456789abcdef", cwd: "/w/marimo-title/app", repo: "marimo" };
    expect(folderName(base)).toBe("marimo");
    expect(folderName({ ...base, repo: undefined })).toBe("app");
    expect(folderName({ ...base, repo: undefined, cwd: "C:\\w\\proj\\" })).toBe("proj");
    expect(folderName({ ...base, repo: "", cwd: null })).toBe("01234567");
  });

  it("uses the title instead of the folders the Codex desktop app makes per chat", () => {
    const scratch = {
      session_id: "0123456789abcdef",
      cwd: "/Users/u/Documents/Codex/2026-09-28/touch-test-txt",
      title: "ファイル作成の確認",
      provider: "codex" as const,
    };
    expect(folderName(scratch)).toBe("ファイル作成の確認");
    expect(folderName({ ...scratch, title: undefined })).toBe("Codex");
    expect(folderName({ ...scratch, cwd: "C:\\Users\\u\\Documents\\Codex\\2026-09-28\\t\\" })).toBe("ファイル作成の確認");
    expect(folderName({ ...scratch, repo: "marimo" })).toBe("marimo");
    expect(folderName({ ...scratch, provider: "claude" })).toBe("touch-test-txt");
    expect(folderName({ ...scratch, cwd: "/Users/u/Documents/Codex/notes" })).toBe("notes");
  });

  it("names a Hermes chat by its channel and a Hermes CLI session by its folder", () => {
    const chat = { session_id: "20260101_000000_abcd1234", cwd: null, title: "#general", provider: "hermes" as const };
    expect(folderName(chat)).toBe("#general");
    expect(titleIsName(chat)).toBe(true);
    expect(folderName({ ...chat, title: undefined })).toBe("Hermes");
    const cli = { ...chat, cwd: "/w/proj", title: undefined };
    expect(folderName(cli)).toBe("proj");
    expect(titleIsName(cli)).toBe(false);
  });
});

describe("formatAge", () => {
  it("counts whole minutes, hours and days since the update", () => {
    const now = 10 * 86_400_000;
    const cases: [number, string][] = [
      [0, "今"],
      [59_999, "今"],
      [60_000, "1分前"],
      [59 * 60_000, "59分前"],
      [3_600_000, "1時間前"],
      [86_400_000 - 1, "23時間前"],
      [3 * 86_400_000, "3日前"],
      [-5_000, "今"],
    ];
    for (const [elapsed, want] of cases) expect(formatAge(now - elapsed, now), String(elapsed)).toBe(want);
  });
});
