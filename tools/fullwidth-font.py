"""Make crates/zipp-dom/assets/fonts/ZippFullwidth.ttf from DejaVu Sans.

Fullwidth forms (U+FF01-FF5E: "＋", "！", "Ａ") are ASCII drawn in a square cell,
as CJK text sets them. Pages use them as icons ("＋ Add task"), and no font the
browser carries has them, so they drew as boxes. This font has each one as a
composite of DejaVu Sans' ASCII glyph, centred in a cell one em wide, and
nothing else: about 12 KB, where a CJK font is megabytes.

DejaVu Sans' license (Bitstream Vera's) lets a derived font be shipped under
another name, which is why this one is "ZIPP Fullwidth".

    pip install fonttools
    python tools/fullwidth-font.py
"""
import os

from fontTools.subset import Options, Subsetter
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables._g_l_y_f import Glyph, GlyphComponent

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = os.path.join(HERE, "..", "crates", "zipp-dom", "assets", "fonts")
ASCII = range(0x21, 0x7F)
FULLWIDTH = 0xFF01 - 0x21  # added to an ASCII code point gives its fullwidth form

font = TTFont(os.path.join(FONTS, "DejaVuSans.ttf"))
options = Options()
options.layout_features = []
options.hinting = False
options.notdef_outline = True
options.name_IDs = []
subsetter = Subsetter(options)
subsetter.populate(unicodes=ASCII)
subsetter.subset(font)

glyf, hmtx = font["glyf"], font["hmtx"]
em = font["head"].unitsPerEm
best = font.getBestCmap()
added = {}
for code in ASCII:
    source = best[code]
    advance, _ = hmtx[source]
    dx = (em - advance) // 2
    component = GlyphComponent()
    component.glyphName = source
    component.x, component.y = dx, 0
    component.flags = 0x0002 | 0x0004  # ARGS_ARE_XY_VALUES | ROUND_XY_TO_GRID
    glyph = Glyph()
    glyph.numberOfContours = -1
    glyph.components = [component]
    name = "uni%04X" % (code + FULLWIDTH)
    glyf[name] = glyph
    glyph.recalcBounds(glyf)
    hmtx[name] = (em, getattr(glyph, "xMin", 0))
    added[code + FULLWIDTH] = name
# The parts, then the composites, each once (glyf may have listed them as added).
order = list(dict.fromkeys(list(font.getGlyphOrder()) + list(added.values())))
font.setGlyphOrder(order)
glyf.glyphOrder = order

# Only the fullwidth forms: the ASCII glyphs stay, as the composites' parts.
for table in font["cmap"].tables:
    if table.isUnicode():
        table.cmap = {code: name for code, name in table.cmap.items() if code not in ASCII}
        table.cmap.update(added)

names = font["name"]
names.names = []
for name_id, text in ((0, "Derived from DejaVu Sans (Bitstream Vera license; DejaVu changes public domain)."),
                      (1, "ZIPP Fullwidth"), (2, "Regular"), (3, "ZIPP Fullwidth Regular"),
                      (4, "ZIPP Fullwidth"), (5, "Version 1.0"), (6, "ZIPPFullwidth-Regular")):
    names.setName(text, name_id, 3, 1, 0x409)
    names.setName(text, name_id, 1, 0, 0)
os2 = font["OS/2"]
os2.usFirstCharIndex = min(added)
os2.usLastCharIndex = max(added)
os2.recalcUnicodeRanges(font)

out = os.path.join(FONTS, "ZippFullwidth.ttf")
font.save(out)
print(out, os.path.getsize(out), "bytes,", len(added), "fullwidth forms")
