import { expect, test, type Page } from "@playwright/test";

import { emit, mockTauri } from "./tauriMock";

// 背の低い clawd を使う。立ち絵の上端が上の隅と下の隅で違う位置になり、配置の取り違えが見える。
// 窓は scale.rs の window_size と同じ大きさにする（倍率 1、縦横比 0.5）。
const WIDTH = 516;
const HEIGHT = 380;
const MARGIN = 8;
const BUBBLE_ROOM = 120;
const STAGE = { w: 180, h: 90 };
const PANEL_W = 312;

function snapshot() {
  const now = Date.now();
  const base = { cwd: null, started_at: now - 60_000, status_since: now - 60_000, last_event: null, context: null };
  const agent = { started_at: now, last_seen: now, agent_type: "Explore" };
  return {
    aggregate: "waiting",
    rate_limits: null,
    sessions: [
      {
        ...base,
        session_id: "wait-1",
        title: "承認待ちの作業",
        status: "waiting",
        status_reason: "permission",
        updated_at: now,
        activity: { kind: "tool", tool: "Bash", summary: "npm test" },
      },
      {
        ...base,
        session_id: "work-1",
        title: "動いている作業",
        status: "working",
        updated_at: now,
        activity: { kind: "tool", tool: "Read", summary: "main.ts を読んでいる" },
        agents: { a: agent, b: agent, c: agent },
      },
    ],
  };
}

async function open(page: Page): Promise<void> {
  await page.setViewportSize({ width: WIDTH, height: HEIGHT });
  await mockTauri(page, {
    get_layout: { left: false, top: false },
    get_snapshot: snapshot(),
    get_scale: 1,
    get_character: "clawd",
    get_panel_display: null,
    get_bubble_settings: { bubble_waiting: true, bubble_done: true, bubble_error: true, bubble_seconds: 0 },
    get_acknowledged: [],
    get_dialogue: {},
    app_icons: { claude: null, codex: null, hermes: null, opencode: null },
  });
  await page.goto("/");
  await expect(page.locator("#rows .row")).toHaveCount(2);
  await expect(page.locator("#stage .character")).toHaveCount(1);
  await expect(page.locator("#bubble")).toHaveClass(/show/);
}

const box = async (page: Page, selector: string) => {
  const b = await page.locator(selector).first().boundingBox();
  if (!b) throw new Error(`${selector} is not shown`);
  return { left: b.x, top: b.y, right: b.x + b.width, bottom: b.y + b.height };
};

test("each corner places the stage, panel and bubble", async ({ page }, info) => {
  await open(page);
  for (const corner of [
    { left: false, top: false },
    { left: true, top: false },
    { left: false, top: true },
    { left: true, top: true },
  ]) {
    const name = `${corner.top ? "top" : "bottom"}-${corner.left ? "left" : "right"}`;
    await emit(page, "layout", corner);
    await page.waitForTimeout(50);
    await page.screenshot({ path: info.outputPath(`${name}.png`), animations: "disabled" });
    const stage = await box(page, "#stage");
    const panel = await box(page, "#panel");
    const bubble = await box(page, "#bubble");

    expect(stage.left, name).toBeCloseTo(corner.left ? MARGIN : WIDTH - MARGIN - STAGE.w, 0);
    expect(stage.top, name).toBeCloseTo(corner.top ? BUBBLE_ROOM : HEIGHT - MARGIN - STAGE.h, 0);
    // パネルは立ち絵の内側の隣に置き、立ち絵と同じ端を揃えて反対の向きへ伸ばす。
    const panelLeft = corner.left ? MARGIN + STAGE.w + MARGIN : WIDTH - MARGIN - STAGE.w - MARGIN - PANEL_W;
    expect(panel.left, name).toBeCloseTo(panelLeft, 0);
    if (corner.top) expect(panel.top, name).toBeCloseTo(BUBBLE_ROOM, 0);
    else expect(panel.bottom, name).toBeCloseTo(HEIGHT - MARGIN, 0);
    // 吹き出しは立ち絵の外側の端に揃え、窓の上端からはみ出さない。
    if (corner.left) expect(bubble.left, name).toBeCloseTo(stage.left, 0);
    else expect(bubble.right, name).toBeCloseTo(stage.right, 0);
    expect(bubble.top, name).toBeGreaterThanOrEqual(0);
  }
});

test("pressing the stage switches the panel between detail and counts", async ({ page }) => {
  await open(page);
  await expect(page.locator(".row-agents")).toHaveText("サブ 3");
  const pressed = page.locator("#panel-toggle button[aria-pressed=true]");
  await expect(pressed).toHaveText("詳細");

  const stage = await box(page, "#stage");
  const center = { x: (stage.left + stage.right) / 2, y: (stage.top + stage.bottom) / 2 };
  for (const expected of ["件数", "詳細"]) {
    await page.mouse.click(center.x, center.y);
    await expect(pressed).toHaveText(expected);
  }
  const saved = await page.evaluate(() =>
    ((window as unknown as { __marimoCalls: [string, { display?: { panel_style: string } }][] }).__marimoCalls)
      .filter(([cmd]) => cmd === "set_panel_display")
      .map(([, args]) => args.display?.panel_style),
  );
  expect(saved).toEqual(["counts", "detail"]);
});
