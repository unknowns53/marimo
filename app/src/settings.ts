import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";

import {
  LANGUAGES,
  LANGUAGE_EVENT,
  LANGUAGE_NAMES,
  setLang,
  t,
  translateDocument,
  type LanguageState,
} from "./i18n";
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
  usageRefreshLabel,
  usageRefreshOptions,
} from "./settingsForm";
import {
  MAIN_WINDOW,
  SECTION_EVENT,
  SETTINGS_REQUEST_EVENT,
  SETTINGS_STATE_EVENT,
  UPDATE_PROGRESS_EVENT,
  UPDATE_STATUS_EVENT,
  type SettingsRequest,
  type SettingsState,
  type UpdateProgress,
  type UpdateStatus,
} from "./settingsProtocol";

const SCALE_STEP = 0.1;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const form = $<HTMLFormElement>("settings");
const opacityInput = $<HTMLInputElement>("panel-opacity");
const scaleInput = $<HTMLInputElement>("scale");
const bubbleSecondsSelect = $<HTMLSelectElement>("bubble-seconds");
const characterSelect = $<HTMLSelectElement>("character-select");
const languageSelect = $<HTMLSelectElement>("language");
const usageRefreshSelect = $<HTMLSelectElement>("usage-refresh");
const sectionButtons = [...document.querySelectorAll<HTMLButtonElement>("#sections [data-section]")];
const versionNode = $("version");
const latestNode = $("latest");
const checkedAtNode = $("checked-at");
const checkedAtLabel = $("checked-at-label");
const updateSummary = $("update-summary");
const checkButton = $<HTMLButtonElement>("check-update");
const installButton = $<HTMLButtonElement>("install-update");
const updateStatus = $("update-status");
const confirmDialog = $<HTMLDialogElement>("confirm-update");
const confirmTitle = $("confirm-update-title");

let state: SettingsState | null = null;
// つまみを動かしている間のスライダー。前の値の反映が遅れて届いても、つまみを引き戻さない。
const dragging = new Set<string>();
let update: UpdateStatus | null = null;
let installing = false;

function request(r: SettingsRequest): void {
  void emitTo(MAIN_WINDOW, SETTINGS_REQUEST_EVENT, r).catch((e) => console.error("settings request", e));
}

function render(next: SettingsState): void {
  state = next;
  fillSelect(languageSelect, LANGUAGES.map((l) => [l, LANGUAGE_NAMES[l]]));
  fillSelect(
    bubbleSecondsSelect,
    bubbleSecondsOptions(next.bubble.bubble_seconds).map((s) => [String(s), bubbleSecondsLabel(s)]),
  );
  fillSelect(
    characterSelect,
    next.characters.map((c) => [c.id, c.name]),
  );
  fillSelect(
    usageRefreshSelect,
    usageRefreshOptions(next.usage_refresh_minutes).map((m) => [String(m), usageRefreshLabel(m)]),
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

// 選択肢を作り直すと開いている一覧が閉じるので、内容が変わったときだけ作り直す。
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

function showSection(id: string): void {
  if (!sectionButtons.some((b) => b.dataset.section === id)) id = sectionButtons[0].dataset.section ?? "";
  for (const button of sectionButtons) {
    const current = button.dataset.section === id;
    button.setAttribute("aria-current", String(current));
    $(`section-${button.dataset.section ?? ""}`).hidden = !current;
  }
}

function bindSections(): void {
  for (const button of sectionButtons) {
    button.addEventListener("click", () => showSection(button.dataset.section ?? ""));
  }
  // トレイの「X に更新できます」から開いたときは settings.html#update で開く。
  showSection(location.hash.slice(1));
}

function showStatus(text: string, error = false): void {
  updateStatus.textContent = text;
  updateStatus.classList.toggle("error", error);
}

const EN_MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function clock(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  return t(`${d.getMonth() + 1}/${d.getDate()} ${time}`, `${EN_MONTHS[d.getMonth()]} ${d.getDate()}, ${time}`);
}

function renderUpdate(status: UpdateStatus): void {
  update = status;
  versionNode.textContent = status.current;
  latestNode.textContent =
    status.latest ?? (status.error ? t("不明", "Unknown") : t("確認していません", "Not checked yet"));
  checkedAtNode.textContent = status.checked_at === null ? "" : clock(status.checked_at);
  checkedAtNode.hidden = checkedAtLabel.hidden = status.checked_at === null;
  const target = status.newer ? status.latest : null;
  installButton.hidden = target === null;
  installButton.textContent = target ? t(`${target} に更新`, `Update to ${target}`) : "";
  updateSummary.textContent = target
    ? t(`${target} に更新できます`, `Update Available: ${target}`)
    : t("現在のバージョンと確認", "Current version and checks");
  updateSummary.classList.toggle("note", target !== null);
  if (!installing) showStatus(status.error ?? "", status.error !== null);
}

async function checkUpdate(): Promise<void> {
  checkButton.disabled = true;
  checkButton.textContent = t("確認しています…", "Checking…");
  try {
    renderUpdate(await invoke<UpdateStatus>("check_update"));
  } catch (e) {
    showStatus(String(e), true);
  } finally {
    checkButton.disabled = false;
    checkButton.textContent = t("更新を確認", "Check for Updates");
  }
}

// 成功するとアプリが終了して起動し直すので、install_update は戻ってこないことがある。
async function installUpdate(version: string): Promise<void> {
  installing = true;
  checkButton.disabled = true;
  installButton.disabled = true;
  showStatus("");
  try {
    await invoke("install_update", { version });
  } catch (e) {
    showStatus(String(e), true);
    checkButton.disabled = false;
    installButton.disabled = false;
    installing = false;
  }
}

function bindUpdate(): void {
  checkButton.addEventListener("click", () => void checkUpdate());
  installButton.addEventListener("click", () => {
    if (!update?.newer || !update.latest) return;
    confirmTitle.textContent = t(`${update.latest} に更新`, `Update to ${update.latest}`);
    confirmDialog.dataset.version = update.latest;
    confirmDialog.returnValue = "";
    confirmDialog.showModal();
  });
  confirmDialog.addEventListener("close", () => {
    const version = confirmDialog.dataset.version;
    if (confirmDialog.returnValue === "ok" && version) void installUpdate(version);
  });
}

// 地の文の日本語は最初の translateDocument で data-ja に写すので、コードが文言を書き換える前に一度呼ぶ。
function applyLanguage(language: LanguageState): void {
  setLang(language.lang);
  translateDocument();
  if (state) render(state);
  if (update) renderUpdate(update);
}

async function start(): Promise<void> {
  await listen<LanguageState>(LANGUAGE_EVENT, (e) => applyLanguage(e.payload));
  applyLanguage(await invoke<LanguageState>("get_language").catch((): LanguageState => ({ setting: "auto", lang: "ja" })));
  bindSections();
  bindForm();
  bindUpdate();
  await listen<SettingsState>(SETTINGS_STATE_EVENT, (e) => render(e.payload));
  await listen<UpdateProgress>(UPDATE_PROGRESS_EVENT, (e) => showStatus(e.payload.message));
  await listen<UpdateStatus>(UPDATE_STATUS_EVENT, (e) => renderUpdate(e.payload));
  await listen<string>(SECTION_EVENT, (e) => showSection(e.payload));
  // 聞き始めてから求めないと、返ってきた状態を取りこぼす。
  request({ kind: "sync" });
  renderUpdate(await invoke<UpdateStatus>("update_status"));
}

void start();
