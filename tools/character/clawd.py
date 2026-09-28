"""Clawd（Claude Code のマスコット）のドット絵の素材一式を描き出す。

使い方（リポジトリ直下で実行する）:
    .venv/bin/python tools/character/clawd.py
    .venv/bin/python tools/character/clawd.py --preview preview.png

形は Claude Code の起動時のロゴ（ブロック文字で描いたもの）を、1 文字を 2×2 の画素に読み解いたものに従う。
ロゴの 1 画素は端末の文字の升目の 4 分の 1 で、横 1 に対して縦がほぼ 2 あるので、横 2 マス、縦 4 マスで描き、
端末で見るときと同じ縦横比に保つ。

キャンバスは 60×30 マスで、1 マスを 6 px にした 360×180 px の PNG を書き出す。立ち絵の枠は倍率 1.0 で幅
180 CSS px なので、1 マスは 3 CSS px になり、等倍の画面でも 2 倍の画面でも整数の画素に載る。
"""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw

REPO = Path(__file__).resolve().parents[2]
OUT_DIR = REPO / "assets/character/clawd"

COLS, ROWS = 60, 30
CELL = 6
PX_W, PX_H = 2, 4

PALETTE = {
    "#": (215, 119, 87),
    "E": (30, 30, 30),
    "k": (30, 30, 30),
    "p": (244, 160, 170),
    "y": (250, 204, 72),
    "a": (245, 166, 35),
    "b": (110, 182, 236),
    "w": (250, 248, 242),
    "g": (150, 150, 162),
    "z": (160, 170, 205),
}

LOGO = [
    "...############...",
    "...##.######.##...",
    ".################.",
    "...############...",
    "....#.#....#.#....",
]
EYE_HOLES = [(5, 1), (12, 1)]

# 腕はロゴの画素で置く。下ろした形がロゴそのもので、ほかは同じ太さのまま位置だけを変える。
ARMS = {
    "down": [(1, 2), (2, 2), (15, 2), (16, 2)],
    "lift": [(1, 1), (2, 1), (15, 1), (16, 1)],
    "left_up": [(2, 1), (1, 1), (1, 0), (15, 2), (16, 2)],
    "right_up": [(1, 2), (2, 2), (15, 1), (16, 1), (16, 0)],
    "both_up": [(2, 1), (1, 1), (1, 0), (15, 1), (16, 1), (16, 0)],
    "high": [(2, 1), (1, 1), (1, 0), (1, -1), (15, 1), (16, 1), (16, 0), (16, -1)],
}

# 顔は体の上の 20×8 マスに重ねる。"." のマスは体の色のまま残し、"E" のマスは瞬きで閉じる目として、
# "k" のマスは瞬きでも閉じない濃い線として塗る。
FACE_X, FACE_Y = 8, 2


def face(*rows):
    return [r.ljust(20, ".") for r in rows] + ["." * 20] * (8 - len(rows))


OPEN = face("", "", "..EE............EE", "..EE............EE", "..EE............EE", "..EE............EE")
DOWN = face("", "", "", "..EE............EE", "..EE............EE", "..EE............EE", "..EE............EE")
READ = face("", "", "", "", "..EE............EE", "..EE............EE", "..EE............EE", "..EE............EE")
UP = face("", "..EE............EE", "..EE............EE", "..EE............EE", "..EE............EE")
UP_RIGHT = face("...EE............EE", "...EE............EE", "...EE............EE", "...EE............EE")
LEFT = face("", "", "EE............EE", "EE............EE", "EE............EE", "EE............EE")
RIGHT = face("", "", "....EE............EE", "....EE............EE", "....EE............EE", "....EE............EE")
FOCUS = face("", ".kk..............kk", "...k............k", "", "..EE............EE", "..EE............EE")
WINK = face("", "", "..EE", "..EE.............kk", "..EE............k..k", "..EE")
HAPPY = face("", "", "..kk............kk", ".k..k..........k..k")
SHY = face("", "", "..kk............kk", ".k..k..........k..k", "", "ppp..............ppp", "ppp..............ppp")
CROSS = face("", ".k..k..........k..k", "..kk............kk", "..kk............kk", ".k..k..........k..k")
SLEEP = face("", "", "", "", ".k..k..........k..k", "..kk............kk")
CLOSED = face("", "", "", "", ".kkkk..........kkkk")

QUESTION = [".###.", "#...#", "....#", "..##.", "..#..", ".....", "..#.."]
EXCLAIM = ["aa", "aa", "aa", "aa", "..", "aa"]
DOTS = ["##.##.##", "##.##.##"]
SPARKLE = ["..y..", "..y..", "yy.yy", "..y..", "..y.."]
TWINKLE = [".y.", "yyy", ".y."]
SWEAT = ["..b..", "..b..", ".bbb.", "bbbbb", "bbbbb", ".bbb."]
BIG_Z = ["zzzzz", "....z", "...z.", "..z..", ".z...", "zzzzz"]
SMALL_Z = ["zzzz", "...z", "..z.", ".z..", "zzzz"]
PAGE = ["wwwwwww", "wgggwww", "wwwwwww", "wggggww", "wwwwwww", "wgggwww", "wwwwwww"]
TICK = ["g", "g"]


def state(face_rows, arms="down", hop=0, extras=()):
    return {"face": face_rows, "arms": arms, "hop": hop, "extras": extras}


# 飾りの位置はキャンバスのマスで書く。Clawd の体はキャンバスの x 14〜45 にあり、頭の上端は y 10 にある。
STATES = {
    "idle": state(OPEN),
    "working": state(DOWN, extras=[(TICK, 11, 17), (TICK, 48, 17)]),
    "working_focus": state(FOCUS, extras=[(TICK, 11, 17), (TICK, 48, 17)]),
    "working_read": state(READ, extras=[(PAGE, 27, 20)]),
    "working_curious": state(UP, extras=[(QUESTION, 47, 1)]),
    "working_think": state(UP_RIGHT, extras=[(DOTS, 46, 5)]),
    "working_delegate": state(WINK, arms="right_up"),
    "waiting": state(OPEN, arms="left_up", extras=[(EXCLAIM, 47, 2), (TICK, 9, 8), (TICK, 7, 12)]),
    "done": state(HAPPY, arms="both_up", hop=2, extras=[(SPARKLE, 4, 3), (TWINKLE, 51, 1), (SPARKLE, 52, 12)]),
    "error": state(CROSS, extras=[(SWEAT, 47, 5)]),
    "idle_look_left": state(LEFT),
    "idle_look_right": state(RIGHT),
    "idle_hop": state(OPEN, arms="lift", hop=4),
    "idle_sleep": state(SLEEP, extras=[(SMALL_Z, 46, 5), (BIG_Z, 51, 0)]),
    "idle_stretch": state(CLOSED, arms="high"),
    "react_shy": state(SHY),
}
BLINKS = ["idle", "working", "working_focus", "working_read", "working_curious", "working_think", "working_delegate"]


def blink(face_rows):
    """閉じた目の線が開いた目の高さの中ほどに来るよう、目の塊ごとに中ほどの 1 行だけを残す。

    目の形は表情ごとに違うので、行を決め打ちにせず塊を探して決める。
    """
    cells = {(x, y) for y, row in enumerate(face_rows) for x, c in enumerate(row) if c == "E"}
    keep = set()
    while cells:
        stack = [cells.pop()]
        blob = set(stack)
        while stack:
            x, y = stack.pop()
            for n in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                if n in cells:
                    cells.remove(n)
                    blob.add(n)
                    stack.append(n)
        top = min(y for _, y in blob)
        bottom = max(y for _, y in blob)
        keep |= {(x, y) for x, y in blob if y == top + (bottom - top + 1) // 2}
    return [
        "".join("k" if (x, y) in keep else ("." if c == "E" else c) for x, c in enumerate(row))
        for y, row in enumerate(face_rows)
    ]


def draw(spec):
    grid = [["."] * COLS for _ in range(ROWS)]
    ox = (COLS - len(LOGO[0]) * PX_W) // 2
    oy = ROWS - len(LOGO) * PX_H - spec["hop"]

    def paint(x, y, c):
        if c != "." and 0 <= x < COLS and 0 <= y < ROWS:
            grid[y][x] = c

    def logo_pixel(lx, ly):
        for dy in range(PX_H):
            for dx in range(PX_W):
                paint(ox + lx * PX_W + dx, oy + ly * PX_H + dy, "#")

    for ly, row in enumerate(LOGO):
        for lx, c in enumerate(row):
            if c == "#" and (lx, ly) not in ARMS["down"]:
                logo_pixel(lx, ly)
    for lx, ly in EYE_HOLES + ARMS[spec["arms"]]:
        logo_pixel(lx, ly)
    for y, row in enumerate(spec["face"]):
        for x, c in enumerate(row):
            paint(ox + FACE_X + x, oy + FACE_Y + y, c)
    for sprite, sx, sy in spec["extras"]:
        for y, row in enumerate(sprite):
            for x, c in enumerate(row):
                paint(sx + x, sy + y, c)

    img = Image.new("RGBA", (COLS, ROWS), (0, 0, 0, 0))
    for y, row in enumerate(grid):
        for x, c in enumerate(row):
            if c != ".":
                img.putpixel((x, y), PALETTE[c] + (255,))
    return img.resize((COLS * CELL, ROWS * CELL), Image.NEAREST)


def build():
    images = {}
    for name, spec in STATES.items():
        images[name] = draw(spec)
        if name in BLINKS:
            images[f"{name}_blink"] = draw({**spec, "face": blink(spec["face"])})
    return images


def preview(images, path):
    # 暗い壁紙と明るい壁紙の両方に重ね、飾りの色がどちらでも見えるかを目で確かめる。
    tw, th, label = COLS * CELL, ROWS * CELL, 18
    per_row = 5
    lines = (len(images) + per_row - 1) // per_row
    sheet = Image.new("RGB", (tw * per_row, (th + label) * lines * 2), (128, 128, 128))
    pen = ImageDraw.Draw(sheet)
    for block, (bg, fg) in enumerate([((40, 44, 52), (230, 230, 230)), ((250, 248, 244), (40, 40, 40))]):
        for i, (name, img) in enumerate(images.items()):
            x = (i % per_row) * tw
            y = (block * lines + i // per_row) * (th + label)
            tile = Image.new("RGBA", (tw, th + label), bg + (255,))
            tile.alpha_composite(img, (0, label))
            sheet.paste(tile.convert("RGB"), (x, y))
            pen.text((x + 4, y + 3), name, fill=fg)
            pen.rectangle([x, y, x + tw - 1, y + th + label - 1], outline=(100, 100, 100))
    sheet.save(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--preview", type=Path, help="暗い背景と明るい背景に重ねた一覧画像の保存先")
    args = parser.parse_args()

    images = build()
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for name, img in images.items():
        img.save(OUT_DIR / f"{name}.png", optimize=True)
        print(f"{name}: {OUT_DIR / f'{name}.png'}")
    if args.preview:
        preview(images, args.preview)
        print(f"preview: {args.preview}")


if __name__ == "__main__":
    main()
