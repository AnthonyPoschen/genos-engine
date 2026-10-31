#!/usr/bin/env python3
"""Score a lighting harness run (scripts/lighting/oracle_run.sh) as compact JSON.

Per scenario (sun-x100, sun-x10, sun-x1, spin, switch, turn, and lamps-spin,
lamps-switch from the lamps profile):

* ref_err_near: near-camera-weighted error against the path-traced reference,
  sum(w |live - ref|) / sum(w ref) in linear luminance over 2x2 blocks, with
  w = 1 / max(distance, 0.5 m) from the depth view. Worst frame and mean.
* ref_err: the same without the weight, for comparison.
* grid_worst / grid_longest_run: the PR #94 checkerboard metric (grid_metric.py)
  over every saved frame of the uninterrupted pass.
* grid_excess_worst / grid_excess_frame / grid_excess_longest_run: for every
  frame step i-1 -> i where the oracle pass has a reference at both frames
  (ORACLE_REF_EVERY_FRAME), the live grid score of the uninterrupted pass minus
  the reference's grid score for the same step, clamped at 0. Real shadow
  motion scores the same in both and cancels; only the checkerboard the live
  cache adds is left. The run counts steps over 0.15, like grid_longest_run.
  Proposed (pending Anthony) to replace grid_worst as the gate, which stays a
  reported number.
* unchanged: the recomputed-but-unchanged counter. Every lighting pass runs
  on every pixel each frame, so `px_share` is the share of 8x8 tiles whose
  picture did not change (|delta luminance| < 1/255) while it was recomputed;
  `refreshes` is the mean cache_entries_refreshed per frame from gi_metrics and
  `refreshes_unchanged` the part of it that landed on unchanged tiles, assuming
  refreshes spread evenly over the screen (a screen-space proxy, not a
  per-texel count).
* switch and lamps-switch only: frames_to_ref, the first frame after the switch whose ref_err_near
  is within 1.1x of the settled frame's.

`--self-test` checks the error, weight and counter on synthetic pictures.
"""
import csv, glob, json, os, re, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import grid_metric  # noqa: E402  (PR #94's metric, reused unchanged)

LUM = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
DEPTH_RANGE = 50.0  # lighting_compose.comp: depth view = 1 - distance / 50


def srgb_to_linear(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def load_rgb(path):
    from PIL import Image
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0


def lin_luma(path):
    return srgb_to_linear(load_rgb(path)) @ LUM


def distance(path):
    v = load_rgb(path)[..., 0]
    return DEPTH_RANGE * (1.0 - v)


def block(a, b=2):
    h, w = a.shape[0] // b * b, a.shape[1] // b * b
    return a[:h, :w].reshape(h // b, b, w // b, b).mean(axis=(1, 3))


def ref_error(live, ref, dist=None):
    """(near-weighted, unweighted) share of the light that is wrong."""
    if live.shape != ref.shape:
        from PIL import Image
        live = np.asarray(Image.fromarray(live).resize(ref.shape[::-1], Image.BILINEAR))
    l, r = block(live), block(ref)
    d = np.abs(l - r)
    plain = float(d.sum() / (r.sum() + 1e-6))
    if dist is None:
        return plain, plain
    if dist.shape != live.shape:
        from PIL import Image
        dist = np.asarray(Image.fromarray(dist).resize(ref.shape[::-1], Image.NEAREST))
    w = 1.0 / np.maximum(block(dist), 0.5)
    near = float((w * d).sum() / ((w * r).sum() + 1e-6))
    return near, plain


def unchanged(frames, tile=8, eps=1.0 / 255.0):
    shares = []
    for a, b in zip(frames, frames[1:]):
        t = grid_metric.tiles(np.abs(b - a), tile)
        shares.append(float((t < eps).mean()))
    return float(np.mean(shares)) if shares else 0.0


def grid_excess(live, refs, tile=8, run_thr=0.15):
    """live: {frame: luminance}, refs: {frame: luminance}. None when no step has both."""
    worst, worst_at, run, longest, steps = 0.0, 0, 0, 0, 0
    for i in sorted(refs):
        if i - 1 not in refs or i not in live or i - 1 not in live:
            run = 0
            continue
        gl, step = grid_metric.grid_score(live[i - 1], live[i], tile)
        gr, _ = grid_metric.grid_score(refs[i - 1], refs[i], tile)
        steps += 1
        # Same rule as grid_metric.analyse: an unchanged frame has no pattern.
        e = 0.0 if step < 0.002 else max(gl - gr, 0.0)
        run = run + 1 if e > run_thr else 0
        longest = max(longest, run)
        if e > worst:
            worst, worst_at = e, i
    if not steps:
        return None
    return {"grid_excess_worst": round(worst, 4), "grid_excess_frame": worst_at,
            "grid_excess_longest_run": longest, "grid_excess_steps": steps}


def gi_refreshes(path):
    if not os.path.exists(path):
        return None
    with open(path) as f:
        rows = list(csv.DictReader(f))
    if not rows:
        return None
    return float(np.mean([float(r["cache_entries_refreshed"]) for r in rows])), \
        float(np.median([float(r["lighting_ms"]) for r in rows]))


def score(out):
    res = {}
    names = sorted({os.path.basename(p) for p in glob.glob(os.path.join(out, "temporal", "*"))}
                   | {os.path.basename(p) for p in glob.glob(os.path.join(out, "oracle", "*"))})
    for n in names:
        s = {}
        tfiles = sorted(glob.glob(os.path.join(out, "temporal", n, "live-*.png")))
        if len(tfiles) >= 2:
            fr = [grid_metric.load(f) for f in tfiles]
            g = grid_metric.analyse(fr, 8, 0.02)
            s.update(frames=g["frames"], grid_worst=g["grid_worst"], grid_worst_frame=g["grid_worst_frame"],
                     grid_longest_run=g["grid_longest_run"])
            lin = [lin_luma(f) for f in tfiles]
            u = {"px_share": round(unchanged(lin), 4)}
            gi = gi_refreshes(os.path.join(out, "gi", f"{n}-frames.csv"))
            if gi:
                u["refreshes"] = round(gi[0])
                u["refreshes_unchanged"] = round(gi[0] * u["px_share"])
                s["lighting_ms_p50_lavapipe"] = round(gi[1], 1)
            s["unchanged"] = u
        per = []
        for lv in sorted(glob.glob(os.path.join(out, "oracle", n, "f*-live.png"))):
            stem = lv[:-len("-live.png")]
            ref = stem + "-reference.png"
            if not os.path.exists(ref):
                continue
            dp = stem + "-depth.png"
            near, plain = ref_error(lin_luma(lv), lin_luma(ref), distance(dp) if os.path.exists(dp) else None)
            per.append((int(re.search(r"f(\d+)$", stem).group(1)), near, plain))
        refs = {int(re.search(r"f(\d+)-reference", p).group(1)): grid_metric.load(p)
                for p in glob.glob(os.path.join(out, "oracle", n, "f*-reference.png"))}
        if len(tfiles) >= 2 and refs:
            live = {int(re.search(r"live-(\d+)", f).group(1)): grid_metric.load(f) for f in tfiles}
            ex = grid_excess(live, refs)
            if ex:
                s.update(ex)
        if per:
            near = [p[1] for p in per]
            worst = max(per, key=lambda p: p[1])
            s.update(ref_frames=len(per), ref_err_near_worst=round(worst[1], 4), ref_err_near_worst_frame=worst[0],
                     ref_err_near_mean=round(float(np.mean(near)), 4),
                     ref_err_mean=round(float(np.mean([p[2] for p in per])), 4))
            if n == "switch" or n == "lamps-switch":
                settled = per[-1][1]
                s["frames_to_ref"] = next((f for f, e, _ in per if e <= 1.1 * settled + 1e-4), per[-1][0])
                s["ref_err_near_by_frame"] = {str(f): round(e, 4) for f, e, _ in per}
        res[n] = s
    return res


def self_test():
    rng = np.random.default_rng(3)
    ref = rng.uniform(0.1, 0.5, (32, 32)).astype(np.float32)
    dist = np.full((32, 32), 20.0, np.float32)
    dist[:, :16] = 1.0  # left half near the camera
    near_bad = ref.copy(); near_bad[:, :16] *= 1.5
    far_bad = ref.copy(); far_bad[:, 16:] *= 1.5
    n1, p1 = ref_error(near_bad, ref, dist)
    n2, p2 = ref_error(far_bad, ref, dist)
    same, _ = ref_error(ref, ref, dist)
    still = [ref, ref, ref]
    moving = [ref, ref * 1.2, ref * 1.44]
    # A sliding edge in both live and reference cancels; a checkerboard only
    # in live does not.
    edge = [np.zeros((48, 48), np.float32) for _ in range(3)]
    for k, e in enumerate(edge):
        e[:, : 12 + 8 * k] = 0.5
    board = [e.copy() for e in edge]
    board[2] = board[2] + np.kron((np.indices((6, 6)).sum(0) % 2) * 0.2, np.ones((8, 8))).astype(np.float32)
    ex_same = grid_excess(dict(enumerate(edge)), dict(enumerate(edge)))
    ex_board = grid_excess(dict(enumerate(board)), dict(enumerate(edge)))
    ok_ex = ex_same["grid_excess_worst"] == 0.0 and ex_board["grid_excess_worst"] > 0.15 \
        and grid_excess({0: ref}, {5: ref}) is None
    ok = (ok_ex and same < 1e-6 and abs(p1 - p2) < 0.05 and n1 > 3 * n2
          and unchanged(still) == 1.0 and unchanged(moving) == 0.0)
    print(json.dumps({"near_err_near_bad": round(n1, 4), "near_err_far_bad": round(n2, 4),
                      "plain": [round(p1, 4), round(p2, 4)]}))
    print("self-test", "ok" if ok else "FAIL")
    return 0 if ok and grid_metric.self_test() == 0 else 1


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        return self_test()
    if len(sys.argv) != 2:
        print(__doc__.split("\n")[0], "\nusage: oracle_metrics.py OUT | --self-test", file=sys.stderr)
        return 2
    print(json.dumps(score(sys.argv[1]), separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
