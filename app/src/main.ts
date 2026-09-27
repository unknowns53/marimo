import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

import { Bubble } from "./bubble";
import { Acknowledged, triggerKey } from "./acknowledged";
import { BubbleModel } from "./bubbleModel";
import { HitReporter, rectOf, type HitRegions, type Rect } from "./hitArea";
import { renderPanel } from "./panel";
import { planPanel } from "./panelModel";
import { createRenderer, loadManifest, type CharacterRenderer } from "./renderer";
import { nearestPreset, SCALE_PRESETS, ScaleControl } from "./scale";
import { Speech } from "./speech";
import type { Dialogue, SessionState, Snapshot } from "./types";

const CHARACTER_BASE = new URL("/character/default/", window.location.href).href;
const SHOW_ROWS_KEY = "marimo.showRows";
// 利用制限の「古い」「リセット済み」は時間だけで変わるので、変更通知とは別に描き直す。
const PANEL_REFRESH_MS = 30_000;
const REACTION_SPEECH_MS = 2500;
// 押してからこれ以上動いたらドラッグとみなし、立ち絵の反応は出さない。
const DRAG_THRESHOLD_PX = 4;

const $ = (id: string) => document.getElementById(id) as HTMLElement;
const stage = $("stage");
const panelElements = { panel: $("panel"), rows: $("rows"), limits: $("limits") };
const bubbleNode = $("bubble");
const acknowledged = new Acknowledged();
const bubbleModel = new BubbleModel(acknowledged);
// 吹き出しを押して閉じたら、そのきっかけを見たものとして扱い、完了なら行も畳む。
// 完了の吹き出しを閉じるのは知らせを受け取ったという意思表示で、行だけが残っても
// 同じ知らせが二重に場所を取るだけだからである。
const speech = new Speech();
const bubble = new Bubble(bubbleNode, () => {
  if (bubbleModel.view) bubbleModel.dismiss();
  speech.clearReaction();
  showSpeech();
  redrawPanel();
});
const hits = new HitReporter(collectHitRegions);

let renderer: CharacterRenderer | undefined;
let snapshot: Snapshot | null = null;
let dialogue: Dialogue = {};
// 利用者の dialogue.json に無い分類（後から足した reaction など）は、素材フォルダの既定で補う。
let defaultDialogue: Dialogue = {};
let speechTimer: number | undefined;
let showRows = readShowRows();
let scale: ScaleControl | undefined;
// スナップショットは続けて届くことがあり、セリフの読み込みを待つ間に順序が入れ替わらないよう直列にする。
let applying: Promise<void> = Promise.resolve();

function queueSnapshot(next: Snapshot): void {
  applying = applying.then(() => applySnapshot(next)).catch((e) => console.error("snapshot", e));
}

async function applySnapshot(next: Snapshot): Promise<void> {
  snapshot = next;
  acknowledged.prune(next);
  renderer?.update({ status: next.aggregate, tool: focusedTool(next) });
  // セリフの JSON はユーザーが編集するものなので、新しいきっかけのたびに読み直して再起動なしで反映する。
  if (bubbleModel.needsText(next)) await reloadDialogue();
  bubbleModel.update(next, dialogue);
  showSpeech();
  redrawPanel();
}

function focusedTool(s: Snapshot): string | null {
  return s.sessions.find((x) => x.status === s.aggregate)?.activity?.tool ?? null;
}

async function reloadDialogue(): Promise<void> {
  const user = await invoke<Dialogue>("get_dialogue").catch((e) => {
    console.error("dialogue", e);
    return dialogue;
  });
  dialogue = { ...defaultDialogue, ...user };
}

function showSpeech(): void {
  const text = speech.current(bubbleModel.view, performance.now());
  if (text) bubble.show(text);
  else bubble.hide();
  hits.schedule();
}

async function reactToTouch(): Promise<void> {
  renderer?.react();
  if (bubbleModel.view) return;
  await reloadDialogue();
  const lines = dialogue.reaction ?? [];
  const text = lines[Math.floor(Math.random() * lines.length)];
  if (!speech.react(text, performance.now(), REACTION_SPEECH_MS, bubbleModel.view)) return;
  showSpeech();
  window.clearTimeout(speechTimer);
  speechTimer = window.setTimeout(showSpeech, REACTION_SPEECH_MS);
}

function collectHitRegions(): HitRegions {
  const rects = [rectOf(panelElements.panel)];
  // マウスを載せて広げた層はパネルの外へ伸びるので、表示中のものを加える。
  for (const layer of panelElements.panel.querySelectorAll(".hover-layer")) rects.push(rectOf(layer));
  if (bubbleNode.classList.contains("show")) rects.push(rectOf(bubbleNode));
  const area = renderer?.hitArea();
  const box = rectOf(area?.element ?? stage);
  let mask: HitRegions["mask"] = null;
  if (area?.mask && box) mask = { ...box, ...area.mask };
  else rects.push(box);
  return { rects: rects.filter((r): r is Rect => r !== null), mask };
}

function redrawPanel(): void {
  const plan = planPanel(snapshot, acknowledged);
  renderPanel(panelElements, plan, snapshot?.rate_limits ?? null, showRows, Date.now(), selectSession);
  hits.schedule();
}

// 行を押してセッションへ移動したら、そのきっかけを見たものとして扱う。完了の行は畳み、
// 同じきっかけの吹き出しも閉じる。承認待ちとエラーの行は、解決するまで残す。
function selectSession(session: SessionState): void {
  void invoke("focus_session", { sessionId: session.session_id }).catch((e) =>
    console.error("focus", e),
  );
  acknowledged.add(triggerKey(session));
  if (bubbleModel.view?.key === triggerKey(session)) {
    bubbleModel.dismiss();
    bubble.hide();
  }
  redrawPanel();
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
  const marked = scale ? nearestPreset(scale.current) : undefined;
  const sizeItems = await Promise.all(
    SCALE_PRESETS.map((p) =>
      CheckMenuItem.new({
        text: p.label,
        checked: p.scale === marked,
        enabled: scale !== undefined,
        action: () => scale?.set(p.scale),
      }),
    ),
  );
  // ログイン項目の状態はアプリの外（システム設定など）でも変わるので、開くたびに読み直す。
  const autostart = await isEnabled().catch((e) => {
    console.error("autostart", e);
    return undefined;
  });
  const menu = await Menu.new({
    items: [
      await CheckMenuItem.new({
        text: "セッションの行を表示",
        checked: showRows,
        action: () => setShowRows(!showRows),
      }),
      await PredefinedMenuItem.new({ item: "Separator" }),
      ...sizeItems,
      await PredefinedMenuItem.new({ item: "Separator" }),
      await CheckMenuItem.new({
        text: "ログイン時に起動",
        checked: autostart === true,
        enabled: autostart !== undefined,
        action: () => void (autostart ? disable() : enable()).catch((e) => console.error("autostart", e)),
      }),
      await PredefinedMenuItem.new({ item: "Separator" }),
      await MenuItem.new({ text: "終了", action: () => void invoke("quit") }),
    ],
  });
  await menu.popup();
}

function bindWindowControls(): void {
  // data-tauri-drag-region はダブルクリックで最大化を切り替えるので使わず、自前で始める。
  // ドラッグはすぐには始めず、押したまま少し動いてから始める。動かずに離したら、立ち絵を押した
  // ことになる。行と吹き出しは押して操作するので、そこからはドラッグを始めない。
  let press: { x: number; y: number; onStage: boolean; dragging: boolean } | null = null;
  document.addEventListener("mousedown", (e) => {
    const target = e.target as HTMLElement | null;
    if (e.button !== 0 || target?.closest(".row, .working-group, #bubble")) {
      press = null;
      return;
    }
    press = { x: e.screenX, y: e.screenY, onStage: !!target?.closest("#stage"), dragging: false };
  });
  document.addEventListener("mousemove", (e) => {
    if (!press || press.dragging || !(e.buttons & 1)) return;
    if (Math.hypot(e.screenX - press.x, e.screenY - press.y) < DRAG_THRESHOLD_PX) return;
    press.dragging = true;
    void getCurrentWindow().startDragging();
  });
  document.addEventListener("mouseup", () => {
    if (press && !press.dragging && press.onStage) void reactToTouch();
    press = null;
  });
  document.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    void openMenu();
  });
}

async function start(): Promise<void> {
  bindWindowControls();
  try {
    scale = new ScaleControl(await invoke<number>("get_scale"));
    scale.bindWheel(stage);
  } catch (e) {
    console.error("scale", e);
  }
  defaultDialogue = await fetch(new URL("dialogue.json", CHARACTER_BASE))
    .then((r) => (r.ok ? (r.json() as Promise<Dialogue>) : {}))
    .catch(() => ({}));
  dialogue = { ...defaultDialogue };
  try {
    const manifest = await loadManifest(CHARACTER_BASE);
    renderer = createRenderer(manifest, CHARACTER_BASE);
    renderer.onShapeChange = () => hits.schedule();
    await renderer.mount(stage);
  } catch (e) {
    console.error("character", e);
  }
  // マウスが立ち絵の上にあるかは Rust 側がクリックを通す判定のついでに調べて知らせる。透明な部分では
  // 窓がマウスのイベントを受け取らないので、DOM の mouseleave は当てにできない。
  await listen<boolean>("portrait-hover", (e) => renderer?.setHover(e.payload));
  await listen<Snapshot>("snapshot", (e) => queueSnapshot(e.payload));
  queueSnapshot(await invoke<Snapshot>("get_snapshot"));
  window.setInterval(redrawPanel, PANEL_REFRESH_MS);
  // 倍率の変更や行の増減で形が変わったら、クリックを受け取る領域を送り直す。
  const observer = new ResizeObserver(() => hits.schedule());
  for (const el of [stage, panelElements.panel, bubbleNode]) observer.observe(el);
  // 行や作業中の要約にマウスを載せると層が広がるが、ResizeObserver には現れないので別に拾う。
  panelElements.panel.addEventListener("mouseover", () => hits.schedule());
  panelElements.panel.addEventListener("mouseout", () => hits.schedule());
  window.addEventListener("resize", () => hits.schedule());
}

void start();
