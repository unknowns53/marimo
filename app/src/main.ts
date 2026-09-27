import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { Bubble } from "./bubble";
import { renderPanel } from "./panel";
import { createRenderer, loadManifest, type CharacterRenderer } from "./renderer";
import type { Dialogue, Snapshot, Status } from "./types";

const CHARACTER_BASE = new URL("/character/default/", window.location.href).href;
const SPEAKING: ReadonlySet<Status> = new Set(["waiting", "done", "error"]);
const SHOW_ROWS_KEY = "marimo.showRows";
// 利用制限の「古い」「リセット済み」は時間だけで変わるので、変更通知とは別に描き直す。
const PANEL_REFRESH_MS = 30_000;

const $ = (id: string) => document.getElementById(id) as HTMLElement;
const stage = $("stage");
const panel = $("panel");
const rows = $("rows");
const limits = $("limits");
const bubble = new Bubble($("bubble"));

let renderer: CharacterRenderer | undefined;
let snapshot: Snapshot | null = null;
let shown: Status = "idle";
let showRows = readShowRows();

function applySnapshot(next: Snapshot): void {
  snapshot = next;
  renderer?.setStatus(next.aggregate);
  if (next.aggregate !== shown && SPEAKING.has(next.aggregate)) {
    void speak(next.aggregate);
  }
  shown = next.aggregate;
  redrawPanel();
}

// セリフの JSON はユーザーが編集するものなので、話すたびに読み直して再起動なしで反映する。
async function speak(status: Status): Promise<void> {
  try {
    bubble.say(status, await invoke<Dialogue>("get_dialogue"));
  } catch (e) {
    console.error("dialogue", e);
  }
}

function redrawPanel(): void {
  renderPanel(panel, rows, limits, snapshot, showRows, Date.now());
}

function readShowRows(): boolean {
  try {
    return localStorage.getItem(SHOW_ROWS_KEY) !== "false";
  } catch {
    return true;
  }
}

function setShowRows(value: boolean): void {
  showRows = value;
  try {
    localStorage.setItem(SHOW_ROWS_KEY, String(value));
  } catch {
    // 保存できなくても、この起動中の切り替えは効かせる。
  }
  redrawPanel();
}

async function openMenu(): Promise<void> {
  const menu = await Menu.new({
    items: [
      await CheckMenuItem.new({
        text: "セッションの行を表示",
        checked: showRows,
        action: () => setShowRows(!showRows),
      }),
      await PredefinedMenuItem.new({ item: "Separator" }),
      await MenuItem.new({ text: "終了", action: () => void invoke("quit") }),
    ],
  });
  await menu.popup();
}

function bindWindowControls(): void {
  // data-tauri-drag-region はダブルクリックで最大化を切り替えるので使わず、自前で始める。
  document.addEventListener("mousedown", (e) => {
    if (e.button === 0) void getCurrentWindow().startDragging();
  });
  document.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    void openMenu();
  });
}

async function start(): Promise<void> {
  bindWindowControls();
  try {
    const manifest = await loadManifest(CHARACTER_BASE);
    renderer = createRenderer(manifest, CHARACTER_BASE);
    await renderer.mount(stage);
  } catch (e) {
    console.error("character", e);
  }
  await listen<Snapshot>("snapshot", (e) => applySnapshot(e.payload));
  applySnapshot(await invoke<Snapshot>("get_snapshot"));
  window.setInterval(redrawPanel, PANEL_REFRESH_MS);
}

void start();
