#!/usr/bin/env python3
"""No XOR of products in shader hashes (S86).

`x * p1 ^ y * p2 ^ ...` (the Teschner spatial hash) gives sign-mirrored inputs
the same key: a shadow cell west of a wall read as the cell east of it. Key a
cell with `hash_cell3` (lighting_common.glsl) and chain other inputs through
`lighting_hash(a ^ lighting_hash(b))`. `lint/fail/shader_xor_hash.comp` must
trip; `lint/pass` must stay clean.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

EXTS = {".glsl", ".comp", ".vert", ".frag"}
BIG = r"(?:0x[0-9a-fA-F]{5,}|\d{5,})u?"
XOR_OF_PRODUCTS = re.compile(rf"\*\s*{BIG}\s*\)?\s*\^[^;]*?\*\s*{BIG}")


def root() -> Path:
    return Path(__file__).resolve().parents[1]


def hits(path: Path) -> list[str]:
    out = []
    for no, line in enumerate(path.read_text().splitlines(), 1):
        code = line.split("//", 1)[0]
        if XOR_OF_PRODUCTS.search(code):
            out.append(f"{path}:{no}: XOR of products hash; use hash_cell3 or chain lighting_hash")
    return out


def shaders(base: Path) -> list[Path]:
    return [
        p
        for p in sorted(base.rglob("*"))
        if p.suffix in EXTS and "target" not in p.parts and ".git" not in p.parts and "lint" not in p.relative_to(base).parts[:1]
    ]


def main() -> int:
    base = root()
    if not hits(base / "lint/fail/shader_xor_hash.comp"):
        print("lint-shader-hashes: lint/fail/shader_xor_hash.comp did not trip")
        return 1
    found = [h for p in shaders(base) + sorted((base / "lint/pass").glob("*")) if p.suffix in EXTS for h in hits(p)]
    for h in found:
        print(h)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
