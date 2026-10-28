#!/usr/bin/env python3
"""Score a lighting harness run (scripts/lighting/oracle_run.sh) as compact JSON.

Per scenario (sun-x100, sun-x10, sun-x1, spin, switch, turn):

* ref_err_near: near-camera-weighted error against the path-traced reference,
  sum(w |live - ref|) / sum(w ref) in linear luminance over 2x2 blocks, with
  w = 1 / max(distance, 0.5 m) from the depth view. Worst frame and mean.
* ref_err: the same without the weight, for comparison.
* grid_worst / grid_longest_run: the PR #94 checkerboard metric (grid_metric.py)
  over every saved frame of the uninterrupted pass.
* unchanged: the recomputed-but-unchanged counter. Every lighting pass runs
  on every pixel each frame, so `px_share` is the share of 8x8 tiles whose
  picture did not change (|delta luminance| < 1/255) while it was recomputed;
  `refreshes` is the mean cache_entries_refreshed per frame from gi_metrics and
  `refreshes_unchanged` the part of it that landed on unchanged tiles, assuming
  refreshes spread evenly over the screen (a screen-space proxy, not a
  per-texel count).
* switch only: frames_to_ref, the first frame after the switch whose ref_err_near
  is within 1.1x of the settled frame's.

* gloss-* scenarios (specular stage S0, docs/systems/lighting/specular-v3-design.md):
  spec_err_near is ref_err_near over pixels whose reference specular share
  (f*-reference-spec.png / f*-reference.png) is >= 0.2, worst and mean, and per
  roughness row where the scenario names rows (gloss-sweep). spec_pop is the
  worst frame-to-frame luminance step in the masked pixels of the live sequence
  minus the same step in the reference sequence. gloss-switch adds
  frames_to_ref as for switch. spec_ms is the median spec_ms column of the
  gi_metrics CSV (lavapipe numbers only compare run against run). Without the
  specular AOV the mask is reported as missing and the spec metrics are left out.

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


def gi_refreshes(path):
    if not os.path.exists(path):
        return None
    with open(path) as f:
        rows = list(csv.DictReader(f))
    if not rows:
        return None
    return float(np.mean([float(r["cache_entries_refreshed"]) for r in rows])), \
        float(np.median([float(r["lighting_ms"]) for r in rows]))


SPEC_SHARE = 0.2
# gloss-sweep rows: five spheres at these roughness values, left to right,
# metal row on top and dielectric row below (lighting_oracle_spec.rhai).
SWEEP_ROUGHNESS = [0.05, 0.2, 0.4, 0.6, 0.9]


def spec_mask(spec_ref, total_ref, share=SPEC_SHARE):
    """Pixels where the reference light is at least `share` specular."""
    return (total_ref > 1e-4) & (spec_ref >= share * np.maximum(total_ref, 1e-6))


def masked_error(live, ref, mask, dist=None):
    """ref_error's near-weighted share, over masked pixels only."""
    d = np.abs(live - ref) * mask
    r = ref * mask
    w = np.ones_like(ref) if dist is None else 1.0 / np.maximum(dist, 0.5)
    return float((w * d).sum() / ((w * r).sum() + 1e-6))


def worst_step(frames, mask):
    """Worst mean |delta luminance| between consecutive frames over the mask."""
    if len(frames) < 2 or not mask.any():
        return 0.0
    return max(float(np.abs(b - a)[mask].mean()) for a, b in zip(frames, frames[1:]))


def spec_pop(live, ref, mask):
    return worst_step(live, mask) - worst_step(ref, mask)


def sweep_rows(mask, cols=len(SWEEP_ROUGHNESS)):
    """Masks for each roughness column (both metal and dielectric rows)."""
    h, w = mask.shape
    out = {}
    for i, r in enumerate(SWEEP_ROUGHNESS[:cols]):
        m = np.zeros_like(mask)
        m[:, i * w // cols:(i + 1) * w // cols] = True
        out[str(r)] = m & mask
    return out


def spec_ms(path):
    if not os.path.exists(path):
        return None
    with open(path) as f:
        rows = [r for r in csv.DictReader(f) if r.get("spec_ms") not in (None, "")]
    return float(np.median([float(r["spec_ms"]) for r in rows])) if rows else None


def score_spec(out, n, s):
    per, lives, refs, mask_all = [], [], [], None
    for lv in sorted(glob.glob(os.path.join(out, "oracle", n, "f*-live.png"))):
        stem = lv[:-len("-live.png")]
        ref, sp = stem + "-reference.png", stem + "-reference-spec.png"
        if not (os.path.exists(ref) and os.path.exists(sp)):
            continue
        l, r = lin_luma(lv), lin_luma(ref)
        if l.shape != r.shape:
            continue
        m = spec_mask(lin_luma(sp), r)
        dp = stem + "-depth.png"
        dist = distance(dp) if os.path.exists(dp) else None
        f = int(re.search(r"f(\d+)$", stem).group(1))
        per.append((f, masked_error(l, r, m, dist), m, l, r, dist))
        lives.append(l); refs.append(r)
        mask_all = m if mask_all is None else (mask_all | m)
    if not per:
        s["spec_mask"] = "missing"
        return
    errs = [p[1] for p in per]
    worst = max(per, key=lambda p: p[1])
    s.update(spec_frames=len(per), spec_err_near_worst=round(worst[1], 4),
             spec_err_near_worst_frame=worst[0], spec_err_near_mean=round(float(np.mean(errs)), 4),
             spec_mask_share=round(float(np.mean([p[2].mean() for p in per])), 4),
             spec_pop=round(spec_pop(lives, refs, mask_all), 4))
    if n == "gloss-sweep":
        rows = {}
        for k in SWEEP_ROUGHNESS:
            rows[str(k)] = round(float(np.mean([masked_error(p[3], p[4], sweep_rows(p[2])[str(k)], p[5])
                                                for p in per])), 4)
        s["spec_err_near_by_roughness"] = rows
    if n == "gloss-switch":
        settled = per[-1][1]
        s["frames_to_ref"] = next((f for f, e, *_ in per if e <= 1.1 * settled + 1e-4), per[-1][0])
        s["spec_err_near_by_frame"] = {str(p[0]): round(p[1], 4) for p in per}


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
        if per:
            near = [p[1] for p in per]
            worst = max(per, key=lambda p: p[1])
            s.update(ref_frames=len(per), ref_err_near_worst=round(worst[1], 4), ref_err_near_worst_frame=worst[0],
                     ref_err_near_mean=round(float(np.mean(near)), 4),
                     ref_err_mean=round(float(np.mean([p[2] for p in per])), 4))
            if n == "switch":
                settled = per[-1][1]
                s["frames_to_ref"] = next((f for f, e, _ in per if e <= 1.1 * settled + 1e-4), per[-1][0])
                s["ref_err_near_by_frame"] = {str(f): round(e, 4) for f, e, _ in per}
        if n.startswith("gloss-"):
            score_spec(out, n, s)
            ms = spec_ms(os.path.join(out, "gi", f"{n}-frames.csv"))
            if ms is not None:
                s["spec_ms_p50_lavapipe"] = round(ms, 2)
        res[n] = s
    return res


def spec_self_test():
    rng = np.random.default_rng(7)
    diffuse = rng.uniform(0.2, 0.4, (32, 32)).astype(np.float32)
    spec = np.zeros_like(diffuse)
    spec[8:24, 8:24] = 0.6  # a glossy patch
    total = diffuse + spec
    m = spec_mask(spec, total)
    mask_ok = bool(m[8:24, 8:24].all()) and not m[:8].any() and not m[24:].any()
    e1 = masked_error(total + 0.05 * m, total, m)
    e2 = masked_error(total + 0.10 * m, total, m)
    e_off = masked_error(total + 0.10 * (~m), total, m)  # error outside the mask is ignored
    linear = abs(e2 - 2 * e1) < 1e-4 and e1 > 0 and e_off < 1e-6
    still = [total] * 5
    spike = list(still); spike[2] = total + 0.3 * m
    pop_caught = spec_pop(spike, still, m) > 0.25 and abs(spec_pop(still, still, m)) < 1e-6
    lit = [total * 0.1, total, total]  # reference changes too: that step is not a pop
    pop_ref = abs(spec_pop(lit, lit, m)) < 1e-6
    rows = sweep_rows(np.ones((10, 50), bool))
    rows_ok = len(rows) == 5 and all(v.sum() == 100 for v in rows.values())
    ok = mask_ok and linear and pop_caught and pop_ref and rows_ok
    print(json.dumps({"spec_mask": mask_ok, "spec_err_linear": [round(e1, 4), round(e2, 4)],
                      "spec_pop_spike": pop_caught, "spec_pop_ref": pop_ref, "rows": rows_ok}))
    print("spec self-test", "ok" if ok else "FAIL")
    return 0 if ok else 1


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
    ok = (same < 1e-6 and abs(p1 - p2) < 0.05 and n1 > 3 * n2
          and unchanged(still) == 1.0 and unchanged(moving) == 0.0)
    print(json.dumps({"near_err_near_bad": round(n1, 4), "near_err_far_bad": round(n2, 4),
                      "plain": [round(p1, 4), round(p2, 4)]}))
    print("self-test", "ok" if ok else "FAIL")
    return 0 if ok and grid_metric.self_test() == 0 and spec_self_test() == 0 else 1


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
