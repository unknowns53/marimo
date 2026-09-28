import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

import { Bubble } from "./bubble";
import { Acknowledged, triggerKey, withAcknowledged } from "./acknowledged";
import { BubbleModel } from "./bubbleModel";
import { fillTemplate, linesFor, mergeDialogue, reactionCategory } from "./dialogue";
import { HitReporter, hitRegions, rectOf, type HitRegions } from "./hitArea";
import { renderPanel } from "./panel";
import { PanelExpansion } from "./panelExpansion";
import { PANEL_MODES, panelView, showsCharacter, type PanelMode } from "./panelModel";
import { createRenderer, loadManifest, type CharacterRenderer } from "./renderer";
import { nearestPreset, SCALE_PRESETS, ScaleControl } from "./scale";
import { Speech } from "./speech";
import type { Dialogue, SessionState, Snapshot } from "./types";

const CHARACTER_BASE = new URL("/character/default/", window.location.href).href;
// 以前は行を隠す設定だけをこの名前で localStorage に持っていた。段階の保存先を MARIMO_HOME へ
// 移したので、初回だけ読み替えて引き継ぐ。
const LEGACY_SHOW_ROWS_KEY = "marimo.showRows";
// 利用制限の「古い」「リセット済み」と既読の行を畳む時期は時間だけで変わるので、変更通知とは別に描き直す。
const PANEL_REFRESH_MS = 30_000;
const REACTION_SPEECH_MS = 2500;
// 押してからこれ以上動いたらドラッグとみなし、立ち絵の反応は出さない。
const DRAG_THRESHOLD_PX = 4;

const $ = (id: string) => document.getElementById(id) as HTMLElement;
const app = $("app");
const stage = $("stage");
const panelElements = {
  panel: $("panel"),
  rows: $("rows"),
  limits: $("limits"),
  toggle: $("panel-toggle"),
};
const bubbleNode = $("bubble");
const acknowledged = new Acknowledged((keys) =>
  void invoke("set_acknowledged", { keys }).catch((e) => console.error("acknowledged", e)),
);
const bubbleModel = new BubbleModel(acknowledged);
// 吹き出しを押して閉じたら、そのきっかけを見たものとして扱い、完了なら行も既読として薄くしてから畳む。
// 完了の吹き出しを閉じるのは知らせを受け取ったという意思表示だからである。
const speech = new Speech();
const bubble = new Bubble(bubbleNode, () => {
  if (bubbleModel.view) bubbleModel.dismiss();
  speech.clearReaction();
  refreshAcknowledged();
});
const hits = new HitReporter(collectHitRegions);
const expansion = new PanelExpansion(
  panelElements.panel,
  () => hits.flush(),
  () => hits.schedule(),
);

let renderer: CharacterRenderer | undefined;
let snapshot: Snapshot | null = null;
// snapshot を、見たと示された完了を除いて集約し直したもの。表情、吹き出し、パネルはこちらを使う。
let shown: Snapshot | null = null;
let dialogue: Dialogue = {};
// 組み込みの既定のセリフ。利用者の dialogue.json は上書きしたい分類だけを持ち、分類ごとに重ねる。
let defaultDialogue: Dialogue = {};
let speechTimer: number | undefined;
let panelMode: PanelMode = "detail";
let scale: ScaleControl | undefined;
// スナップショットは続けて届くことがあり、セリフの読み込みを待つ間に順序が入れ替わらないよう直列にする。
let applying: Promise<void> = Promise.resolve();

function queueSnapshot(next: Snapshot): void {
  applying = applying.then(() => applySnapshot(next)).catch((e) => console.error("snapshot", e));
}

async function applySnapshot(next: Snapshot): Promise<void> {
  snapshot = next;
  acknowledged.prune(next);
  shown = withAcknowledged(next, acknowledged);
  renderer?.update({ status: shown.aggregate, tool: focusedTool(shown) });
  // セリフの JSON はユーザーが編集するものなので、新しいきっかけのたびに読み直して再起動なしで反映する。
  if (bubbleModel.needsText(shown)) await reloadDialogue();
  bubbleModel.update(shown, dialogue);
  showSpeech();
  redrawPanel();
}

// 見たと示したきっかけが増えたら、集約をやり直して表情、吹き出し、パネルへ反映する。
function refreshAcknowledged(): void {
  if (snapshot) {
    shown = withAcknowledged(snapshot, acknowledged);
    renderer?.update({ status: shown.aggregate, tool: focusedTool(shown) });
    bubbleModel.update(shown, dialogue);
  }
  showSpeech();
  redrawPanel();
}

function focusedTool(s: Snapshot): string | null {
  return s.sessions.find((x) => x.status === s.aggregate)?.activity?.tool ?? null;
}

async function reloadDialogue(): Promise<void> {
  const user = await invoke<unknown>("get_dialogue").catch((e) => {
    console.error("dialogue", e);
    return {};
  });
  dialogue = mergeDialogue(defaultDialogue, user);
}

// リストだけの段階では吹き出しを出さない。知らせの吹き出しはきっかけが続く間 bubbleModel に残るので、
// 段階を戻したときにまだ続いていれば、そのとき出る。
function showSpeech(): void {
  const text = showsCharacter(panelMode) ? speech.current(bubbleModel.view, performance.now()) : null;
  if (text) bubble.show(text);
  else bubble.hide();
  hits.schedule();
}

async function reactToTouch(): Promise<void> {
  renderer?.react();
  if (bubbleModel.view) return;
  await reloadDialogue();
  const aggregate = shown?.aggregate ?? "idle";
  const focus = shown?.sessions.find((s) => s.status === aggregate) ?? null;
  const lines = linesFor(dialogue, reactionCategory(aggregate));
  const template = lines[Math.floor(Math.random() * lines.length)];
  const text = template === undefined ? undefined : fillTemplate(template, focus);
  if (!speech.react(text, performance.now(), REACTION_SPEECH_MS, bubbleModel.view)) return;
  showSpeech();
  window.clearTimeout(speechTimer);
  speechTimer = window.setTimeout(showSpeech, REACTION_SPEECH_MS);
}

function collectHitRegions(): HitRegions {
  // 切り替えのボタンはパネルの上辺の外に付けるので、パネルとは別に加える。
  const rects = [rectOf(panelElements.panel), rectOf(panelElements.toggle)];
  // 広げた層はパネルの外へ伸びるので、表示中のもの（見せる前に測っているものを含む）を加える。
  for (const layer of panelElements.panel.querySelectorAll(".hover-layer")) rects.push(rectOf(layer));
  if (bubbleNode.classList.contains("show")) rects.push(rectOf(bubbleNode));
  if (!showsCharacter(panelMode)) return hitRegions(rects, null);
  const area = renderer?.hitArea();
  return hitRegions(rects, { box: rectOf(area?.element ?? stage), mask: area?.mask ?? null });
}

function redrawPanel(): void {
  const view = panelView(shown, acknowledged, panelMode, Date.now());
  renderPanel(panelElements, view, shown?.rate_limits ?? null, Date.now(), selectSession);
  expansion.evaluate();
  hits.schedule();
}

// 行を押してセッションへ移動したら、そのきっかけを見たものとして扱う。完了の行は既読として薄くしてから畳み、
// 同じきっかけの吹き出しも閉じる。承認待ちとエラーの行は、解決するまで残す。
function selectSession(session: SessionState): void {
  void invoke("focus_session", { sessionId: session.session_id }).catch((e) =>
    console.error("focus", e),
  );
  acknowledged.add(triggerKey(session));
  if (bubbleModel.view?.key === triggerKey(session)) bubbleModel.dismiss();
  refreshAcknowledged();
}

async function loadPanelMode(): Promise<PanelMode> {
  const saved = await invoke<string | null>("get_panel_mode").catch(() => null);
  if (saved && (PANEL_MODES as readonly string[]).includes(saved)) return saved as PanelMode;
  let legacyHidden = false;
  try {
    legacyHidden = localStorage.getItem(LEGACY_SHOW_ROWS_KEY) === "false";
  } catch {
    // 読めなければ引き継ぐものは無いとみなす。
  }
  const mode: PanelMode = legacyHidden ? "picture" : "detail";
  if (legacyHidden) void invoke("set_panel_mode", { mode }).catch(() => {});
  return mode;
}

function setPanelMode(mode: PanelMode): void {
  panelMode = mode;
  void invoke("set_panel_mode", { mode }).catch((e) => console.error("panel mode", e));
  applyCharacterVisibility();
  showSpeech();
  redrawPanel();
}

// 立ち絵は隠している間も描き続け、瞬きや表情の切り替えの時計も止めない。戻したときに今の状態の
// 表情がすぐ出るようにするためである。
function applyCharacterVisibility(): void {
  app.classList.toggle("character-hidden", !showsCharacter(panelMode));
}

const PANEL_MODE_LABEL: Record<PanelMode, string> = {
  detail: "詳細を表示",
  counts: "件数だけ表示",
  list: "リストだけ表示",
  picture: "絵だけ表示",
};

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
  const usageApi = await invoke<boolean>("get_usage_api").catch((e) => {
    console.error("usage api", e);
    return undefined;
  });
  const menu = await Menu.new({
    items: [
      ...(await Promise.all(
        PANEL_MODES.map((mode) =>
          CheckMenuItem.new({
            text: PANEL_MODE_LABEL[mode],
            checked: mode === panelMode,
            action: () => setPanelMode(mode),
          }),
        ),
      )),
      await PredefinedMenuItem.new({ item: "Separator" }),
      ...sizeItems,
      await PredefinedMenuItem.new({ item: "Separator" }),
      await CheckMenuItem.new({
        text: "利用制限を API から取得",
        checked: usageApi === true,
        enabled: usageApi !== undefined,
        action: () => void invoke("set_usage_api", { enabled: !usageApi }).catch((e) => console.error("usage api", e)),
      }),
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

function bindPanelToggle(): void {
  panelElements.toggle.addEventListener("click", (e) => {
    e.stopPropagation();
    const mode = (e.target as HTMLElement | null)?.closest<HTMLElement>("button[data-mode]")?.dataset.mode;
    if ((mode === "detail" || mode === "counts") && mode !== panelMode) setPanelMode(mode);
  });
}

function bindWindowControls(): void {
  // data-tauri-drag-region はダブルクリックで最大化を切り替えるので使わず、自前で始める。
  // ドラッグはすぐには始めず、押したまま少し動いてから始める。動かずに離したら、立ち絵を押した
  // ことになる。行と吹き出しは押して操作するので、そこからはドラッグを始めない。
  let press: { x: number; y: number; onStage: boolean; dragging: boolean } | null = null;
  document.addEventListener("mousedown", (e) => {
    const target = e.target as HTMLElement | null;
    if (e.button !== 0 || target?.closest(".row, .counts-group, #panel-toggle, #bubble")) {
      press = null;
      return;
    }
    const onStage = showsCharacter(panelMode) && !!target?.closest("#stage");
    press = { x: e.screenX, y: e.screenY, onStage, dragging: false };
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
  bindPanelToggle();
  panelMode = await loadPanelMode();
  applyCharacterVisibility();
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
  await listen<{ x: number; y: number } | null>("window-cursor", (e) => expansion.setCursor(e.payload));
  // 最初のスナップショットより先に戻さないと、既読の完了の吹き出しが一度出てしまう。
  acknowledged.restore(
    await invoke<string[]>("get_acknowledged").catch((e) => {
      console.error("acknowledged", e);
      return [];
    }),
  );
  await listen<Snapshot>("snapshot", (e) => queueSnapshot(e.payload));
  queueSnapshot(await invoke<Snapshot>("get_snapshot"));
  window.setInterval(redrawPanel, PANEL_REFRESH_MS);
  // 倍率の変更や行の増減で形が変わったら、クリックを受け取る領域を送り直す。
  const observer = new ResizeObserver(() => hits.schedule());
  for (const el of [stage, panelElements.panel, bubbleNode]) observer.observe(el);
  window.addEventListener("resize", () => hits.schedule());
}

void start();
