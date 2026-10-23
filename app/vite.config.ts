import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  // 素材はリポジトリ直下の assets/ を正とし、ビルド時にそのまま dist/ へコピーする。
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
    // 立ち絵のウィンドウと設定のウィンドウは別のページなので、両方を入口にする。
    rolldownOptions: {
      input: {
        main: fileURLToPath(new URL("index.html", import.meta.url)),
        settings: fileURLToPath(new URL("settings.html", import.meta.url)),
      },
    },
  },
});
