"""Draw the ZIPP Browser application icon matching ZIPP's official brand mark
(`docs/assets/zipp-bolt.svg`): the angular purple-to-cyan lightning bolt on
ZIPP's dark violet-to-teal rounded square.

    python make_icon.py

writes zipp.ico (16 to 256 px, for the .exe and the Windows taskbar/title bar)
and zipp.png (256 px, for platforms that take an RGBA icon), beside this script.
Drawn at 1024 px and scaled down with Lanczos filtering so every size is crisp.
"""

import os
from PIL import Image, ImageDraw, ImageFilter

S = 1024
HERE = os.path.dirname(os.path.abspath(__file__))


def lerp_stop(stops, t):
    t = max(0.0, min(1.0, t))
    for i in range(len(stops) - 1):
        t0, c0 = stops[i]
        t1, c1 = stops[i + 1]
        if t <= t1:
            u = 0.0 if t1 == t0 else (t - t0) / (t1 - t0)
            return tuple(round(a + (b - a) * u) for a, b in zip(c0, c1))
    return stops[-1][1]


def diagonal_gradient(size, stops):
    """Create an RGBA image filled with a top-left to bottom-right linear gradient."""
    img = Image.new("RGBA", (size, size))
    px = img.load()
    denom = float(2 * size - 2)
    lut = [lerp_stop(stops, i / denom) + (255,) for i in range(2 * size - 1)]
    for y in range(size):
        row_base = y
        for x in range(size):
            px[x, y] = lut[x + row_base]
    return img


# Official ZIPP background stops (#0b1020 -> #24124d @ 55% -> #083b4c)
BG_STOPS = [
    (0.00, (11, 16, 32)),
    (0.55, (36, 18, 77)),
    (1.00, (8, 59, 76)),
]

# Official ZIPP lightning bolt stops (#c4b5fd -> #7c3aed @ 50% -> #22d3ee)
BOLT_STOPS = [
    (0.00, (196, 181, 253)),
    (0.50, (124, 58, 237)),
    (1.00, (34, 211, 238)),
]

# Exact path from zipp-bolt.svg:
#   d="M92 0 0 122h58l-25 86L142 69h-62z" transform="translate(57 24)"
# in 256x256 space, scaled 4x to 1024x1024.
BOLT_SVG_256 = [
    (92 + 57, 0 + 24),
    (0 + 57, 122 + 24),
    (58 + 57, 122 + 24),
    (33 + 57, 208 + 24),
    (142 + 57, 69 + 24),
    (80 + 57, 69 + 24),
]
BOLT_POLY = [(x * 4, y * 4) for x, y in BOLT_SVG_256]


def tile_mask():
    mask = Image.new("L", (S, S), 0)
    # rx=56 in 256x256 -> radius=224 in 1024x1024, inset 16px for crisp edges.
    ImageDraw.Draw(mask).rounded_rectangle((16, 16, S - 17, S - 17), radius=224, fill=255)
    return mask


def bolt_layer():
    """Render the gradient-filled official ZIPP lightning bolt at 1024x1024."""
    xs = [p[0] for p in BOLT_POLY]
    ys = [p[1] for p in BOLT_POLY]
    min_x, max_x = min(xs), max(xs)
    min_y, max_y = min(ys), max(ys)
    bw = max_x - min_x
    bh = max_y - min_y

    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).polygon(BOLT_POLY, fill=255)

    grad = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    px = grad.load()
    denom = float(bw + bh)
    for y in range(min_y, max_y + 1):
        dy = y - min_y
        for x in range(min_x, max_x + 1):
            t = ((x - min_x) + dy) / denom
            px[x, y] = lerp_stop(BOLT_STOPS, t) + (255,)

    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(grad, (0, 0), mask)
    return out


def icon():
    tmask = tile_mask()
    bg = diagonal_gradient(S, BG_STOPS)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(bg, (0, 0), tmask)

    # Subtle radial aura behind the bolt (echoing zipp-hero / zipp-bolt glow)
    bolt = bolt_layer()
    glow = bolt.filter(ImageFilter.GaussianBlur(36))
    out.alpha_composite(glow)
    out.alpha_composite(bolt)

    # Subtle inner border highlight so the dark tile separates cleanly on dark taskbars
    border = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(border).rounded_rectangle(
        (22, 22, S - 23, S - 23),
        radius=218,
        outline=(167, 139, 250, 72),
        width=10,
    )
    out.alpha_composite(border)

    # Clip everything strictly to the rounded tile silhouette
    out.putalpha(Image.composite(out.getchannel("A"), Image.new("L", (S, S), 0), tmask))
    return out


if __name__ == "__main__":
    big = icon()
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    frames = [big.resize((n, n), Image.LANCZOS) for n in sizes]
    frames[-1].save(os.path.join(HERE, "zipp.png"))
    frames[-1].save(
        os.path.join(HERE, "zipp.ico"),
        sizes=[(n, n) for n in sizes],
        append_images=frames[:-1],
    )
    print("wrote zipp.ico and zipp.png (official ZIPP lightning bolt)")
