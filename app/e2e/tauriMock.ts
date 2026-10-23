import { readFileSync } from "node:fs";
import type { Page } from "@playwright/test";

// @tauri-apps/api の mocks を、ページのスクリプトより先に読ませる。ESM の export を外して大域に置く。
const MOCKS = readFileSync(new URL("../node_modules/@tauri-apps/api/mocks.js", import.meta.url), "utf8").replace(
  /^export \{[^}]*\};?$/m,
  "",
);

/**
 * `responses` にあるコマンドはその値を返し、set_ で始まるものとプラグインの呼び出しは null を返す。
 * それ以外は失敗させ、main.ts が失敗したときの既定値で進むようにする。呼ばれたコマンドは
 * `window.__marimoCalls` に残る。
 */
export async function mockTauri(page: Page, responses: Record<string, unknown>): Promise<void> {
  await page.addInitScript(
    ([mocks, responses]) => {
      (0, eval)(mocks);
      const w = window as unknown as Record<string, unknown> & {
        mockIPC: (cb: (cmd: string, args: unknown) => unknown, options: object) => void;
        mockWindows: (label: string) => void;
      };
      const calls: [string, unknown][] = [];
      w.__marimoCalls = calls;
      w.mockWindows("main");
      w.mockIPC(
        (cmd, args) => {
          calls.push([cmd, args]);
          if (cmd in responses) return responses[cmd];
          if (cmd.startsWith("set_") || cmd.startsWith("plugin:")) return null;
          throw new Error(`not mocked: ${cmd}`);
        },
        { shouldMockEvents: true },
      );
    },
    [MOCKS, responses] as const,
  );
}

/** Rust からの通知と同じ経路で、フロントエンドの listen に届ける。 */
export async function emit(page: Page, event: string, payload: unknown): Promise<void> {
  await page.evaluate(
    ([event, payload]) =>
      (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a: object) => Promise<unknown> } })
        .__TAURI_INTERNALS__.invoke("plugin:event|emit", { event, payload }),
    [event, payload] as const,
  );
}
