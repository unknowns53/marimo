import { describe, expect, it } from "vitest";

import type { BubbleView } from "./bubbleModel";
import { Speech } from "./speech";

const notice: BubbleView = { key: "k", sessionKey: "s", status: "done", text: "終わったよ" };

describe("Speech", () => {
  it("shows a reaction until its duration has passed, the end instant being already over", () => {
    const speech = new Speech();
    expect(speech.react("やあ", 1000, 500, null)).toBe(true);
    expect(speech.current(null, 1000)).toBe("やあ");
    expect(speech.current(null, 1499)).toBe("やあ");
    expect(speech.current(null, 1500)).toBeNull();
  });

  it("refuses a reaction while a status bubble is up or the text is empty", () => {
    const speech = new Speech();
    expect(speech.react("やあ", 0, 500, notice)).toBe(false);
    expect(speech.react(undefined, 0, 500, null)).toBe(false);
    expect(speech.react("", 0, 500, null)).toBe(false);
    expect(speech.current(null, 1)).toBeNull();
  });

  it("lets a status bubble win and discard the pending reaction for good", () => {
    const speech = new Speech();
    speech.react("やあ", 0, 1000, null);
    expect(speech.current(notice, 100)).toBe("終わったよ");
    expect(speech.current(null, 200)).toBeNull();
  });

  it("drops the reaction on clearReaction and replaces an earlier one with a later one", () => {
    const speech = new Speech();
    speech.react("one", 0, 1000, null);
    speech.react("two", 10, 1000, null);
    expect(speech.current(null, 20)).toBe("two");
    speech.clearReaction();
    expect(speech.current(null, 30)).toBeNull();
  });
});
