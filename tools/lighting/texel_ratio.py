#!/usr/bin/env python3
"""Match fine surface-cache texels between two S78S dumps by cache key.

  texel_ratio.py DIR_A DIR_B FRAME

DIR_A / DIR_B hold f{FRAME}_cache.bin and f{FRAME}_keys.bin (GENOS_LIGHTING_DUMP).
The join is the cache key, which is the cell: the same surface is the same slot
when both runs claimed it. A ratio is luminance A / luminance B.

The mean of those ratios is not a brightness figure. A texel B never traced
(sets 0, luminance 0) or a pre-fill set with no firefly cap makes a handful of
ratios enormous while the median stays near 1. The print splits those out.
"""
import sys
import numpy as np

S = 1048576
LW = np.array([0.2126, 0.7152, 0.0722], np.float32)
FINE = 0.13


def load(d, f):
    c = np.fromfile(f"{d}/f{f}_cache.bin", np.float32)
    cu = c.view(np.uint32)
    k = np.fromfile(f"{d}/f{f}_keys.bin", np.uint32)
    i = np.arange(S)
    # slot vec4 2 is irradiance (w = sets); WORLD_AT.z is the shown-frame stamp.
    shown = cu[(3 * S + i) * 4 + 2]
    return dict(
        key=k[:S],
        size=c[(3 * i + 1) * 4 + 3],
        lum=(c[(3 * i + 2) * 4] * LW[0] + c[(3 * i + 2) * 4 + 1] * LW[1] + c[(3 * i + 2) * 4 + 2] * LW[2]),
        sets=c[(3 * i + 2) * 4 + 3],
        shown=shown,
    )


def stats(name, r):
    if r.size == 0:
        print(f"  {name}: 0")
        return
    print(
        f"  {name}: n {r.size} mean {r.mean():.3f} median {np.median(r):.3f} "
        f"p10 {np.percentile(r, 10):.3f} p90 {np.percentile(r, 90):.3f} "
        f">10 {(r > 10).sum()} >100 {(r > 100).sum()}"
    )


def main():
    if len(sys.argv) != 4:
        sys.exit("usage: texel_ratio.py DIR_A DIR_B FRAME")
    a, b = load(sys.argv[1], sys.argv[3]), load(sys.argv[2], sys.argv[3])
    fa = (a["key"] != 0) & (a["size"] <= FINE)
    fb = (b["key"] != 0) & (b["size"] <= FINE)
    print(f"fine live: A {fa.sum()} B {fb.sum()}")
    by_b = {}
    for i in np.nonzero(fb)[0]:
        by_b[int(b["key"][i])] = int(i)
    ia, ib = [], []
    for i in np.nonzero(fa)[0]:
        j = by_b.get(int(a["key"][i]))
        if j is not None:
            ia.append(int(i))
            ib.append(j)
    ia, ib = np.array(ia, np.int64), np.array(ib, np.int64)
    print(f"matched by key: {ia.size}")
    if ia.size == 0:
        return
    la, lb = a["lum"][ia], b["lum"][ib]
    sa, sb = a["sets"][ia], b["sets"][ib]
    shown_a, shown_b = a["shown"][ia] != 0, b["shown"][ib] != 0
    print(
        f"  sets>=1 both {(np.minimum(sa, sb) >= 1).sum()}  "
        f"either sets<1 {((sa < 1) | (sb < 1)).sum()}  "
        f"shown both {(shown_a & shown_b).sum()}  "
        f"either never shown {(~shown_a | ~shown_b).sum()}"
    )
    zero_b = (lb < 1e-4) & (la > 1e-3)
    zero_a = (la < 1e-4) & (lb > 1e-3)
    print(f"  A lit and B ~0: {zero_b.sum()}   B lit and A ~0: {zero_a.sum()}")
    both = (la > 1e-4) & (lb > 1e-4)
    stats("A/B where both lum > 1e-4", la[both] / lb[both])
    lit_sets = both & (sa >= 1) & (sb >= 1) & shown_a & shown_b
    stats("A/B shown, both sets>=1, both lit", la[lit_sets] / lb[lit_sets])
    off = both & (~shown_a | ~shown_b)
    stats("A/B either never shown, both lit", la[off] / lb[off])


if __name__ == "__main__":
    main()
