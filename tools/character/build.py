"""立ち絵の素材一式を、生成した基準画像と表情差分から作る。

使い方（リポジトリ直下で実行する）:
    .venv/bin/python tools/character/build.py tools/character/default.json
    .venv/bin/python tools/character/build.py tools/character/default.json --preview preview.png

設定ファイルの states には、状態ごとに次のどれか一つを書く。
    {"source": "base"}                  基準画像をそのまま使う
    {"face": "<画像>"}                  差分画像の顔の領域だけを基準画像へ合成する
    {"face": "<画像>", "onto": "<状態>"}  顔の領域を、基準画像ではなく別の状態の合成結果へ合成する
    {"whole": "<画像>"}                 差分画像を合成せずにそのまま使う（腕を描き足した差分など）
"""

import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from pymatting import estimate_alpha_cf, estimate_foreground_ml
from rembg import new_session, remove

REPO = Path(__file__).resolve().parents[2]


def load_rgb(path):
    return np.asarray(Image.open(path).convert("RGB")).astype(np.float64)


def quantize(rgb):
    return np.clip(rgb + 0.5, 0, 255).astype(np.uint8).astype(np.float64)


def morph(mask, size, grow):
    f = ImageFilter.MaxFilter(size) if grow else ImageFilter.MinFilter(size)
    return np.asarray(Image.fromarray((mask * 255).astype(np.uint8)).filter(f)) > 127


def rembg_alpha(rgb, session):
    img = Image.fromarray(rgb.astype(np.uint8))
    return np.asarray(remove(img, session=session, only_mask=True)).astype(np.float64) / 255


def face_mask(ellipse, a_base):
    # 顔の楕円を基準の人物の輪郭から 15 px 以上内側に制限する。輪郭付近の画素が基準と
    # 同一になるので、基準から作った透明度を合成した差分すべてで共有できる
    h, w = a_base.shape
    face = Image.new("L", (w, h), 0)
    cx, cy, rx, ry = ellipse["cx"], ellipse["cy"], ellipse["rx"], ellipse["ry"]
    ImageDraw.Draw(face).ellipse((cx - rx, cy - ry, cx + rx, cy + ry), fill=255)
    inner = morph(a_base > 0.99, 31, False)
    mask = np.minimum(np.asarray(face), inner * 255).astype(np.uint8)
    blurred = Image.fromarray(mask).filter(ImageFilter.GaussianBlur(10))
    return np.asarray(blurred).astype(np.float64)[..., None] / 255


def matte(rgb, a_model):
    """rembg の透明度から trimap を作り、縁の帯だけを closed-form matting で解き直す。

    rembg の透明度は髪の縁で不透明寄りに出て、縁の画素に背景のグレーが残る。そのまま
    暗い壁紙へ重ねると輪郭が白く浮くので、帯の中の透明度と前景色を pymatting で推定し直す。
    """
    rgb01 = rgb / 255
    # 髪の房の間からのぞく背景は rembg が人物と判定してしまうので、輪郭から 40 px 以内で
    # 色が背景のグレーに近い画素も帯へ入れる
    bg_color = np.median(rgb01[a_model < 0.02], axis=0)
    near_edge = morph(a_model < 0.5, 81, True)
    bg_like = near_edge & (np.linalg.norm(rgb01 - bg_color, axis=2) < 18 / 255)
    fg = morph(a_model > 0.95, 9, False) & ~morph(bg_like, 5, True)
    bg = ~morph(a_model > 0.05, 9, True)
    trimap = np.full(a_model.shape, 0.5)
    trimap[fg] = 1
    trimap[bg] = 0
    alpha = np.clip(estimate_alpha_cf(rgb01, trimap), 0, 1)
    fore = np.clip(estimate_foreground_ml(rgb01, alpha), 0, 1)
    return alpha, fore


def compose(config, session):
    base = load_rgb(REPO / config["base"])
    a_base = rembg_alpha(base, session)
    mask = face_mask(config["face_ellipse"], a_base)

    composed, groups = {}, {}
    pending = dict(config["states"])
    while pending:
        progressed = False
        for state, spec in list(pending.items()):
            onto = spec.get("onto")
            if onto is not None and onto not in composed:
                continue
            if spec.get("source") == "base":
                composed[state], groups[state] = base, "base"
            elif "face" in spec:
                dst = composed[onto] if onto else base
                composed[state] = load_rgb(REPO / spec["face"]) * mask + dst * (1 - mask)
                groups[state] = "base"
            elif "whole" in spec:
                composed[state], groups[state] = load_rgb(REPO / spec["whole"]), state
            else:
                raise ValueError(f"state {state!r} needs one of source, face, whole")
            del pending[state]
            progressed = True
        if not progressed:
            raise ValueError(f"unresolvable onto references: {sorted(pending)}")

    # 合成結果は一度 8 bit に丸めてから透過の処理へ渡す。素材を PNG で保存して読み直す
    # 手順と同じ結果にそろえるためである
    composed = {s: quantize(rgb) for s, rgb in composed.items()}

    mattes = {"base": matte(base, a_base)}
    for state, group in groups.items():
        if group != "base":
            mattes[group] = matte(composed[state], rembg_alpha(composed[state], session))
    return composed, groups, mattes


def render(rgb, alpha, fore, canvas):
    # 不透明な画素は各差分の色を使い、半透明の縁は matting で推定した前景色を使う。
    # 顔の差分は輪郭の帯より内側にしか及ばないので、基準から推定した縁を共有できる
    color = np.where((alpha > 0.999)[..., None], rgb / 255, fore)
    rgba = (np.dstack([color, alpha]) * 255 + 0.5).astype(np.uint8)
    # 縮小時に透明な画素の色が輪郭へ滲まないよう、乗算済みアルファの状態で縮小する
    img = Image.fromarray(rgba, "RGBA").convert("RGBa")
    return img.resize(tuple(canvas), Image.LANCZOS).convert("RGBA")


def preview(images, path):
    # 暗い壁紙と明るい壁紙の両方に重ね、縁に背景のグレーが残っていないかを目で確かめるための一覧
    tw, th = 240, 360
    sheet = Image.new("RGB", (tw * len(images), th * 2))
    for row, bg in enumerate([(40, 44, 52), (250, 248, 244)]):
        for i, img in enumerate(images.values()):
            tile = Image.new("RGBA", (tw, th), bg + (255,))
            tile.alpha_composite(img.resize((tw, th), Image.LANCZOS))
            sheet.paste(tile.convert("RGB"), (i * tw, row * th))
    sheet.save(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("config", type=Path)
    parser.add_argument("--preview", type=Path, help="暗い背景と明るい背景に重ねた一覧画像の保存先")
    args = parser.parse_args()

    config = json.loads(args.config.read_text(encoding="utf-8"))
    session = new_session(config.get("rembg_model", "isnet-anime"))
    composed, groups, mattes = compose(config, session)

    out_dir = REPO / config["out_dir"]
    out_dir.mkdir(parents=True, exist_ok=True)
    images = {}
    for state in config["states"]:
        alpha, fore = mattes[groups[state]]
        images[state] = render(composed[state], alpha, fore, config["canvas"])
        images[state].save(out_dir / f"{state}.png", optimize=True)
        print(f"{state}: {out_dir / f'{state}.png'}")
    if args.preview:
        preview(images, args.preview)
        print(f"preview: {args.preview}")


if __name__ == "__main__":
    main()
