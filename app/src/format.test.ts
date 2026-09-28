import { describe, expect, it } from "vitest";

import { folderName } from "./format";

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
});
