"""Draw the Zipp browser's icon: a paper airplane on a blue tile.

    python make_icon.py

writes zipp.ico (16 to 256 px, for the .exe and the window) and zipp.png
(256 px, for platforms that take an RGBA icon), beside this script. Drawn at
1024 px and scaled down, so every size is antialiased from the same shapes.
"""

import os
from PIL import Image, ImageDraw, ImageFilter

S = 1024
HERE = os.path.dirname(os.path.abspath(__file__))


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def tile():
    """The rounded square, lit from the top left: zipp.org's accent blue."""
    top_left, bottom_right = (110, 182, 255), (24, 92, 214)
    gradient = Image.new("RGBA", (S, S))
    px = gradient.load()
    for y in range(S):
        for x in range(S):
            px[x, y] = lerp(top_left, bottom_right, (x + y) / (2 * S - 2)) + (255,)
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((40, 40, S - 41, S - 41), radius=230, fill=255)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(gradient, (0, 0), mask)
    return out


# The plane, nose to the upper right: a wing above the fold, the keel below.
NOSE = (838, 196)
WING_TIP = (176, 470)
FOLD = (430, 598)
KEEL = (548, 842)
TUCK = (472, 668)


def plane(layer):
    d = ImageDraw.Draw(layer)
    d.polygon([NOSE, WING_TIP, FOLD], fill=(255, 255, 255, 255))
    d.polygon([NOSE, FOLD, KEEL], fill=(214, 230, 255, 255))
    d.polygon([FOLD, TUCK, KEEL], fill=(150, 188, 245, 255))


def speed_lines(layer):
    """Two streaks off the tail: it zips."""
    d = ImageDraw.Draw(layer)
    for (x0, y0, x1, y1), width in [((150, 660, 330, 590), 38), ((210, 790, 390, 720), 38)]:
        d.line((x0, y0, x1, y1), fill=(255, 255, 255, 170), width=width)
        for x, y in ((x0, y0), (x1, y1)):
            d.ellipse((x - width / 2, y - width / 2, x + width / 2, y + width / 2), fill=(255, 255, 255, 170))


def icon():
    out = tile()
    # A soft shadow under the plane, so it lifts off the tile.
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).polygon(
        [(x + 18, y + 26) for x, y in (NOSE, WING_TIP, FOLD, KEEL)], fill=(8, 40, 110, 120)
    )
    out.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(22)))
    # Drawn on a layer of their own: drawn straight onto the tile, their
    # translucent white replaced its pixels, and the tile showed through.
    streaks = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    speed_lines(streaks)
    out.alpha_composite(streaks)
    plane(out)
    # Keep the shadow inside the tile.
    tile_alpha = tile().getchannel("A")
    out.putalpha(Image.composite(out.getchannel("A"), Image.new("L", (S, S), 0), tile_alpha))
    return out


if __name__ == "__main__":
    big = icon()
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    frames = [big.resize((n, n), Image.LANCZOS) for n in sizes]
    frames[-1].save(os.path.join(HERE, "zipp.png"))
    frames[-1].save(os.path.join(HERE, "zipp.ico"), sizes=[(n, n) for n in sizes], append_images=frames[:-1])
    print("wrote zipp.ico and zipp.png")
