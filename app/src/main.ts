import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

import { Bubble } from "./bubble";
import {
  Acknowledged,
  isRead,
  MARKABLE,
  sessionKey,
  triggerKey,
  unreadMarkable,
  withAcknowledged,
} from "./acknowledged";
import { BubbleModel, type BubbleView } from "./bubbleModel";
import { bubbleEnabled, DEFAULT_BUBBLE_SETTINGS, type BubbleSettings } from "./bubbleSettings";
import { categoryFor, mergeDialogue } from "./dialogue";
import { HitReporter, hitRegions, rectOf, type HitRegions, type Rect } from "./hitArea";
import type { Point } from "./expander";
import { characterCandidates, characterInfo, DEFAULT_CHARACTER, type CharacterInfo } from "./manifest";
import { onScrollbar, READ_KEY, renderPanel } from "./panel";
import { PanelExpansion } from "./panelExpansion";
import { RowHover } from "./rowHover";
import {
  DEFAULT_PANEL_DISPLAY,
  PANEL_STYLES,
  panelView,
  ROW_ORDERS,
  trayList,
  type PanelDisplay,
  type PanelStyle,
  type RowOrder,
} from "./panelModel";
import { createRenderer, loadManifest, type CharacterRenderer, type Manifest, type RendererInput } from "./renderer";
import { ScaleControl } from "./scale";
import {
  SETTINGS_REQUEST_EVENT,
  SETTINGS_STATE_EVENT,
  SETTINGS_WINDOW,
  type SettingsRequest,
  type SettingsState,
} from "./settingsProtocol";
import type { AppIcons, Dialogue, SessionState, Snapshot } from "./types";

const CHARACTER_ROOT = new URL("/character/", window.location.href).href;
// 以前は行を隠す設定だけをこの名前で localStorage に持っていた。表示の保存先を MARIMO_HOME へ
// 移したので、display.json に何も無いときだけ読み替えて引き継ぐ。
const LEGACY_SHOW_ROWS_KEY = "marimo.showRows";
// 利用制限の「古い」「リセット済み」と、今日の既読に数える日は時間だけで変わるので、変更通知とは別に描き直す。
const PANEL_REFRESH_MS = 30_000;
// style.css の #bubble の bottom に足している、立ち絵と吹き出しの間の隙間と揃える必要がある。
const BUBBLE_GAP_PX = 2;
// 押してからこれ以上動いたらドラッグとみなし、立ち絵を押したことにはしない。
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
let bubbleSettings: BubbleSettings = DEFAULT_BUBBLE_SETTINGS;
const bubbleModel = new BubbleModel(acknowledged, undefined, (status) => bubbleEnabled(bubbleSettings, status));
// 知らせの吹き出しは、押すとその行を押したのと同じ操作になる。移動せずに閉じたいときは行と同じ修飾キーを使う。
const bubble = new Bubble(bubbleNode, (e) => {
  const view = bubbleModel.view;
  const session = view ? shown?.sessions.find((s) => triggerKey(s) === view.key) : undefined;
  if (session && !e.altKey) {
    selectSession(session);
    return;
  }
  if (view) bubbleModel.dismiss();
  refreshAcknowledged();
});
const hits = new HitReporter(collectHitRegions);
const expansion = new PanelExpansion(
  panelElements.panel,
  () => hits.flush(),
  () => hits.schedule(),
);
const rowHover = new RowHover(panelElements.panel);

let renderer: CharacterRenderer | undefined;
// manifest を読めなかったキャラクターは選んでも出せないので、読めたものだけを設定のウィンドウに並べる。
let characters: { info: CharacterInfo; manifest: Manifest }[] = [];
let characterId: string | undefined;
// 立ち絵の画像を読む間に別のキャラクターが選ばれることがあるので、最後に頼んだものだけを出す。
let requestedCharacter: string | undefined;
let characterRequest = 0;
let snapshot: Snapshot | null = null;
// snapshot を、見たと示された完了とエラーを除いて集約し直したもの。表情、吹き出し、パネルはこちらを使う。
let shown: Snapshot | null = null;
let dialogue: Dialogue = {};
// 組み込みの既定のセリフ。利用者の dialogue.json は上書きしたい分類だけを持ち、分類ごとに重ねる。
let defaultDialogue: Dialogue = {};
let panelDisplay: PanelDisplay = DEFAULT_PANEL_DISPLAY;
let appIcons: AppIcons = { claude: null, codex: null, hermes: null, opencode: null };
let scale: ScaleControl | undefined;
// ログイン項目の状態はアプリの外（システム設定など）でも変わるので、設定のウィンドウが開くたびに読み直す。
// 読めなかったときは undefined にする。
let autostart: boolean | undefined;
// Rust から届く、ウィンドウの中のカーソル位置。ウィンドウの外では null になる。
let cursor: Point | null = null;
// マウスがパネルの上にある間に既読にした完了とエラーの triggerKey。マウスがパネルを離れるまで、その行をその場所に残す。
const keptRead = new Set<string>();
// 今日の既読の一覧を開いているか。保存せず、起動のたびに閉じた状態から始める。
let todayOpen = false;
// スナップショットは続けて届くことがあり、セリフの読み込みを待つ間に順序が入れ替わらないよう直列にする。
let applying: Promise<void> = Promise.resolve();

function queueSnapshot(next: Snapshot): void {
  applying = applying.then(() => applySnapshot(next)).catch((e) => console.error("snapshot", e));
}

async function applySnapshot(next: Snapshot): Promise<void> {
  snapshot = next;
  acknowledged.prune(next);
  shown = withAcknowledged(next, acknowledged);
  syncAnyUnread();
  syncTraySessions();
  renderer?.update(rendererInput(shown));
  // セリフの JSON はユーザーが編集するものなので、新しいトリガーのたびに読み直して再起動なしで反映する。
  if (bubbleModel.needsText(shown)) await reloadDialogue();
  bubbleModel.update(shown, dialogue);
  showSpeech();
  redrawPanel();
}

// 見たと示したトリガーが増えたら、集約をやり直して表情、吹き出し、パネルへ反映する。
function refreshAcknowledged(): void {
  if (snapshot) {
    shown = withAcknowledged(snapshot, acknowledged);
    syncAnyUnread();
    syncTraySessions();
    renderer?.update(rendererInput(shown));
    bubbleModel.update(shown, dialogue);
  }
  showSpeech();
  redrawPanel();
}

// 表情は、集約で選ばれたセッションの使っているツールと、吹き出しのセリフと同じ分類から決める。
// 既読のセッションは集約に効いていないので、同じ状態でも選ばない。
function rendererInput(s: Snapshot): RendererInput {
  const focus = s.sessions.find((x) => x.status === s.aggregate && !isRead(x, acknowledged));
  return { status: s.aggregate, tool: focus?.activity?.tool ?? null, category: focus ? categoryFor(focus) : null };
}

async function reloadDialogue(): Promise<void> {
  const user = await invoke<unknown>("get_dialogue").catch((e) => {
    console.error("dialogue", e);
    return {};
  });
  dialogue = mergeDialogue(defaultDialogue, user);
}

// 立ち絵を隠している間は吹き出しを出さない。知らせの吹き出しはトリガーが続く間 bubbleModel に残るので、
// 立ち絵を戻したときにまだ続いていれば、そのとき出る。
function showSpeech(): void {
  const view = panelDisplay.show_character ? bubbleModel.view : null;
  scheduleExpiry(view);
  if (view) bubble.show(view.text, `クリックするとセッションへ移動します（${READ_KEY} を押しながらクリックすると閉じます）`);
  else bubble.hide();
  placeBubble();
  hits.schedule();
}

// 知らせの吹き出しの表示時間は、出ている間だけ数える。立ち絵を隠している間は数えず、戻したときに
// 数え直す。秒数の設定を変えたときも数え直す。
let expiry: { key: string; seconds: number; timer: number } | undefined;

function scheduleExpiry(view: BubbleView | null): void {
  const seconds = bubbleSettings.bubble_seconds;
  if (view && seconds > 0 && expiry?.key === view.key && expiry.seconds === seconds) return;
  if (expiry) window.clearTimeout(expiry.timer);
  expiry = undefined;
  if (!view || seconds <= 0) return;
  const key = view.key;
  const timer = window.setTimeout(() => {
    expiry = undefined;
    bubbleModel.expire(key);
    showSpeech();
  }, seconds * 1000);
  expiry = { key, seconds, timer };
}

// 頭の上に出した吹き出しは、立ち絵の外側の端に揃えてパネルの側へ伸びるので、背の低い立ち絵や上の隅では
// パネルやタブに重なることがある。重なるときだけその上端より上へ持ち上げ、ウィンドウの上端からははみ出させない。
// 吹き出しには動きの transform が掛かるので、大きさは transform の影響を受けない offset の値で測る。
function placeBubble(): void {
  const box = stage.getBoundingClientRect();
  const bottom = box.top - BUBBLE_GAP_PX;
  const width = bubbleNode.offsetWidth;
  const [left, right] = app.classList.contains("left") ? [box.left, box.left + width] : [box.right - width, box.right];
  const tops = [rectOf(panelElements.panel), rectOf(panelElements.toggle), rectOf(panelElements.order)]
    .filter((r): r is Rect => r !== null && r.x + r.w > left && r.x < right)
    .map((r) => r.y - BUBBLE_GAP_PX);
  const lift = Math.max(0, Math.min(bottom - Math.min(bottom, ...tops), bottom - bubbleNode.offsetHeight));
  bubbleNode.style.setProperty("--bubble-lift", `${lift}px`);
}

function collectHitRegions(): HitRegions {
  // 切り替えと並べ方のボタンはパネルの上辺の外に付けるので、パネルとは別に加える。
  const rects = [rectOf(panelElements.panel), rectOf(panelElements.toggle), rectOf(panelElements.order)];
  // 広げたレイヤーはパネルの外へ伸びるので、表示中のもの（見せる前に測っているものを含む）を加える。
  for (const layer of panelElements.panel.querySelectorAll(".hover-layer")) rects.push(rectOf(layer));
  if (bubbleNode.classList.contains("show")) rects.push(rectOf(bubbleNode));
  if (!panelDisplay.show_character) return hitRegions(rects, null);
  const area = renderer?.hitArea();
  return hitRegions(rects, { box: rectOf(area?.element ?? stage), mask: area?.mask ?? null });
}

function redrawPanel(): void {
  const now = Date.now();
  const view = panelView(shown, acknowledged, panelDisplay.panel_style, panelDisplay.row_order, now, keptRead);
  const limits = { claude: shown?.rate_limits ?? null, codex: shown?.codex_rate_limits ?? null };
  const actions = { select: selectSession, markRead, toggleToday };
  renderPanel(panelElements, view, limits, appIcons, now, actions, todayOpen);
  placeBubble();
  expansion.evaluate();
  rowHover.apply();
  hits.schedule();
}

function selectSession(session: SessionState): void {
  if (!session.cloud) {
    void invoke("focus_session", { provider: session.provider ?? "claude", sessionId: session.session_id }).catch(
      (e) => console.error("focus", e),
    );
  }
  markRead(session);
}

function toggleToday(): void {
  todayOpen = !todayOpen;
  redrawPanel();
}

// ウィンドウは透明な部分でマウスのイベントを受け取らないので、DOM の mouseleave ではなく Rust から届く位置で見る。
// 件数の行に重ねた詳細はパネルの外へ伸びるが、DOM ではパネルの中にあるので、その上もパネルの上として扱う。
function pointerOverPanel(): boolean {
  const hit = cursor ? document.elementFromPoint(cursor.x, cursor.y) : null;
  return !!hit && panelElements.panel.contains(hit);
}

// 既読にした完了とエラーの行は折りたたむが、マウスがパネルの上にあればその場所に薄く残し、離れたときに折りたたむ。
// 既に既読のもの（今日の既読の一覧の行）を押したときは、一覧から上へ戻さない。
function acknowledge(session: SessionState): void {
  const key = triggerKey(session);
  if (MARKABLE.has(session.status) && !acknowledged.has(session) && pointerOverPanel()) keptRead.add(key);
  acknowledged.add(key);
}

function releaseKeptRead(): void {
  if (keptRead.size === 0 || pointerOverPanel()) return;
  keptRead.clear();
  redrawPanel();
}

// 行のトリガーを見たものとして扱い、同じトリガーの吹き出しも閉じる。承認待ちの行は、解決するまで残す。
function markRead(session: SessionState): void {
  acknowledge(session);
  if (bubbleModel.view?.key === triggerKey(session)) bubbleModel.dismiss();
  refreshAcknowledged();
}

// 未読の完了とエラーの有無は、トレイのメニューの項目を使えるかどうかに要る。変わったときだけ Rust に知らせる。
let anyUnreadKnown: boolean | undefined;

function syncAnyUnread(): void {
  const any = unreadMarkable(snapshot, acknowledged).length > 0;
  if (any === anyUnreadKnown) return;
  anyUnreadKnown = any;
  void invoke("set_any_unread", { any }).catch((e) => console.error("any unread", e));
}

// トレイのメニューは Rust が持つので、並べるセッションの一覧が変わったときだけ送る。
let traySessionsSent: string | undefined;

function syncTraySessions(): void {
  const list = trayList(snapshot, acknowledged);
  const serialized = JSON.stringify(list);
  if (serialized === traySessionsSent) return;
  traySessionsSent = serialized;
  void invoke("set_tray_sessions", { items: list.items, more: list.more }).catch((e) =>
    console.error("tray sessions", e),
  );
}

// 選ばれたときに一覧を送ってから状態が変わっていることがあるので、今のスナップショットから探す。
// 見つからなければ、そのセッションはもう無いので何もしない。
function selectTraySession(key: string): void {
  const session = shown?.sessions.find((s) => sessionKey(s) === key);
  if (session) selectSession(session);
}

// 承認待ちは、解決するまで残す行なので対象にしない。
function markAllRead(): void {
  const unread = unreadMarkable(snapshot, acknowledged);
  for (const s of unread) acknowledge(s);
  if (bubbleModel.view && unread.some((s) => triggerKey(s) === bubbleModel.view?.key)) bubbleModel.dismiss();
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
// セリフは立ち絵より先に読む。mount は読み終えた最後に立ち絵を DOM へ加えるので、その後で待たずに
// 古いものを外せば、新旧の立ち絵が重なって描かれることがない。
// 立ち絵の領域の高さは素材の縦横比で決まり、ウィンドウの大きさも合わせるよう Rust に知らせる。
async function showCharacter(id: string): Promise<"shown" | "failed" | "superseded"> {
  const request = ++characterRequest;
  requestedCharacter = id;
  const superseded = () => request !== characterRequest;
  const entry = characters.find((c) => c.info.id === id);
  const base = characterBase(id);
  const nextDialogue = await fetch(new URL("dialogue.json", base))
    .then((r) => (r.ok ? (r.json() as Promise<Dialogue>) : {}))
    .catch(() => ({}));
  if (superseded()) return "superseded";
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
  if (shown) renderer.update(rendererInput(shown));
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
  applyPanelOpacity(display.panel_opacity);
  showSpeech();
  redrawPanel();
  publishSettings();
}

function setPanelDisplay(display: PanelDisplay): void {
  applyPanelDisplay(display);
  void invoke("set_panel_display", { display }).catch((e) => console.error("panel display", e));
}

function setBubbleSettings(next: BubbleSettings): void {
  bubbleSettings = next;
  void invoke("set_bubble_settings", { settings: next }).catch((e) => console.error("bubble settings", e));
  if (shown) bubbleModel.update(shown, dialogue);
  showSpeech();
  publishSettings();
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

interface Corner {
  left: boolean;
  top: boolean;
}

// 立ち絵の隅が変わると、パネルの位置と伸びる向き、吹き出しの向きが変わるので、測り直して描き直す。
function applyLayout(corner: Corner): void {
  app.classList.toggle("left", corner.left);
  app.classList.toggle("top", corner.top);
  redrawPanel();
}

// 立ち絵は隠している間も描き続け、瞬きや表情の切り替えの時計も止めない。戻したときに今の状態の
// 表情がすぐ出るようにするためである。
function applyCharacterVisibility(): void {
  app.classList.toggle("character-hidden", !panelDisplay.show_character);
}

function applyPanelOpacity(opacity: number): void {
  document.documentElement.style.setProperty("--panel-alpha", String(opacity));
}

// 設定のウィンドウから頼まれた変更は、メニューやパネルのボタンと同じ経路で反映して保存し、反映したあとの状態を
// 送り返す。トレイで変えたときも同じように送る。
function settingsState(): SettingsState {
  return {
    display: panelDisplay,
    bubble: bubbleSettings,
    characters: characters.map(({ info }) => ({ id: info.id, name: info.displayName })),
    character: characterId ?? null,
    scale: scale?.current ?? null,
    autostart: autostart ?? null,
  };
}

function publishSettings(): void {
  void emitTo(SETTINGS_WINDOW, SETTINGS_STATE_EVENT, settingsState()).catch((e) => console.error("settings state", e));
}

async function readAutostart(): Promise<void> {
  autostart = await isEnabled().catch((e) => {
    console.error("autostart", e);
    return undefined;
  });
}

async function handleSettingsRequest(request: SettingsRequest): Promise<void> {
  switch (request.kind) {
    case "sync":
      await readAutostart();
      break;
    case "display":
      setPanelDisplay({ ...panelDisplay, ...request.patch });
      return;
    case "opacity-preview":
      applyPanelOpacity(request.opacity);
      return;
    case "bubble":
      setBubbleSettings({ ...bubbleSettings, ...request.patch });
      return;
    case "character":
      await switchCharacter(request.id);
      break;
    case "scale":
      // 反映したあとの倍率は set_scale の戻りで決まるので、ScaleControl の知らせを待って送る。
      scale?.set(request.scale);
      return;
    case "autostart":
      await (request.enabled ? enable() : disable()).catch((e) => console.error("autostart", e));
      await readAutostart();
      break;
  }
  publishSettings();
}

const PANEL_STYLE_LABEL: Record<PanelStyle, string> = {
  detail: "詳細を表示",
  counts: "件数だけ表示",
};

async function openMenu(): Promise<void> {
  const menu = await Menu.new({
    items: [
      await CheckMenuItem.new({
        text: "立ち絵を表示",
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
      await MenuItem.new({
        text: "すべて既読にする",
        enabled: unreadMarkable(snapshot, acknowledged).length > 0,
        action: markAllRead,
      }),
      await PredefinedMenuItem.new({ item: "Separator" }),
      await MenuItem.new({
        text: "設定…",
        action: () => void invoke("open_settings").catch((e) => console.error("open settings", e)),
      }),
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
  // ことになり、パネルの詳細と件数を切り替える。行と吹き出しとボタンとスクロールバーは押して操作するので、そこからはドラッグを始めない。
  let press: { x: number; y: number; onStage: boolean; dragging: boolean } | null = null;
  document.addEventListener("mousedown", (e) => {
    const target = e.target as HTMLElement | null;
    if (e.button !== 0 || onScrollbar(e) || target?.closest(".row, .today-line, .counts-group, #panel-toggle, #row-order, #bubble")) {
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
    if (press && !press.dragging && press.onStage)
      setPanelStyle(panelDisplay.panel_style === "detail" ? "counts" : "detail");
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
  await listen("tray-mark-all-read", markAllRead);
  await listen<string>("tray-session", (e) => selectTraySession(e.payload));
  await listen<SettingsRequest>(SETTINGS_REQUEST_EVENT, (e) => void handleSettingsRequest(e.payload));
  await listen<Corner>("layout", (e) => applyLayout(e.payload));
  applyLayout(await invoke<Corner>("get_layout").catch(() => ({ left: false, top: false })));
  const savedDisplay = await loadPanelDisplay();
  if (!trayChose) panelDisplay = savedDisplay;
  bubbleSettings = await invoke<BubbleSettings>("get_bubble_settings").catch((e) => {
    console.error("bubble settings", e);
    return DEFAULT_BUBBLE_SETTINGS;
  });
  appIcons = await invoke<AppIcons>("app_icons").catch((e) => {
    console.error("app icons", e);
    return appIcons;
  });
  applyCharacterVisibility();
  applyPanelOpacity(panelDisplay.panel_opacity);
  try {
    scale = new ScaleControl(await invoke<number>("get_scale"), () => publishSettings());
    scale.bindWheel(stage);
  } catch (e) {
    console.error("scale", e);
  }
  const index = await loadCharacters();
  const saved = await invoke<string>("get_character").catch(() => null);
  for (const id of characterCandidates(saved, index)) {
    if ((await showCharacter(id)) !== "failed") break;
  }
  // 読み込みの途中で設定のウィンドウが開かれていたら、キャラクターの一覧を送り直す。
  publishSettings();
  // マウスが立ち絵の上にあるかは Rust 側がクリックを通す判定のついでに調べて知らせる。透明な部分では
  // ウィンドウがマウスのイベントを受け取らないので、DOM の mouseleave は当てにできない。
  await listen<boolean>("portrait-hover", (e) => renderer?.setHover(e.payload));
  await listen<Point | null>("window-cursor", (e) => {
    cursor = e.payload;
    expansion.setCursor(e.payload);
    rowHover.setCursor(e.payload);
    releaseKeptRead();
  });
  // 最初のスナップショットより先に戻さないと、既読の完了やエラーの吹き出しが一度出てしまう。
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
  const observer = new ResizeObserver(() => {
    placeBubble();
    hits.schedule();
  });
  for (const el of [stage, panelElements.panel, bubbleNode]) observer.observe(el);
  window.addEventListener("resize", () => hits.schedule());
}

void start();
