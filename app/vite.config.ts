import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  // 素材はリポジトリ直下の assets/ を正とし、ビルド時にそのまま dist/ へ写す。
  publicDir: "../assets",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
    // 立ち絵の窓と設定の窓は別のページなので、両方を入口にする。
    rolldownOptions: {
      input: {
        main: fileURLToPath(new URL("index.html", import.meta.url)),
        settings: fileURLToPath(new URL("settings.html", import.meta.url)),
      },
    },
  },
});
