import type { HitMask } from "./hitArea";
import type { StandingManifest } from "./manifest";
import type { Status } from "./types";
import { StandingRenderer } from "./standing";

export interface RendererInput {
  status: Status;
  // 集約で選ばれたセッションが今使っているツール。作業内容に合わせて表情を変えるのに使う。
  tool: string | null;
}

// 絵の描き方はすべてこの口を実装し、main.ts は描き方の違いを知らずに済むようにする。
// 立ち絵、SD、Live2D のどれを使うかは素材フォルダの manifest.json の mode で決める。
export interface CharacterRenderer {
  mount(container: HTMLElement): Promise<void>;
  update(input: RendererInput): void;
  setHover(on: boolean): void;
  /** 押されたときの反応。反応の表情を出せたら true を返す。 */
  react(): boolean;
  /** 絵の輪郭が変わったとき（表情の切り替えなど）に呼ばれる。 */
  onShapeChange: (() => void) | null;
  destroy(): void;
  // 窓の透明な部分のクリックを下へ通すために、今の絵のどこが不透明かを返す。
  // mask が null なら element の矩形全体を不透明とみなす。
  hitArea(): { element: HTMLElement; mask: HitMask | null } | null;
}

// Codex Pet 規格（8 列、1 コマ 192×208 px のスプライトシート）の素材を読むための形。
export interface SpriteManifest {
  mode: "sprite";
  name?: string;
  display_name?: string;
  pixelated?: boolean;
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
