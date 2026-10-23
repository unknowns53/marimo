import type { SessionState, Snapshot, Status } from "./types";

/** 優先度の高い順。marimo-core の Status::priority と同じ順で、集約もパネルの状態順もこれに従う。 */
export const STATUS_PRIORITY: readonly Status[] = ["waiting", "error", "working", "done", "idle"];

// ツールごとに session_id を別々に振るので、同じ id が複数のツールにありうる。Claude Code の
// セッションは以前のバージョンと同じキーのままにして、保存してある既読の記録をそのまま使えるようにする。
export function sessionKey(s: Pick<SessionState, "session_id" | "provider">): string {
  return s.provider && s.provider !== "claude" ? `${s.provider}:${s.session_id}` : s.session_id;
}

// トリガーは「どのセッションが、いつから、どの状態か」で見分ける。status_since を含めるので、
// 同じセッションが一度別の状態を経て同じ状態へ戻れば、新しいトリガーになる。
export function triggerKey(s: SessionState): string {
  return `${sessionKey(s)}:${s.status}:${s.status_since}`;
}

/**
 * 利用者が「見た」と示したトリガーの集まり。吹き出しを押して閉じたときと、行を押してセッションへ
 * 移動したときに加える。吹き出しはこれに含まれるトリガーを再び出さず、パネルは完了とエラーの行を折りたたむ。
 * 再起動のたびに同じ完了やエラーをまた知らせないよう、変わるたびに onChange で保存してもらう。
 */
export class Acknowledged {
  private keys = new Set<string>();

  constructor(private readonly onChange: (keys: string[]) => void = () => {}) {}

  // 保存してあった記録を戻す。戻すだけなので保存し直さない。
  restore(keys: readonly string[]): void {
    this.keys = new Set(keys);
  }

  add(key: string): void {
    if (this.keys.has(key)) return;
    this.keys.add(key);
    this.onChange([...this.keys]);
  }

  has(session: SessionState): boolean {
    return this.keys.has(triggerKey(session));
  }

  // 記録は、そのトリガーが続いている間だけ要る。
  prune(snapshot: Snapshot | null): void {
    const live = new Set((snapshot?.sessions ?? []).map(triggerKey));
    let changed = false;
    for (const key of this.keys) {
      if (!live.has(key)) {
        this.keys.delete(key);
        changed = true;
      }
    }
    if (changed) this.onChange([...this.keys]);
  }
}

/**
 * 見たと示せば既読として折りたたむ状態。完了とエラーはターンが終わった後の状態で、次のプロンプトを送るまで
 * 続く。承認待ちは利用者が応じるまでエージェントが止まっているので、見ただけでは折りたたまない。
 */
export const MARKABLE: ReadonlySet<Status> = new Set(["done", "error"]);

export function isRead(s: SessionState, ack: Acknowledged): boolean {
  return MARKABLE.has(s.status) && ack.has(s);
}

// まだ既読にしていない完了とエラーのセッション。
export function unreadMarkable(snapshot: Snapshot | null, ack: Acknowledged): SessionState[] {
  return (snapshot?.sessions ?? []).filter((s) => MARKABLE.has(s.status) && !ack.has(s));
}

/**
 * 見たと示された完了とエラーを待機とみなして集約し直したスナップショットを返す。フックは利用者が
 * 結果を読んだかどうかを知らないので、Rust 側の集約のままだと、読み終えた後も表情が完了やエラーの
 * まま残り、他のセッションが作業中でもその顔に戻らない。
 */
export function withAcknowledged(snapshot: Snapshot, ack: Acknowledged): Snapshot {
  const effective = snapshot.sessions.map((s) => (isRead(s, ack) ? "idle" : s.status));
  const aggregate = STATUS_PRIORITY.find((p) => effective.includes(p)) ?? "idle";
  return aggregate === snapshot.aggregate ? snapshot : { ...snapshot, aggregate };
}
