import { isRead, sessionKey, STATUS_PRIORITY, triggerKey, type Acknowledged } from "./acknowledged";
import { chatTitle, folderName, formatAge, statusLabel } from "./format";
import { lang, t } from "./i18n";
import type { SessionState, Snapshot, Status } from "./types";

/** 並べ方は詳細と件数のどちらの出し方にも効く。 */
export type RowOrder = "started" | "status" | "updated";

export const ROW_ORDERS: readonly RowOrder[] = ["started", "status", "updated"];

// 英語はトレイのメニュー（tray.rs の order_label）と揃える。
export function rowOrderLabel(order: RowOrder): string {
  switch (order) {
    case "started":
      return t("開始順", "Start Time");
    case "status":
      return t("状態順", "Status");
    case "updated":
      return t("更新順", "Last Update");
  }
}

export function rowOrderDescription(order: RowOrder): string {
  switch (order) {
    case "started":
      return t("始まった時刻の新しい順", "Most recently started first");
    case "status":
      return t("承認待ち、エラー、作業中、完了の順", "Waiting, error, working, then done");
    case "updated":
      return t("最後に更新された時刻の新しい順", "Most recently updated first");
  }
}

/** パネルの並べ方のボタンは、押すたびに ROW_ORDERS の順に次の並べ方へ移る。 */
export function nextRowOrder(order: RowOrder): RowOrder {
  return ROW_ORDERS[(ROW_ORDERS.indexOf(order) + 1) % ROW_ORDERS.length];
}

export interface PanelPlan {
  rows: SessionState[];
  /** その場所に残している既読の完了とエラーの行の sessionKey。薄く描く。 */
  read: ReadonlySet<string>;
  /** 今日完了かエラーになって既読にしたセッションのうち、その場所に残していないもの。その状態になった時刻の新しい順。 */
  today: SessionState[];
}


/**
 * 待機以外のセッションは作業中も折りたたまずに 1 行ずつ並べ、パネルに収まらない分はスクロールで見せるので、
 * 行の数に上限を設けない。
 * 見たと示された完了とエラーは折りたたむが、keep に triggerKey のある行だけはその場所に薄くして残す。パネルは下端を
 * 固定しているので、続けて ✓ を押す間に行が折りたたまれると上の行が下へずれ、次に押そうとした行がマウスの下から
 * 動いてしまうからである。keep はマウスがパネルの上にある間に既読にした行で、呼び出し側がマウスがパネルを
 * 離れたときに空にする。状態順でも、残した行は未読の同じ状態の行と同じ規則で並べて動かさない。
 * 折りたたんだ既読の行のうち、その状態になったのが今日のものは today に集め、パネルの下の 1 行から開いて見せる。
 * 完了やエラーのまま放っておかれるセッションは多く、前の日の分まで並べると古い既読で埋まる。
 *
 * started は started_at の新しい順に並べる。パネルは下端を固定して上へ伸びるので、新しいセッションが
 * 一番上に加わっても既にある行は画面上の位置が変わらず、状態が変わっても行は動かない。started_at を
 * 持たない古いファイルのセッションは最も古いものとして一番下に置き、更新のたびに動かないようにする。
 * どの並べ方でも最後は sessionKey で決め、描き直すたびに同じ順にする。
 */
export function planPanel(
  snapshot: Snapshot | null,
  ack: Acknowledged,
  order: RowOrder = "started",
  now: number = Date.now(),
  keep: ReadonlySet<string> = new Set(),
): PanelPlan {
  const active = (snapshot?.sessions ?? []).filter((s) => s.status !== "idle");
  const kept = active.filter((s) => isRead(s, ack) && keep.has(triggerKey(s)));
  const read = new Set(kept.map(sessionKey));
  const compare: Record<RowOrder, (a: SessionState, b: SessionState) => number> = {
    started: (a, b) => b.started_at - a.started_at,
    status: (a, b) =>
      STATUS_PRIORITY.indexOf(a.status) - STATUS_PRIORITY.indexOf(b.status) ||
      b.updated_at - a.updated_at,
    updated: (a, b) => b.updated_at - a.updated_at,
  };
  const rows = [...active.filter((s) => !isRead(s, ack)), ...kept].sort(
    (a, b) => compare[order](a, b) || compareIds(sessionKey(a), sessionKey(b)),
  );
  const today = active
    .filter((s) => isRead(s, ack) && !keep.has(triggerKey(s)) && sameLocalDay(s.status_since, now))
    .sort((a, b) => b.status_since - a.status_since || compareIds(sessionKey(a), sessionKey(b)));
  return { rows, read, today };
}

function sameLocalDay(a: number, b: number): boolean {
  const x = new Date(a);
  const y = new Date(b);
  return x.getFullYear() === y.getFullYear() && x.getMonth() === y.getMonth() && x.getDate() === y.getDate();
}

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

export function isEmpty(plan: PanelPlan): boolean {
  return plan.rows.length === 0;
}

/** 行の出し方は、立ち絵を出すかどうかとは独立に選べる。 */
export type PanelStyle = "detail" | "counts";

export const PANEL_STYLES: readonly PanelStyle[] = ["detail", "counts"];

// 範囲と既定は Rust 側の scale.rs と揃える（sharedConstants.test.ts が照合する）。保存するときは Rust が範囲に収め直す。
export const PANEL_OPACITY_MIN = 0.2;
export const PANEL_OPACITY_MAX = 1;
export const DEFAULT_PANEL_OPACITY = 0.8;

/** Rust の scale::PanelDisplay と同じ形で、display.json に保存する。 */
export interface PanelDisplay {
  show_character: boolean;
  panel_style: PanelStyle;
  row_order: RowOrder;
  /** パネルの背景だけの不透明度。文字やアイコンや状態の点は薄くしない。 */
  panel_opacity: number;
}

export const DEFAULT_PANEL_DISPLAY: PanelDisplay = {
  show_character: true,
  panel_style: "detail",
  row_order: "started",
  panel_opacity: DEFAULT_PANEL_OPACITY,
};

export interface StatusCount {
  status: Status;
  count: number;
}

/** 件数の行の 1 項目。「承認待ち 1」のように状態と件数を並べる。 */
export function countLabel({ status, count }: StatusCount): string {
  return `${statusLabel(status)} ${count}`;
}

export interface PanelView {
  style: PanelStyle;
  order: RowOrder;
  plan: PanelPlan;
  counts: StatusCount[];
  /** 件数の行を押したときに移動する、最も優先度の高い要対応のセッション。 */
  target: SessionState | null;
}

const ATTENTION: ReadonlySet<Status> = new Set(["waiting", "error", "done"]);
// 件数の行は待機を数えない。
const COUNT_ORDER = STATUS_PRIORITY.filter((s) => s !== "idle");

/**
 * 件数だけの表示でも数え方は詳細と同じにし、見たと示された完了とエラーは数えない。吹き出しや表情を決める
 * 仕組みは表示によらず同じで、立ち絵を隠している間はそれを見せないだけにする。
 */
export function panelView(
  snapshot: Snapshot | null,
  ack: Acknowledged,
  style: PanelStyle,
  order: RowOrder = "started",
  now: number = Date.now(),
  keep: ReadonlySet<string> = new Set(),
): PanelView {
  const plan = planPanel(snapshot, ack, order, now, keep);
  const sessions = snapshot?.sessions ?? [];
  const counted = sessions.filter((s) => !isRead(s, ack));
  const counts = COUNT_ORDER.map((status) => ({
    status,
    count: counted.filter((s) => s.status === status).length,
  })).filter((c) => c.count > 0);
  const target = counted.find((s) => ATTENTION.has(s.status)) ?? null;
  return { style, order, plan, counts, target };
}

/**
 * 出す行が無いときも、パネルは無いことを示す 1 行を出して残す。
 * パネルごと消えると、並べ方を切り替える場所も、立ち絵を隠しているときに右クリックやドラッグを
 * 受ける場所も無くなるからである。
 */
export function hasContent(view: PanelView): boolean {
  return view.style === "detail" ? !isEmpty(view.plan) : view.counts.length > 0;
}

/**
 * パネルの行の描き方を決める入力をまとめた文字列。同じなら作り直しても同じ行になるので、呼び出し側は DOM を
 * 置き換えずに済ませる。作り直すと押している途中の行やボタンが消えてクリックを取りこぼし、ツールチップも消えるからである。
 * 行の押したときの操作はセッションの値を持つので、表示に出ない値もセッションごと含める。時刻からは経過時間の
 * 表記だけが表示に効くので、now そのものではなくその表記を含める。
 */
export function panelSignature(view: PanelView, now: number, todayOpen: boolean): string {
  const { plan } = view;
  const ages = [...plan.rows, ...plan.today].map((s) => formatAge(s.updated_at, now));
  return JSON.stringify([
    lang(),
    view.style,
    view.order,
    view.counts,
    view.target,
    todayOpen,
    plan.rows,
    [...plan.read].sort(),
    plan.today,
    ages,
  ]);
}

/** トレイのメニューの先頭に並べる、手を貸す必要のあるセッション。key は sessionKey である。 */
export interface TrayItem {
  key: string;
  label: string;
}

export interface TrayList {
  items: TrayItem[];
  /** 上限を超えて並べなかった件数。 */
  more: number;
}

// メニューが長くなると下の操作が画面の外へ押し出されるので、件数と 1 項目の長さを抑える。
export const TRAY_LIMIT = 10;
const TRAY_LABEL_CHARS = 40;
// 名前が長くてもタイトルの頭は読めるよう、タイトルにはこれだけの字数を残す。
const TRAY_TITLE_MIN_CHARS = 10;
const TRAY_ORDER: Status[] = ["waiting", "error", "done"];

/**
 * 承認待ち、まだ既読にしていないエラーと完了を、この状態の順に、同じ状態の中では更新の新しい順に並べる。
 * 並べ方の選択には従わない。メニューは開いたときに一番上から読むので、急ぐものを上に置く。
 */
export function trayList(snapshot: Snapshot | null, ack: Acknowledged): TrayList {
  const sessions = (snapshot?.sessions ?? [])
    .filter((s) => TRAY_ORDER.includes(s.status) && !isRead(s, ack))
    .sort(
      (a, b) =>
        TRAY_ORDER.indexOf(a.status) - TRAY_ORDER.indexOf(b.status) ||
        b.updated_at - a.updated_at ||
        compareIds(sessionKey(a), sessionKey(b)),
    );
  const items = sessions.slice(0, TRAY_LIMIT).map((s) => ({ key: sessionKey(s), label: trayLabel(s) }));
  return { items, more: sessions.length - items.length };
}

// 字数はコードポイントで数え、Rust の activity::clip と同じく末尾を … にする。
function trayLabel(s: SessionState): string {
  const head = `${statusLabel(s.status)}  ${folderName(s)}`;
  const title = chatTitle(s);
  if (!title) return head;
  const room = Math.max(TRAY_LABEL_CHARS - [...head].length - 2, TRAY_TITLE_MIN_CHARS);
  const chars = [...title];
  const clipped = chars.length <= room ? title : `${chars.slice(0, room - 1).join("").trimEnd()}…`;
  return `${head}  ${clipped}`;
}
