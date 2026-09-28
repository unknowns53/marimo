"""アプリのアイコン（水槽の中の顔のあるまりも）を描き出す。

使い方（リポジトリ直下で実行する）:
    .venv/bin/python tools/app_icon.py

macOS の Dock と Finder は、1024 px 四方の画像の中央 824 px を角の丸い板として描く約束の上に並ぶので、
icon.icns はその寸法で板を描き、周りを透明にして板の下に薄い影を落とす。Windows と Linux には
その約束がなく、余白があると他のアプリより小さく見えるので、icon.ico と PNG は同じ絵から板だけを
切り出して画像いっぱいに描く。毛並みは乱数で置いた短い線を数万本重ねて描き、乱数の種を固定して
何度描いても同じ画像になるようにする。
"""

import math
import random
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

REPO = Path(__file__).resolve().parents[1]
ICON_DIR = REPO / "app/src-tauri/icons"

# 細い毛の縁がにじまないよう、2 倍の大きさで描いてから縮める。
K = 2
N = 1024 * K
PLATE = 824 * K
WATER_TOP = (226, 244, 238)
WATER_BOTTOM = (178, 219, 208)
MOSS_DARK = (28, 72, 30)
MOSS_LIGHT = (150, 196, 92)
# 光は左上の手前から当てる。
LIGHT = np.array([-0.55, -0.65, 0.52]) / np.linalg.norm([-0.55, -0.65, 0.52])


def plate_mask():
    """角の丸い板を、指数 5 の超楕円で近似した 0 から 1 の不透明度で返す。"""
    c = (N - 1) / 2
    a = PLATE / 2
    y, x = np.mgrid[0:N, 0:N]
    v = np.abs((x - c) / a) ** 5 + np.abs((y - c) / a) ** 5
    return np.clip((1.0 - v) * a / 2.5 + 0.5, 0, 1)


def water(mask):
    t = np.linspace(0, 1, N)[:, None, None]
    rgb = np.array(WATER_TOP, float) * (1 - t) + np.array(WATER_BOTTOM, float) * t
    rgb = np.broadcast_to(rgb, (N, N, 3))
    return Image.fromarray(np.dstack([rgb, mask * 255]).astype(np.uint8), "RGBA")


def moss(t):
    return tuple(int(MOSS_DARK[i] + (MOSS_LIGHT[i] - MOSS_DARK[i]) * t) for i in range(3))


def brightness(x, y, z):
    return 0.1 + 0.9 * max(0.0, x * LIGHT[0] + y * LIGHT[1] + z * LIGHT[2])


def marimo(layer, cx, cy, r, strands=26000):
    # 毛の隙間から背景が透けないよう、先に陰影を付けた球を塗っておく。
    y, x = np.mgrid[0:N, 0:N]
    dx, dy = (x - cx) / r, (y - cy) / r
    d2 = dx * dx + dy * dy
    z = np.sqrt(np.clip(1 - d2, 0, 1))
    lam = np.clip(dx * LIGHT[0] + dy * LIGHT[1] + z * LIGHT[2], 0, 1)
    t = (0.15 + 0.85 * lam)[..., None]
    rgb = np.array(MOSS_DARK, float) * (1 - t) + np.array(MOSS_LIGHT, float) * t
    ball = np.dstack([rgb, (d2 < 1) * 255]).astype(np.uint8)
    layer.alpha_composite(Image.fromarray(ball, "RGBA"))

    rnd = random.Random(1)
    roots = []
    for _ in range(strands):
        u = rnd.random() * 2 * math.pi
        rr = math.sqrt(rnd.random()) ** 0.8
        px, py = rr * math.cos(u), rr * math.sin(u)
        roots.append((math.sqrt(max(0.0, 1 - px * px - py * py)), px, py))
    # 奥（縁）の毛から手前の毛へ順に重ねる。
    roots.sort()
    draw = ImageDraw.Draw(layer)
    width = max(1, int(r * 0.009))
    for pz, px, py in roots:
        tone = min(1.0, max(0.0, brightness(px, py, pz) + rnd.uniform(-0.18, 0.18)))
        # 縁に近い毛ほど外向きに揃えて、輪郭から毛羽が立つようにする。
        ang = math.atan2(py, px) + rnd.uniform(-1.2, 1.2) * (0.3 + pz)
        length = r * rnd.uniform(0.025, 0.06) * (1.6 - 0.6 * pz)
        sx, sy = cx + px * r, cy + py * r
        ex, ey = sx + math.cos(ang) * length, sy + math.sin(ang) * length
        draw.line([sx, sy, ex, ey], fill=moss(tone) + (255,), width=width)


def blurred(shape, box, alpha, radius, color):
    mask = Image.new("L", (N, N), 0)
    getattr(ImageDraw.Draw(mask), shape)(box, fill=alpha)
    im = Image.new("RGBA", (N, N), color + (0,))
    im.putalpha(mask.filter(ImageFilter.GaussianBlur(radius)))
    return im


def bubbles(layer):
    draw = ImageDraw.Draw(layer)
    for bx, by, br in [(700, 230, 26), (748, 300, 15), (722, 170, 10)]:
        bx, by, br = bx * K, by * K, br * K
        draw.ellipse([bx - br, by - br, bx + br, by + br], outline=(255, 255, 255, 200), width=max(2, int(br * 0.22)))
        draw.ellipse([bx - br * 0.45, by - br * 0.55, bx - br * 0.05, by - br * 0.15], fill=(255, 255, 255, 220))


def face(layer, cx, cy, r):
    ink = (30, 45, 25, 255)
    draw = ImageDraw.Draw(layer)
    ex, ey, er = r * 0.3, cy + r * 0.08, r * 0.075
    for side in (-1, 1):
        x = cx + side * ex
        draw.ellipse([x - er, ey - er * 1.25, x + er, ey + er * 1.25], fill=ink)
        draw.ellipse([x - er * 0.35, ey - er * 0.9, x + er * 0.25, ey - er * 0.25], fill=(255, 255, 255, 255))
    draw.arc([cx - r * 0.12, ey + r * 0.05, cx + r * 0.12, ey + r * 0.25], 20, 160, fill=ink, width=int(r * 0.04))
    for side in (-1, 1):
        x = cx + side * r * 0.5
        layer.alpha_composite(blurred("ellipse", [x - r * 0.1, ey + r * 0.12, x + r * 0.1, ey + r * 0.24], 110, r * 0.03, (240, 130, 130)))


def artwork():
    """板と中身を N 四方に描き、板の外は透明のまま返す。"""
    mask = plate_mask()
    cx, cy, r = N / 2, N / 2 + 20 * K, 285 * K
    layer = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    layer.alpha_composite(blurred("ellipse", [cx - r * 0.85, cy + r * 0.82, cx + r * 0.85, cy + r * 1.08], 90, r * 0.08, (20, 50, 40)))
    marimo(layer, cx, cy, r)
    layer.alpha_composite(blurred("ellipse", [cx - r * 0.62, cy - r * 0.66, cx - r * 0.05, cy - r * 0.2], 90, r * 0.12, (255, 255, 240)))
    face(layer, cx, cy, r)
    bubbles(layer)
    clipped = np.asarray(layer).copy()
    clipped[..., 3] = (clipped[..., 3] * mask).astype(np.uint8)
    art = water(mask)
    art.alpha_composite(Image.fromarray(clipped, "RGBA"))
    return art


def macos_icon(art):
    shade = Image.fromarray((plate_mask() * 80).astype(np.uint8), "L")
    shade = shade.transform((N, N), Image.AFFINE, (1, 0, 0, 0, 1, -10 * K)).filter(ImageFilter.GaussianBlur(18 * K))
    canvas = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    canvas.putalpha(shade)
    canvas.alpha_composite(art)
    return canvas.resize((1024, 1024), Image.LANCZOS)


def full_bleed_icon(art):
    m = (N - PLATE) // 2
    return art.crop((m, m, N - m, N - m)).resize((1024, 1024), Image.LANCZOS)


def main():
    art = artwork()
    mac = macos_icon(art)
    full = full_bleed_icon(art)
    outputs = {
        "icon.icns": (mac, {}),
        "icon.ico": (full, {"sizes": [(s, s) for s in (16, 24, 32, 48, 64, 128, 256)]}),
        "icon.png": (full.resize((512, 512), Image.LANCZOS), {}),
        "32x32.png": (full.resize((32, 32), Image.LANCZOS), {}),
        "128x128.png": (full.resize((128, 128), Image.LANCZOS), {}),
        "128x128@2x.png": (full.resize((256, 256), Image.LANCZOS), {}),
    }
    for name, (img, opts) in outputs.items():
        img.save(ICON_DIR / name, **opts)
        print(f"{name}: {ICON_DIR / name}")


if __name__ == "__main__":
    main()
