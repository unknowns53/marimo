import { defineConfig } from "@playwright/test";

// Tauri の窓を開かず、headless の Chromium でフロントエンドだけを描いて確かめる。Rust との
// やりとりは e2e/tauriMock.ts が偽物に差し替える。tauri dev と同時に動かせるよう別の番号で開く。
const PORT = 1430;

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.e2e.ts",
  use: { baseURL: `http://localhost:${PORT}`, browserName: "chromium", headless: true },
  webServer: {
    command: `npx vite --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: false,
  },
});
