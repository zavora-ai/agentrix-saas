"""Static TTF instances of the self-hosted faces in crates/agentrix-ui/static/fonts (variable
WOFF2, SIL OFL), for Flutter, which loads TTF/OTF only. Run from apps/:
    python3 -I tool/fonts.py     (needs fonttools and brotli)
Writes packages/agentrix_kit/fonts/*.ttf and copies OFL.txt beside them."""
import pathlib
import shutil
import sys

from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/agentrix-ui/static/fonts"
OUT = pathlib.Path(__file__).resolve().parents[1] / "packages/agentrix_kit/fonts"

# (source, family file stem, weights, extra fixed axes)
FACES = [
    ("manrope-latin-wght-normal.woff2", "Manrope", [400, 600, 700, 800], {}),
    ("dm-sans-latin-wght-normal.woff2", "DMSans", [400, 500, 600, 700], {"opsz": 14}),
    ("dm-sans-latin-wght-italic.woff2", "DMSans-Italic", [400], {"opsz": 14}),
    ("jetbrains-mono-latin-wght-normal.woff2", "JetBrainsMono", [500], {}),
    ("fraunces-latin-opsz-normal.woff2", "Fraunces", [600], {"opsz": 72}),
    ("cormorant-garamond-latin-wght-normal.woff2", "CormorantGaramond", [600], {}),
    ("newsreader-latin-wght-normal.woff2", "Newsreader", [500], {"opsz": 16}),
]
STATIC = [
    ("libre-caslon-text-latin-400-normal.woff2", "LibreCaslonText-400"),
    ("libre-caslon-text-latin-700-normal.woff2", "LibreCaslonText-700"),
]


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    for src, stem, weights, fixed in FACES:
        for w in weights:
            font = TTFont(SRC / src)
            axes = {a.axisTag for a in font["fvar"].axes} if "fvar" in font else set()
            loc = {k: v for k, v in fixed.items() if k in axes}
            if "wght" in axes:
                loc["wght"] = w
            static = instancer.instantiateVariableFont(font, loc, inplace=False) if loc else font
            static.flavor = None
            static.save(OUT / f"{stem}-{w}.ttf")
            print("wrote", f"{stem}-{w}.ttf")
    for src, stem in STATIC:
        font = TTFont(SRC / src)
        font.flavor = None
        font.save(OUT / f"{stem}.ttf")
        print("wrote", f"{stem}.ttf")
    shutil.copy(SRC / "OFL.txt", OUT / "OFL.txt")
    return 0


if __name__ == "__main__":
    sys.exit(main())
