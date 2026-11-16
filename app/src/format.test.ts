import { afterEach, describe, expect, it } from "vitest";

import { folderName, formatAge, formatClock, formatUpdated, statusLabel, titleIsName } from "./format";
import { setLang } from "./i18n";

afterEach(() => setLang("ja"));

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

  it("names a folderless chat by its title and a CLI session by its folder", () => {
    const chat = { session_id: "20260101_000000_abcd1234", cwd: null, title: "#general", provider: "hermes" as const };
    expect(folderName(chat)).toBe("#general");
    expect(titleIsName(chat)).toBe(true);
    expect(folderName({ ...chat, title: undefined })).toBe("Hermes");
    const desktop = { ...chat, provider: "antigravity" as const };
    expect([folderName(desktop), titleIsName(desktop)]).toEqual(["#general", true]);
    expect(folderName({ ...desktop, title: undefined })).toBe("Antigravity");
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

  it("uses short English forms", () => {
    setLang("en");
    const now = 10 * 86_400_000;
    const cases: [number, string][] = [
      [59_999, "now"],
      [5 * 60_000, "5m ago"],
      [2 * 3_600_000, "2h ago"],
      [3 * 86_400_000, "3d ago"],
    ];
    for (const [elapsed, want] of cases) expect(formatAge(now - elapsed, now), String(elapsed)).toBe(want);
  });
});

describe("labels", () => {
  it("names statuses and update times in the current language", () => {
    const at = new Date(2026, 0, 1, 14, 5).getTime();
    expect([statusLabel("waiting"), formatUpdated(at)]).toEqual(["承認待ち", `${formatClock(at)} 更新`]);
    setLang("en");
    expect([statusLabel("waiting"), statusLabel("idle"), formatUpdated(at)]).toEqual([
      "Waiting",
      "Idle",
      "Updated 14:05",
    ]);
  });
});
