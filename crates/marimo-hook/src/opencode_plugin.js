// marimo-opencode-plugin
// marimo-hook install が書いたファイルで、marimo-hook uninstall で消える。手で書き換えると、次の install で
// 元に戻る。OpenCode のセッションの状態を marimo のパネルに出すため、イベントを marimo-hook へ渡す。
import { spawn, spawnSync } from "node:child_process"

const HOOK = __MARIMO_HOOK__

// 行の状態を決めるのに使うイベントだけを渡す。delta のように流量の多いものはここで捨てる。
const FORWARD = new Set([
  "session.created",
  "session.renamed",
  "session.deleted",
  "session.execution.started",
  "session.execution.succeeded",
  "session.execution.failed",
  "session.execution.interrupted",
  "session.step.ended",
  "session.tool.input.started",
  "session.tool.called",
  "session.tool.success",
  "session.tool.failed",
  "permission.asked",
  "permission.replied",
  "form.created",
  "form.replied",
  "form.cancelled",
])

const LIMIT = 500

// プラグインが場所ごとに読み込まれても、同じモジュールの値を分け合って同じイベントを二度渡さない。
const seen = new Set()
const children = new Map()
const known = new Map()
const tools = new Map()
const models = new Map()
const lastText = new Map()
const windows = new Map()
const queue = []
let running = false
let listing = Promise.resolve()

function remember(map, key, value) {
  map.delete(key)
  map.set(key, value)
  if (map.size > LIMIT) map.delete(map.keys().next().value)
}

function firstTime(id) {
  if (typeof id !== "string") return true
  if (seen.has(id)) return false
  seen.add(id)
  if (seen.size > LIMIT) seen.delete(seen.values().next().value)
  return true
}

// marimo-hook は一つのイベントごとに起動する。続けて届いたイベントを別々のプロセスで同時に書くと
// 書く順が入れ替わり、完了の後に作業中へ戻ることがあるので、前のプロセスが終わってから次を起動する。
// 待つのはこの列だけで、OpenCode の処理は待たせない。
function send(payload) {
  if (queue.length >= LIMIT) queue.shift()
  queue.push(JSON.stringify(payload))
  if (!running) next()
}

function next() {
  const body = queue.shift()
  if (body === undefined) {
    running = false
    return
  }
  running = true
  let finished = false
  const finish = () => {
    if (finished) return
    finished = true
    clearTimeout(timer)
    next()
  }
  const timer = setTimeout(finish, 10000)
  timer.unref?.()
  try {
    // detached にしておかないと、OpenCode が終わるときに書きかけの marimo-hook も一緒に止まる。
    const child = spawn(HOOK, ["opencode-hook"], {
      stdio: ["pipe", "ignore", "ignore"],
      detached: true,
      windowsHide: true,
    })
    child.on("error", finish)
    child.on("close", finish)
    child.stdin.on("error", () => {})
    child.stdin.end(body)
    child.unref()
  } catch {
    finish()
  }
}

// プラグインを外すとき（opencode run --standalone のように OpenCode が終わるときを含む）は、まだ渡して
// いないイベントをその場で渡し切る。終わった後の OpenCode は列の続きを起動しないので、完了の知らせを失う。
function flush() {
  // 起動済みの marimo-hook が書き終える前に後のイベントを書くと順が入れ替わるので、少し待つ。
  if (running) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 300)
  for (let body = queue.shift(); body !== undefined; body = queue.shift()) {
    try {
      spawnSync(HOOK, ["opencode-hook"], {
        input: body,
        stdio: ["pipe", "ignore", "ignore"],
        timeout: 2000,
        windowsHide: true,
      })
    } catch {}
  }
}

async function lookup(ctx, sessionID) {
  try {
    const info = await ctx.session.get({ sessionID })
    return info?.data ?? info
  } catch {
    return undefined
  }
}

async function loadWindows(ctx) {
  try {
    const listed = await ctx.model.list({})
    for (const m of Array.isArray(listed) ? listed : (listed?.data ?? [])) {
      if (m?.limit?.context > 0) windows.set(`${m.providerID}/${m.id}`, m.limit.context)
    }
  } catch {}
}

// モデルの一覧は読み込みのときに引いておき、ステップの終わりで待たずに済むようにする。一覧に無いモデルに
// 切り替えたときだけ引き直し、それでも無ければ上限を渡さない。
async function contextWindow(ctx, model) {
  if (!model?.id || !model?.providerID) return undefined
  const key = `${model.providerID}/${model.id}`
  await listing
  if (!windows.has(key)) {
    listing = loadWindows(ctx)
    await listing
    if (!windows.has(key)) windows.set(key, 0)
  }
  return windows.get(key) || undefined
}

async function handle(ctx, event) {
  if (!event || typeof event.type !== "string" || !firstTime(event.id)) return
  const data = event.data ?? {}
  const sessionID = data.sessionID ?? data.form?.sessionID
  if (typeof sessionID !== "string") return
  if (event.type === "session.step.started") {
    remember(models, sessionID, data.model)
    return
  }
  if (event.type === "session.text.ended") {
    remember(lastText, sessionID, data.text)
    return
  }
  if (!FORWARD.has(event.type)) return

  const payload = { id: event.id, type: event.type, location: event.location, data }
  // サブエージェントは親の行の作業として見えるので、自分の行を作らせない。作成のイベントを見ていない
  // セッションは、OpenCode に親と題名と場所を尋ねてから扱う。
  if (event.type === "session.created") {
    if (data.parentID) {
      remember(children, sessionID, true)
      return
    }
    remember(known, sessionID, true)
  } else if (children.has(sessionID)) {
    return
  } else if (!known.has(sessionID)) {
    const info = await lookup(ctx, sessionID)
    if (info?.parentID) {
      remember(children, sessionID, true)
      return
    }
    remember(known, sessionID, true)
    if (info) payload.session = { title: info.title, directory: info.location?.directory }
  }

  switch (event.type) {
    case "session.tool.input.started":
      remember(tools, data.id, data.name)
      break
    case "session.tool.called":
      payload.tool = tools.get(data.id)
      break
    case "session.tool.success":
    case "session.tool.failed":
      tools.delete(data.id)
      break
    case "session.execution.started":
      lastText.delete(sessionID)
      break
    case "session.execution.succeeded":
      payload.last_text = lastText.get(sessionID)
      break
    case "session.step.ended":
      payload.context_window = await contextWindow(ctx, models.get(sessionID))
      break
  }
  send(payload)
}

export default {
  id: "marimo",
  setup(ctx) {
    listing = loadWindows(ctx)
    const controller = new AbortController()
    // OpenCode は追いつかない購読を切るので、外すとき以外に切れたら少し置いて購読し直す。
    ;(async () => {
      while (!controller.signal.aborted) {
        try {
          for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
            try {
              await handle(ctx, event)
            } catch {}
          }
        } catch {}
        if (!controller.signal.aborted) await new Promise((resolve) => setTimeout(resolve, 1000))
      }
    })().catch(() => {})
    return () => {
      controller.abort()
      flush()
    }
  },
}
