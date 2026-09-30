import type { SessionState, Snapshot, Status } from "./types";

// marimo-core の集約と同じ順序。
const PRIORITY: Status[] = ["waiting", "error", "working", "done", "idle"];

// ツールごとに session_id を別々に振るので、同じ id が複数のツールにありうる。Claude Code の
// セッションは以前の版と同じ鍵のままにして、保存してある既読の記録をそのまま使えるようにする。
export function sessionKey(s: Pick<SessionState, "session_id" | "provider">): string {
  return s.provider && s.provider !== "claude" ? `${s.provider}:${s.session_id}` : s.session_id;
}

// きっかけは「どのセッションが、いつから、どの状態か」で見分ける。status_since を含めるので、
// 同じセッションが一度別の状態を経て同じ状態へ戻れば、新しいきっかけになる。
export function triggerKey(s: SessionState): string {
  return `${sessionKey(s)}:${s.status}:${s.status_since}`;
}

/**
 * 利用者が「見た」と示したきっかけの集まり。吹き出しを押して閉じたときと、行を押してセッションへ
 * 移動したときに加える。吹き出しはこれに含まれるきっかけを再び出さず、パネルは完了の行を畳む。
 * 再起動のたびに同じ完了をまた知らせないよう、変わるたびに onChange で保存してもらう。
 * 見た時刻は、パネルが押した直後の行をしばらく残すためだけに使うので、保存しない。
 */
export class Acknowledged {
  private keys = new Set<string>();
  private seen = new Map<string, number>();

  constructor(private readonly onChange: (keys: string[]) => void = () => {}) {}

  // 保存してあった記録を戻す。戻すだけなので保存し直さない。
  restore(keys: readonly string[]): void {
    this.keys = new Set(keys);
  }

  add(key: string, at: number = Date.now()): void {
    if (this.keys.has(key)) return;
    this.keys.add(key);
    this.seen.set(key, at);
    this.onChange([...this.keys]);
  }

  has(session: SessionState): boolean {
    return this.keys.has(triggerKey(session));
  }

  // 保存から戻した記録には時刻が無く、undefined を返す。
  seenAt(session: SessionState): number | undefined {
    return this.seen.get(triggerKey(session));
  }

  // 記録は、そのきっかけが続いている間だけ要る。
  prune(snapshot: Snapshot | null): void {
    const live = new Set((snapshot?.sessions ?? []).map(triggerKey));
    let changed = false;
    for (const key of this.keys) {
      if (!live.has(key)) {
        this.keys.delete(key);
        this.seen.delete(key);
        changed = true;
      }
    }
    if (changed) this.onChange([...this.keys]);
  }
}

/**
 * 見たと示された完了を待機とみなして集約し直したスナップショットを返す。フックは利用者が
 * 結果を読んだかどうかを知らないので、Rust 側の集約のままだと、読み終えた後も表情が完了の
 * まま残り、他のセッションが作業中でもその顔に戻らない。
 */
export function withAcknowledged(snapshot: Snapshot, ack: Acknowledged): Snapshot {
  const effective = snapshot.sessions.map((s) =>
    s.status === "done" && ack.has(s) ? "idle" : s.status,
  );
  const aggregate = PRIORITY.find((p) => effective.includes(p)) ?? "idle";
  return aggregate === snapshot.aggregate ? snapshot : { ...snapshot, aggregate };
}
