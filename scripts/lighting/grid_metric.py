#!/usr/bin/env python3
"""Checkerboard and convergence metrics for a lighting frame sequence.

Reads a directory of PNG frames (the `frames_dir` of the debug `flicker` step)
and reports, over the whole sequence (worst case, never an average):

* grid score: how much of each frame-to-frame change sits in blocky tiles that
  disagree with their neighbours. A smooth, global change (the sun moving and
  every surface following it) scores near 0. Some tiles updated and their
  neighbours not (the cache checkerboard) scores high.
* grid longest run: the longest stretch of frames whose grid score stays over
  0.15, the sweep's time-to-converge (how long a checkerboard stays visible).
* converge frames: for a step change (spin, sun stop) the frames until every
  tile is within `--tol` of the final frame and stays there.

Exit code 1 when `--max-grid` or `--max-converge` is given and exceeded, so
agents and CI can gate on it. `--self-test` checks the detector on synthetic
frames and needs no engine.
"""
import argparse, glob, json, os, sys
import numpy as np

LUM = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)


def load(path):
    from PIL import Image
    a = np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0
    return a @ LUM


def tiles(img, t):
    h, w = img.shape[0] // t * t, img.shape[1] // t * t
    return img[:h, :w].reshape(h // t, t, w // t, t).mean(axis=(1, 3))


def grid_score(prev, cur, tile=8):
    """Share of the change that is tile-to-tile disagreement (0..1)."""
    d = tiles(cur - prev, tile)
    if d.shape[0] < 3 or d.shape[1] < 3:
        return 0.0, 0.0
    p = np.pad(d, 1, mode="edge")
    nb = np.stack([p[1 + dy:p.shape[0] - 1 + dy, 1 + dx:p.shape[1] - 1 + dx]
                   for dy in (-1, 0, 1) for dx in (-1, 0, 1) if dy or dx])
    rough = np.abs(d - np.median(nb, axis=0))
    total = np.abs(d).mean() + 1.0e-3
    # Edges of real shadows move too: count only tiles whose step beats the
    # neighbours by 2x, and normalise by the change of the frame.
    return float(rough.mean() / total), float(np.abs(d).mean())


def converge(frames, tile=8, tol=0.02):
    final = tiles(frames[-1], tile)
    last_bad = -1
    for i, f in enumerate(frames):
        if np.abs(tiles(f, tile) - final).max() > tol:
            last_bad = i
    return last_bad + 1


def analyse(frames, tile, tol, stride=1, run_thr=0.15):
    worst, worst_at, step_at_worst = 0.0, 0, 0.0
    run = longest = 0
    for i in range(stride, len(frames), stride):
        g, step = grid_score(frames[i - stride], frames[i], tile)
        # A frame with almost no change has no pattern to see.
        if step < 0.002:
            run = 0
            continue
        run = run + stride if g > run_thr else 0
        longest = max(longest, run)
        if g > worst:
            worst, worst_at, step_at_worst = g, i, step
    return {"frames": len(frames), "grid_worst": round(worst, 4), "grid_worst_frame": worst_at,
            "grid_worst_step": round(step_at_worst, 5),
            "grid_longest_run": longest,
            "converge_frames": converge(frames, tile, tol)}


def synth(kind, n=40, size=128, cell=16, seed=1):
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:size, 0:size] / size
    base = 0.3 + 0.4 * x
    out = []
    order = rng.permutation((size // cell) ** 2)
    for i in range(n):
        if kind == "smooth":
            out.append(base * (0.5 + 0.5 * i / n))
        else:  # cells switch from old to new light a few per frame
            img = base * 0.5
            done = order[: int(len(order) * min(1.0, i / (n * 0.6)))]
            m = np.zeros((size // cell) ** 2, bool); m[done] = True
            m = np.kron(m.reshape(size // cell, size // cell), np.ones((cell, cell))).astype(bool)
            out.append(np.where(m, base, img))
    return out


def self_test():
    s = analyse(synth("smooth"), 8, 0.02)
    c = analyse(synth("checker"), 8, 0.02)
    print(json.dumps({"smooth": s, "checker": c}))
    ok = s["grid_worst"] < 0.15 and c["grid_worst"] > 0.5 and c["converge_frames"] > 20
    print("self-test", "ok" if ok else "FAIL")
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir", nargs="?")
    ap.add_argument("--tile", type=int, default=8)
    ap.add_argument("--tol", type=float, default=0.02)
    ap.add_argument("--stride", type=int, default=1, help="compare frame i with i-stride (slow suns)")
    ap.add_argument("--max-grid", type=float)
    ap.add_argument("--max-converge", type=int)
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    files = sorted(glob.glob(os.path.join(a.dir, "**", "*.png"), recursive=True))
    if len(files) < 2:
        print("need at least 2 frames in", a.dir, file=sys.stderr)
        return 2
    r = analyse([load(f) for f in files], a.tile, a.tol, a.stride)
    print(json.dumps(r))
    bad = (a.max_grid is not None and r["grid_worst"] > a.max_grid) or \
          (a.max_converge is not None and r["converge_frames"] > a.max_converge)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
