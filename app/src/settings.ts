import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";

import { PANEL_OPACITY_MAX, PANEL_OPACITY_MIN } from "./panelModel";
import { SCALE_MAX, SCALE_MIN } from "./scale";
import {
  bubbleSecondsLabel,
  bubbleSecondsOptions,
  formValues,
  OPACITY_STEP_PERCENT,
  opacityPercent,
  requestFor,
  scaleLabel,
} from "./settingsForm";
import {
  MAIN_WINDOW,
  SETTINGS_REQUEST_EVENT,
  SETTINGS_STATE_EVENT,
  UPDATE_PROGRESS_EVENT,
  type SettingsRequest,
  type SettingsState,
  type UpdateInfo,
  type UpdateProgress,
} from "./settingsProtocol";

const SCALE_STEP = 0.1;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const form = $<HTMLFormElement>("settings");
const opacityInput = $<HTMLInputElement>("panel-opacity");
const scaleInput = $<HTMLInputElement>("scale");
const bubbleSecondsSelect = $<HTMLSelectElement>("bubble-seconds");
const characterSelect = $<HTMLSelectElement>("character");
const versionNode = $("version");
const checkButton = $<HTMLButtonElement>("check-update");
const installButton = $<HTMLButtonElement>("install-update");
const updateStatus = $("update-status");
const confirmDialog = $<HTMLDialogElement>("confirm-update");
const confirmText = $("confirm-update-text");

let state: SettingsState | null = null;
// つまみを動かしている間のスライダー。前の値の反映が遅れて届いても、つまみを引き戻さない。
const dragging = new Set<string>();
let update: UpdateInfo | null = null;

function request(r: SettingsRequest): void {
  void emitTo(MAIN_WINDOW, SETTINGS_REQUEST_EVENT, r).catch((e) => console.error("settings request", e));
}

function render(next: SettingsState): void {
  state = next;
  fillSelect(
    bubbleSecondsSelect,
    bubbleSecondsOptions(next.bubble.bubble_seconds).map((s) => [String(s), bubbleSecondsLabel(s)]),
  );
  fillSelect(
    characterSelect,
    next.characters.map((c) => [c.id, c.name]),
  );
  const values = formValues(next);
  for (const el of form.elements) {
    if (!(el instanceof HTMLInputElement || el instanceof HTMLSelectElement)) continue;
    const value = values[el.name];
    if (value === undefined) continue;
    el.disabled = value === null;
    if (value === null || dragging.has(el.name)) continue;
    if (el instanceof HTMLInputElement && el.type === "checkbox") el.checked = value === true;
    else if (el instanceof HTMLInputElement && el.type === "radio") el.checked = el.value === value;
    else el.value = String(value);
  }
  showRangeValues();
}

// 選択肢を作り直すと開いている一覧が閉じるので、中身が変わったときだけ作り直す。
function fillSelect(select: HTMLSelectElement, options: [string, string][]): void {
  const current = [...select.options].map((o) => [o.value, o.text]);
  if (JSON.stringify(current) === JSON.stringify(options)) return;
  select.replaceChildren(...options.map(([value, text]) => new Option(text, value)));
}

function showRangeValues(): void {
  for (const [input, text] of [
    [opacityInput, `${opacityInput.value}%`],
    [scaleInput, scaleLabel(Number(scaleInput.value))],
  ] as const) {
    const output = form.querySelector(`output[data-for="${input.name}"]`);
    if (output) output.textContent = text;
  }
}

function bindForm(): void {
  opacityInput.min = String(opacityPercent(PANEL_OPACITY_MIN));
  opacityInput.max = String(opacityPercent(PANEL_OPACITY_MAX));
  opacityInput.step = String(OPACITY_STEP_PERCENT);
  scaleInput.min = String(SCALE_MIN);
  scaleInput.max = String(SCALE_MAX);
  scaleInput.step = String(SCALE_STEP);
  form.addEventListener("submit", (e) => e.preventDefault());
  for (const phase of ["input", "change"] as const) {
    form.addEventListener(phase, (e) => {
      const target = e.target;
      if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return;
      if (target.type === "range") showRangeValues();
      const r = requestFor(
        { name: target.name, value: target.value, checked: target instanceof HTMLInputElement && target.checked },
        phase,
      );
      if (r) request(r);
    });
  }
  for (const input of [opacityInput, scaleInput]) {
    input.addEventListener("pointerdown", () => dragging.add(input.name));
  }
  document.addEventListener("pointerup", () => {
    if (dragging.size === 0) return;
    dragging.clear();
    if (state) render(state);
  });
}

function showStatus(text: string, error = false): void {
  updateStatus.textContent = text;
  updateStatus.classList.toggle("error", error);
}

async function checkUpdate(): Promise<void> {
  checkButton.disabled = true;
  installButton.hidden = true;
  update = null;
  showStatus("確認しています");
  try {
    const info = await invoke<UpdateInfo>("check_update");
    versionNode.textContent = info.current;
    update = info.newer ? info : null;
    installButton.hidden = !info.newer;
    showStatus(info.newer ? `新しいバージョン ${info.latest} があります` : "最新のバージョンです");
  } catch (e) {
    showStatus(String(e), true);
  } finally {
    checkButton.disabled = false;
  }
}

// 成功するとアプリが終了して起動し直すので、install_update は戻ってこないことがある。
async function installUpdate(): Promise<void> {
  checkButton.disabled = true;
  installButton.disabled = true;
  showStatus("");
  try {
    await invoke("install_update");
  } catch (e) {
    showStatus(String(e), true);
    checkButton.disabled = false;
    installButton.disabled = false;
  }
}

function bindUpdate(): void {
  checkButton.addEventListener("click", () => void checkUpdate());
  installButton.addEventListener("click", () => {
    if (!update) return;
    confirmText.textContent = `marimo を終了して ${update.latest} に更新します`;
    confirmDialog.returnValue = "";
    confirmDialog.showModal();
  });
  confirmDialog.addEventListener("close", () => {
    if (confirmDialog.returnValue === "ok") void installUpdate();
  });
}

async function start(): Promise<void> {
  bindForm();
  bindUpdate();
  await listen<SettingsState>(SETTINGS_STATE_EVENT, (e) => render(e.payload));
  await listen<UpdateProgress>(UPDATE_PROGRESS_EVENT, (e) => showStatus(e.payload.message));
  // 聞き始めてから求めないと、返ってきた状態を取りこぼす。
  request({ kind: "sync" });
  versionNode.textContent = await getVersion().catch((e) => {
    console.error("version", e);
    return "";
  });
}

void start();
