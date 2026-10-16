import { describe, expect, it } from "vitest";

import { DEFAULT_BUBBLE_SETTINGS } from "./bubbleSettings";
import { DEFAULT_PANEL_DISPLAY } from "./panelModel";
import {
  bubbleSecondsLabel,
  bubbleSecondsOptions,
  formValues,
  opacityPercent,
  requestFor,
  type FormControl,
} from "./settingsForm";
import type { SettingsRequest, SettingsState } from "./settingsProtocol";

const STATE: SettingsState = {
  display: { ...DEFAULT_PANEL_DISPLAY, panel_style: "counts", row_order: "status", panel_opacity: 0.35000000000000003 },
  bubble: { ...DEFAULT_BUBBLE_SETTINGS, bubble_done: false, bubble_seconds: 30 },
  characters: [
    { id: "koharu", name: "小春" },
    { id: "clawd", name: "Clawd" },
  ],
  character: "clawd",
  scale: 1.5,
  autostart: false,
};

describe("opacityPercent", () => {
  it("rounds to a whole percent and keeps it in range", () => {
    const cases: [number, number][] = [
      [0.8, 80],
      [0.35000000000000003, 35],
      [1, 100],
      [0.05, 20],
      [-1, 20],
      [3, 100],
      [Number.NaN, 80],
      [Number.POSITIVE_INFINITY, 80],
    ];
    for (const [opacity, percent] of cases) {
      expect(opacityPercent(opacity), String(opacity)).toBe(percent);
    }
  });
});

describe("bubble seconds choices", () => {
  it("labels each choice and adds a saved value that is not one of them", () => {
    const cases: [number, string[]][] = [
      [30, ["押すまで", "10 秒", "30 秒", "1 分", "5 分"]],
      [45, ["押すまで", "10 秒", "30 秒", "45 秒", "1 分", "5 分"]],
      [600, ["押すまで", "10 秒", "30 秒", "1 分", "5 分", "10 分"]],
    ];
    for (const [current, labels] of cases) {
      expect(bubbleSecondsOptions(current).map(bubbleSecondsLabel), String(current)).toEqual(labels);
    }
  });
});

describe("formValues", () => {
  it("maps the state onto the form controls by name", () => {
    expect(formValues(STATE)).toEqual({
      show_character: true,
      panel_style: "counts",
      row_order: "status",
      show_cloud_sessions: false,
      panel_opacity: "35",
      bubble_waiting: true,
      bubble_done: false,
      bubble_error: true,
      bubble_seconds: "30",
      character: "clawd",
      scale: "1.5",
      autostart: false,
    });
  });

  it("disables the settings that could not be read", () => {
    const values = formValues({ ...STATE, characters: [], character: null, scale: null, autostart: null });
    expect([values.character, values.scale, values.autostart]).toEqual([null, null, null]);
  });
});

describe("requestFor", () => {
  it("turns a control change into the request for the main window", () => {
    const control = (name: string, value = "on", checked = false): FormControl => ({ name, value, checked });
    const cases: [string, FormControl, "input" | "change", SettingsRequest | null][] = [
      ["checkbox", control("show_character", "on", true), "change", { kind: "display", patch: { show_character: true } }],
      ["checkbox input", control("show_character", "on", true), "input", null],
      ["cloud", control("show_cloud_sessions"), "change", { kind: "display", patch: { show_cloud_sessions: false } }],
      ["style", control("panel_style", "counts", true), "change", { kind: "display", patch: { panel_style: "counts" } }],
      ["unknown style", control("panel_style", "list", true), "change", null],
      ["order", control("row_order", "updated", true), "change", { kind: "display", patch: { row_order: "updated" } }],
      ["opacity drag", control("panel_opacity", "35"), "input", { kind: "opacity-preview", opacity: 0.35 }],
      ["opacity release", control("panel_opacity", "35"), "change", { kind: "display", patch: { panel_opacity: 0.35 } }],
      ["bubble kind", control("bubble_error"), "change", { kind: "bubble", patch: { bubble_error: false } }],
      ["bubble seconds", control("bubble_seconds", "300"), "change", { kind: "bubble", patch: { bubble_seconds: 300 } }],
      ["bad seconds", control("bubble_seconds", "1.5"), "change", null],
      ["character", control("character", "clawd"), "change", { kind: "character", id: "clawd" }],
      ["scale drag", control("scale", "1.7"), "input", { kind: "scale", scale: 1.7 }],
      ["scale release", control("scale", "1.7"), "change", null],
      ["autostart", control("autostart", "on", true), "change", { kind: "autostart", enabled: true }],
      ["unknown control", control("other"), "change", null],
    ];
    for (const [label, c, phase, expected] of cases) {
      expect(requestFor(c, phase), label).toEqual(expected);
    }
  });
});
