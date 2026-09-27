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
  },
});
