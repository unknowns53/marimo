"""メニューバーと通知領域のアイコン（まりもの影絵）を描き出す。

使い方（リポジトリ直下で実行する）:
    .venv/bin/python tools/tray_icon.py

macOS のメニューバーは画像の高さを 18 pt に縮めて置くので、2 倍の画面で等倍になる 36 px 四方で描く。
ふだんの macOS 用は黒だけのテンプレート画像にして、明るいメニューバーでも暗いメニューバーでも macOS に
塗り分けさせる。Windows の通知領域はテンプレート画像を知らず、黒の影絵は暗いタスクバーに沈むので、
ふだんの絵を苔の緑で塗った画像を別に作る。承認待ちとエラーの絵は色で気付かせたいので、どちらの OS でも
緑の玉に色の付いた印を載せ、テンプレート画像にはしない。
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

REPO = Path(__file__).resolve().parents[1]
ICON_DIR = REPO / "app/src-tauri/icons"

N = 36
MOSS = (92, 150, 80)
WAITING = (245, 166, 35)
ERROR = (230, 70, 70)
WHITE = (255, 255, 255, 255)


def marimo_mask(r=13.0, spikes=32, amp=1.4):
    """影絵でもまりもと分かるよう、縁に細かな毛羽を立てる。

    36 px の小ささでは縁のぎざぎざが目立つので、縁の 1 px だけを濃淡で滑らかにする。
    """
    im = Image.new("L", (N, N), 0)
    px = im.load()
    c = (N - 1) / 2
    for y in range(N):
        for x in range(N):
            dx, dy = x - c, y - c
            d = math.hypot(dx, dy)
            a = math.atan2(dy, dx)
            edge = r + amp * (0.5 + 0.5 * math.sin(spikes * a))
            if d <= edge - 0.5:
                px[x, y] = 255
            elif d <= edge + 0.5:
                px[x, y] = int(255 * (edge + 0.5 - d))
    # 左上の光沢を抜いて、平たい円でなく玉に見せる。
    ImageDraw.Draw(im).arc([8, 8, 22, 22], 180, 260, fill=0, width=2)
    return im


def fill(mask, color):
    im = Image.new("RGBA", (N, N), color + (0,))
    im.putalpha(mask)
    return im


def badge(im, color):
    im = im.copy()
    d = ImageDraw.Draw(im)
    d.ellipse([22, 0, 35, 13], fill=WHITE)
    d.ellipse([23, 1, 34, 12], fill=color + (255,))
    d.rectangle([28, 3, 29, 8], fill=WHITE)
    d.rectangle([28, 10, 29, 10], fill=WHITE)
    return im


def tray_icons():
    mask = marimo_mask()
    moss = fill(mask, MOSS)
    return {
        "tray-template.png": fill(mask, (0, 0, 0)),
        "tray-color.png": moss,
        "tray-waiting.png": badge(moss, WAITING),
        "tray-error.png": badge(moss, ERROR),
    }


def main():
    for name, img in tray_icons().items():
        img.save(ICON_DIR / name, optimize=True)
        print(f"{name}: {ICON_DIR / name}")


if __name__ == "__main__":
    main()
