import type { Status } from "./types";
import { StandingRenderer } from "./standing";

// 絵の描き方はすべてこの口を実装し、main.ts は描き方の違いを知らずに済むようにする。
// 立ち絵、SD、Live2D のどれを使うかは素材フォルダの manifest.json の mode で決める。
export interface CharacterRenderer {
  mount(container: HTMLElement): Promise<void>;
  setStatus(status: Status): void;
  destroy(): void;
}

export interface StandingStateAssets {
  image: string;
  blink?: string;
}

export interface StandingManifest {
  mode: "standing";
  name?: string;
  canvas: { width: number; height: number };
  states: Partial<Record<Status, StandingStateAssets>> & { idle: StandingStateAssets };
}

// Codex Pet 規格（8 列、1 コマ 192×208 px のスプライトシート）の素材を読むための形。
export interface SpriteManifest {
  mode: "sprite";
  name?: string;
  sheet: string;
  columns: number;
  frame: { width: number; height: number };
  states: Partial<Record<Status, { row: number; frames: number; fps?: number }>>;
}

export type Manifest = StandingManifest | SpriteManifest;

export async function loadManifest(baseUrl: string): Promise<Manifest> {
  const res = await fetch(new URL("manifest.json", baseUrl));
  if (!res.ok) throw new Error(`manifest.json: HTTP ${res.status}`);
  return (await res.json()) as Manifest;
}

export function createRenderer(manifest: Manifest, baseUrl: string): CharacterRenderer {
  switch (manifest.mode) {
    case "standing":
      return new StandingRenderer(manifest, baseUrl);
    case "sprite":
      throw new Error("SD モード（sprite）の描画は実装されていません");
  }
}
