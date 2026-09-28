import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

import { Bubble } from "./bubble";
import { Acknowledged, triggerKey, withAcknowledged } from "./acknowledged";
import { BubbleModel } from "./bubbleModel";
import { fillTemplate, linesFor, mergeDialogue, reactionCategory } from "./dialogue";
import { HitReporter, hitRegions, rectOf, type HitRegions, type Rect } from "./hitArea";
import { characterCandidates, characterInfo, DEFAULT_CHARACTER, type CharacterInfo } from "./manifest";
import { onScrollbar, renderPanel } from "./panel";
import { PanelExpansion } from "./panelExpansion";
import {
  DEFAULT_PANEL_DISPLAY,
  PANEL_STYLES,
  panelView,
  ROW_ORDERS,
  type PanelDisplay,
  type PanelStyle,
  type RowOrder,
} from "./panelModel";
import { createRenderer, loadManifest, type CharacterRenderer, type Manifest } from "./renderer";
import { nearestPreset, SCALE_PRESETS, ScaleControl } from "./scale";
import { Speech } from "./speech";
import type { AppIcons, Dialogue, SessionState, Snapshot, UsageStatus } from "./types";

const CHARACTER_ROOT = new URL("/character/", window.location.href).href;
// 以前は行を隠す設定だけをこの名前で localStorage に持っていた。表示の保存先を MARIMO_HOME へ
// 移したので、display.json に何も無いときだけ読み替えて引き継ぐ。
const LEGACY_SHOW_ROWS_KEY = "marimo.showRows";
// 利用制限の「古い」「リセット済み」と既読の行を畳む時期は時間だけで変わるので、変更通知とは別に描き直す。
const PANEL_REFRESH_MS = 30_000;
const REACTION_SPEECH_MS = 2500;
// style.css の #bubble の bottom に足している、立ち絵と吹き出しの間の隙間と揃える必要がある。
const BUBBLE_GAP_PX = 2;
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
  order: $("row-order"),
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
// manifest を読めなかったキャラクターは選んでも出せないので、読めたものだけをメニューに並べる。
let characters: { info: CharacterInfo; manifest: Manifest }[] = [];
let characterId: string | undefined;
// 立ち絵の画像を読む間に別のキャラクターが選ばれることがあるので、最後に頼んだものだけを出す。
let requestedCharacter: string | undefined;
let characterRequest = 0;
let snapshot: Snapshot | null = null;
// snapshot を、見たと示された完了を除いて集約し直したもの。表情、吹き出し、パネルはこちらを使う。
let shown: Snapshot | null = null;
let dialogue: Dialogue = {};
// 組み込みの既定のセリフ。利用者の dialogue.json は上書きしたい分類だけを持ち、分類ごとに重ねる。
let defaultDialogue: Dialogue = {};
let speechTimer: number | undefined;
let panelDisplay: PanelDisplay = DEFAULT_PANEL_DISPLAY;
let appIcons: AppIcons = { claude: null, codex: null };
let usageStatus: UsageStatus | null = null;
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

// 立ち絵を隠している間は吹き出しを出さない。知らせの吹き出しはきっかけが続く間 bubbleModel に残るので、
// 立ち絵を戻したときにまだ続いていれば、そのとき出る。
function showSpeech(): void {
  const text = panelDisplay.show_character ? speech.current(bubbleModel.view, performance.now()) : null;
  if (text) bubble.show(text);
  else bubble.hide();
  placeBubble();
  hits.schedule();
}

// 背の低い立ち絵では、頭の上に出した吹き出しが左のパネルに重なることがある。重なるときだけ
// パネルの上端より上へ持ち上げ、窓の上端からははみ出させない。吹き出しには動きの transform が
// 掛かるので、大きさは transform の影響を受けない offset の値で測る。
function placeBubble(): void {
  const box = stage.getBoundingClientRect();
  const bottom = box.top - BUBBLE_GAP_PX;
  const left = box.right - bubbleNode.offsetWidth;
  const tops = [rectOf(panelElements.panel), rectOf(panelElements.toggle), rectOf(panelElements.order)]
    .filter((r): r is Rect => r !== null && r.x + r.w > left)
    .map((r) => r.y - BUBBLE_GAP_PX);
  const lift = Math.max(0, Math.min(bottom - Math.min(bottom, ...tops), bottom - bubbleNode.offsetHeight));
  bubbleNode.style.setProperty("--bubble-lift", `${lift}px`);
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
  // 切り替えと並べ方のボタンはパネルの上辺の外に付けるので、パネルとは別に加える。
  const rects = [rectOf(panelElements.panel), rectOf(panelElements.toggle), rectOf(panelElements.order)];
  // 広げた層はパネルの外へ伸びるので、表示中のもの（見せる前に測っているものを含む）を加える。
  for (const layer of panelElements.panel.querySelectorAll(".hover-layer")) rects.push(rectOf(layer));
  if (bubbleNode.classList.contains("show")) rects.push(rectOf(bubbleNode));
  if (!panelDisplay.show_character) return hitRegions(rects, null);
  const area = renderer?.hitArea();
  return hitRegions(rects, { box: rectOf(area?.element ?? stage), mask: area?.mask ?? null });
}

function redrawPanel(): void {
  const view = panelView(shown, acknowledged, panelDisplay.panel_style, panelDisplay.row_order, Date.now());
  const limits = { claude: shown?.rate_limits ?? null, codex: shown?.codex_rate_limits ?? null, usage: usageStatus };
  renderPanel(panelElements, view, limits, appIcons, Date.now(), selectSession);
  placeBubble();
  expansion.evaluate();
  hits.schedule();
}

// 行を押してセッションへ移動したら、そのきっかけを見たものとして扱う。完了の行は既読として薄くしてから畳み、
// 同じきっかけの吹き出しも閉じる。承認待ちとエラーの行は、解決するまで残す。
function selectSession(session: SessionState): void {
  void invoke("focus_session", { provider: session.provider ?? "claude", sessionId: session.session_id }).catch(
    (e) => console.error("focus", e),
  );
  acknowledged.add(triggerKey(session));
  if (bubbleModel.view?.key === triggerKey(session)) bubbleModel.dismiss();
  refreshAcknowledged();
}

function characterBase(id: string): string {
  return new URL(`${id}/`, CHARACTER_ROOT).href;
}

async function loadCharacters(): Promise<unknown> {
  const index = await fetch(new URL("index.json", CHARACTER_ROOT))
    .then((r) => (r.ok ? (r.json() as Promise<unknown>) : []))
    .catch(() => []);
  const listed = Array.isArray(index) ? index.filter((x): x is string => typeof x === "string") : [];
  // 一覧を読めなくても、characterCandidates が選ぶ既定のキャラクターの素材は無事なことがある。
  const ids = listed.length > 0 ? listed : [DEFAULT_CHARACTER];
  const loaded = await Promise.all(
    ids.map((id) =>
      loadManifest(characterBase(id)).then(
        (manifest) => ({ info: characterInfo(id, manifest), manifest }),
        (e) => {
          console.error("character", id, e);
          return null;
        },
      ),
    ),
  );
  characters = loaded.filter((c) => c !== null);
  return index;
}

// 新しい立ち絵を読み終えてから古いものと入れ替えるので、読めなかったときは今の立ち絵が残る。
// 読んでいる間に次の依頼が来ていたら、読み終えたものを捨てて何も変えない。
// 枠の高さは素材の縦横比で決まり、窓の大きさも合わせるよう Rust に知らせる。
async function showCharacter(id: string): Promise<"shown" | "failed" | "superseded"> {
  const request = ++characterRequest;
  requestedCharacter = id;
  const superseded = () => request !== characterRequest;
  const entry = characters.find((c) => c.info.id === id);
  const base = characterBase(id);
  let next: CharacterRenderer | undefined;
  try {
    if (!entry) throw new Error("not in the character index");
    next = createRenderer(entry.manifest, base);
    next.onShapeChange = () => hits.schedule();
    await next.mount(stage);
  } catch (e) {
    console.error("character", id, e);
    next?.destroy();
    if (superseded()) return "superseded";
    requestedCharacter = characterId;
    return "failed";
  }
  const nextDialogue = await fetch(new URL("dialogue.json", base))
    .then((r) => (r.ok ? (r.json() as Promise<Dialogue>) : {}))
    .catch(() => ({}));
  if (superseded()) {
    next.destroy();
    return "superseded";
  }
  document.documentElement.style.setProperty("--stage-aspect", String(entry.info.aspect));
  stage.classList.toggle("pixelated", entry.info.pixelated);
  void invoke("set_stage_aspect", { aspect: entry.info.aspect }).catch((e) => console.error("stage aspect", e));
  renderer?.destroy();
  renderer = next;
  characterId = id;
  defaultDialogue = nextDialogue;
  if (shown) renderer.update({ status: shown.aggregate, tool: focusedTool(shown) });
  await reloadDialogue();
  hits.schedule();
  return "shown";
}

async function switchCharacter(id: string): Promise<void> {
  if (id === requestedCharacter || (await showCharacter(id)) !== "shown") return;
  void invoke("set_character", { id }).catch((e) => console.error("character", e));
}

async function loadPanelDisplay(): Promise<PanelDisplay> {
  const saved = await invoke<PanelDisplay | null>("get_panel_display").catch(() => null);
  if (saved) return saved;
  let legacyHidden = false;
  try {
    legacyHidden = localStorage.getItem(LEGACY_SHOW_ROWS_KEY) === "false";
  } catch {
    // 読めなければ引き継ぐものは無いとみなす。
  }
  if (!legacyHidden) return DEFAULT_PANEL_DISPLAY;
  // 行を隠す設定は、行を出さない表示が無くなったので、立ち絵を残して最も場所を取らない件数だけへ読み替える。
  const display: PanelDisplay = { ...DEFAULT_PANEL_DISPLAY, panel_style: "counts" };
  void invoke("set_panel_display", { display }).catch(() => {});
  return display;
}

function applyPanelDisplay(display: PanelDisplay): void {
  panelDisplay = display;
  applyCharacterVisibility();
  showSpeech();
  redrawPanel();
}

function setPanelDisplay(display: PanelDisplay): void {
  applyPanelDisplay(display);
  void invoke("set_panel_display", { display }).catch((e) => console.error("panel display", e));
}

function setShowCharacter(show: boolean): void {
  setPanelDisplay({ ...panelDisplay, show_character: show });
}

function setPanelStyle(style: PanelStyle): void {
  setPanelDisplay({ ...panelDisplay, panel_style: style });
}

function setRowOrder(order: RowOrder): void {
  setPanelDisplay({ ...panelDisplay, row_order: order });
}

// 立ち絵は隠している間も描き続け、瞬きや表情の切り替えの時計も止めない。戻したときに今の状態の
// 表情がすぐ出るようにするためである。
function applyCharacterVisibility(): void {
  app.classList.toggle("character-hidden", !panelDisplay.show_character);
}

const PANEL_STYLE_LABEL: Record<PanelStyle, string> = {
  detail: "詳細を表示",
  counts: "件数だけ表示",
};

const ROW_ORDER_LABEL: Record<RowOrder, string> = {
  started: "始まった順に並べる",
  status: "状態の順に並べる",
  updated: "更新の新しい順に並べる",
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
  const characterItems = await Promise.all(
    characters.map(({ info }) =>
      CheckMenuItem.new({
        text: info.displayName,
        checked: info.id === characterId,
        action: () => void switchCharacter(info.id),
      }),
    ),
  );
  const menu = await Menu.new({
    items: [
      await CheckMenuItem.new({
        text: "絵を表示",
        checked: panelDisplay.show_character,
        action: () => setShowCharacter(!panelDisplay.show_character),
      }),
      ...(await Promise.all(
        PANEL_STYLES.map((style) =>
          CheckMenuItem.new({
            text: PANEL_STYLE_LABEL[style],
            checked: style === panelDisplay.panel_style,
            action: () => setPanelStyle(style),
          }),
        ),
      )),
      await PredefinedMenuItem.new({ item: "Separator" }),
      ...(await Promise.all(
        ROW_ORDERS.map((order) =>
          CheckMenuItem.new({
            text: ROW_ORDER_LABEL[order],
            checked: order === panelDisplay.row_order,
            action: () => setRowOrder(order),
          }),
        ),
      )),
      await PredefinedMenuItem.new({ item: "Separator" }),
      ...sizeItems,
      await PredefinedMenuItem.new({ item: "Separator" }),
      await Submenu.new({ text: "キャラクター", enabled: characterItems.length > 0, items: characterItems }),
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

function bindPanelTabs(): void {
  const pressed = (e: MouseEvent, key: string) =>
    (e.target as HTMLElement | null)?.closest<HTMLElement>(`button[data-${key}]`)?.dataset[key];
  panelElements.toggle.addEventListener("click", (e) => {
    e.stopPropagation();
    const style = PANEL_STYLES.find((s) => s === pressed(e, "style"));
    if (style && style !== panelDisplay.panel_style) setPanelStyle(style);
  });
  panelElements.order.addEventListener("click", (e) => {
    e.stopPropagation();
    const order = ROW_ORDERS.find((o) => o === pressed(e, "order"));
    if (order && order !== panelDisplay.row_order) setRowOrder(order);
  });
}

function bindWindowControls(): void {
  // data-tauri-drag-region はダブルクリックで最大化を切り替えるので使わず、自前で始める。
  // ドラッグはすぐには始めず、押したまま少し動いてから始める。動かずに離したら、立ち絵を押した
  // ことになる。行と吹き出しとボタンとスクロールバーは押して操作するので、そこからはドラッグを始めない。
  let press: { x: number; y: number; onStage: boolean; dragging: boolean } | null = null;
  document.addEventListener("mousedown", (e) => {
    const target = e.target as HTMLElement | null;
    if (e.button !== 0 || onScrollbar(e) || target?.closest(".row, .counts-group, #panel-toggle, #row-order, #bubble")) {
      press = null;
      return;
    }
    const onStage = panelDisplay.show_character && !!target?.closest("#stage");
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
  bindPanelTabs();
  // トレイのメニューで選んだ表示は Rust が保存してから届くので、反映だけをする。起動中に選ばれたものを
  // 取りこぼさないよう保存済みの表示を読む前に聞き始め、読んでいる間に届いていたらそちらを残す。
  let trayChose = false;
  await listen<PanelDisplay>("tray-panel-display", (e) => {
    trayChose = true;
    applyPanelDisplay(e.payload);
  });
  const savedDisplay = await loadPanelDisplay();
  if (!trayChose) panelDisplay = savedDisplay;
  appIcons = await invoke<AppIcons>("app_icons").catch((e) => {
    console.error("app icons", e);
    return appIcons;
  });
  applyCharacterVisibility();
  try {
    scale = new ScaleControl(await invoke<number>("get_scale"));
    scale.bindWheel(stage);
  } catch (e) {
    console.error("scale", e);
  }
  const index = await loadCharacters();
  const saved = await invoke<string>("get_character").catch(() => null);
  for (const id of characterCandidates(saved, index)) {
    if ((await showCharacter(id)) !== "failed") break;
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
  await listen<UsageStatus>("usage-status", (e) => {
    usageStatus = e.payload;
    redrawPanel();
  });
  // 読んでいる間にイベントで新しい状態が届いていたら、そちらを残す。
  const initialUsage = await invoke<UsageStatus>("get_usage_status").catch((e) => {
    console.error("usage status", e);
    return null;
  });
  usageStatus ??= initialUsage;
  await listen<Snapshot>("snapshot", (e) => queueSnapshot(e.payload));
  queueSnapshot(await invoke<Snapshot>("get_snapshot"));
  window.setInterval(redrawPanel, PANEL_REFRESH_MS);
  // 倍率の変更や行の増減で形が変わったら、クリックを受け取る領域を送り直す。
  const observer = new ResizeObserver(() => {
    placeBubble();
    hits.schedule();
  });
  for (const el of [stage, panelElements.panel, bubbleNode]) observer.observe(el);
  window.addEventListener("resize", () => hits.schedule());
}

void start();
