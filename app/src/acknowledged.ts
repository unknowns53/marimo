import type { SessionState, Snapshot } from "./types";

// きっかけは「どのセッションが、いつから、どの状態か」で見分ける。status_since を含めるので、
// 同じセッションが一度別の状態を経て同じ状態へ戻れば、新しいきっかけになる。
export function triggerKey(s: SessionState): string {
  return `${s.session_id}:${s.status}:${s.status_since}`;
}

/**
 * 利用者が「見た」と示したきっかけの集まり。吹き出しを押して閉じたときと、行を押してセッションへ
 * 移動したときに加える。吹き出しはこれに含まれるきっかけを再び出さず、パネルは完了の行を畳む。
 */
export class Acknowledged {
  private keys = new Set<string>();

  add(key: string): void {
    this.keys.add(key);
  }

  has(session: SessionState): boolean {
    return this.keys.has(triggerKey(session));
  }

  // 記録は、そのきっかけが続いている間だけ要る。
  prune(snapshot: Snapshot | null): void {
    const live = new Set((snapshot?.sessions ?? []).map(triggerKey));
    for (const key of this.keys) {
      if (!live.has(key)) this.keys.delete(key);
    }
  }
}
