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
});
