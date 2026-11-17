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

* gloss-* scenarios (specular stage S0, docs/systems/lighting/specular-v3-design.md):
  spec_err_near is ref_err_near over pixels whose reference specular share
  (f*-reference-spec.png / f*-reference.png) is >= 0.2, worst and mean, and per
  roughness row where the scenario names rows (gloss-sweep). spec_pop is the
  worst frame-to-frame luminance step in the masked pixels of the live sequence
  minus the same step in the reference sequence. gloss-switch adds
  frames_to_ref as for switch. spec_ms is the median spec_ms column of the
  gi_metrics CSV (lavapipe numbers only compare run against run). Without the
  specular AOV the mask is reported as missing and the spec metrics are left out.

* blotch_mean / blotch_worst (every scenario with references): the gi-check
  still-frame blotch (examples/stress/src/gi_check/metrics.rs `blotch`): the
  error band blurred s px minus blurred 6 s px, over the reference blurred s px
  plus 5% of its mean, averaged over the picture. s = 4 px at 320 wide, scaled
  to the run's width (2 px at the oracle's 160 x 90). Only arm against arm at
  the same size compares; the gi-check room numbers (ADR 0039: 0.021 at 25 cm,
  0.014 at 12.5 cm) are 320 x 180 of a 640 x 360 run.
* slant* scenarios (ADR 0039 stage 2): slant_mid_err_worst / _mean, the
  unweighted error over the middle half of the picture (where the turned box
  is), per reference frame.

`--self-test` checks the error, weight and counter on synthetic pictures.
"""
import csv, glob, json, os, re, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import grid_metric  # noqa: E402  (PR #94's metric, reused unchanged)

LUM = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
DEPTH_RANGE = 50.0  # lighting_compose.comp: depth view = 1 - distance / 50


def linear_from_display(c):
    """Undo the shoulder tone the engine and reference PNGs store (mesh::tone,
    hdr_shoulder.glsl, trace.rs linear_from_display): linear to 0.64, then
    0.64 + 0.14 e / (e + 1.1). Not sRGB."""
    k = np.clip((c - 0.64) / 0.14, 0.0, 0.999)
    return np.where(c <= 0.64, np.maximum(c, 0.0), 0.64 + 1.1 * k / (1.0 - k))


# Shoulder ceiling: 0.64 + 0.14 = 0.78 (198.9/255). The PNG writers dither by
# up to ~2/255, so anything within 3/255 of the ceiling is saturated: the tone
# curve is flat there and the radiance that produced it is unrecoverable.
SHOULDER_CEILING = 0.78
CEILING_DITHER = 3.0 / 255.0


def over_ceiling(rgb):
    """Per-pixel mask: any channel at the shoulder ceiling (saturated)."""
    return (rgb >= SHOULDER_CEILING - CEILING_DITHER).any(axis=-1)


def load_rgb(path):
    from PIL import Image
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0


def lin_luma(path):
    return linear_from_display(load_rgb(path)) @ LUM


def distance(path):
    v = load_rgb(path)[..., 0]
    return DEPTH_RANGE * (1.0 - v)


def block(a, b=2):
    h, w = a.shape[0] // b * b, a.shape[1] // b * b
    return a[:h, :w].reshape(h // b, b, w // b, b).mean(axis=(1, 3))


def ref_error(live, ref, dist=None, mask=None):
    """(near-weighted, unweighted) share of the light that is wrong.
    mask: pixels (at ref resolution) excluded from numerator and denominator,
    i.e. saturated pixels, reported separately as over_ceiling_px."""
    if live.shape != ref.shape:
        from PIL import Image
        live = np.asarray(Image.fromarray(live).resize(ref.shape[::-1], Image.BILINEAR))
    if mask is not None:
        live, ref = np.where(mask, 0.0, live), np.where(mask, 0.0, ref)
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


def _kernel(s):
    r = int(3.0 * s + 0.5)
    x = np.arange(-r, r + 1, dtype=np.float64)
    k = np.exp(-(x * x) / (2.0 * s * s))
    return k / k.sum()


def gblur(a, s):
    """Separable Gaussian, edges repeated (gi_check/metrics.rs `blur`)."""
    k = _kernel(s)
    r = len(k) // 2
    a = np.asarray(a, np.float64)
    p = np.pad(a, ((0, 0), (r, r)), mode="edge")
    t = sum(kv * p[:, i:i + a.shape[1]] for i, kv in enumerate(k))
    p = np.pad(t, ((r, r), (0, 0)), mode="edge")
    return sum(kv * p[i:i + a.shape[0], :] for i, kv in enumerate(k))


def blotch(live, ref, s=None):
    """gi_check/metrics.rs `blotch`; s defaults to 4 px per 320 px of width."""
    if s is None:
        s = 4.0 * ref.shape[1] / 320.0
    e = np.asarray(live, np.float64) - np.asarray(ref, np.float64)
    base = gblur(ref, s) + 0.05 * float(np.mean(ref))
    return float(np.mean(np.abs(gblur(e, s) - gblur(e, 6.0 * s)) / base))


def mid_err(live, ref):
    """Unweighted error over the middle half of the picture (each axis)."""
    h, w = ref.shape
    sl = (slice(h // 4, h - h // 4), slice(w // 4, w - w // 4))
    return ref_error(live[sl], ref[sl])[1]


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
        blots = []
        mids = []
        energy = []
        for lv in sorted(glob.glob(os.path.join(out, "oracle", n, "f*-live.png"))):
            stem = lv[:-len("-live.png")]
            ref = stem + "-reference.png"
            if not os.path.exists(ref):
                continue
            dp = stem + "-depth.png"
            lrgb, rrgb = load_rgb(lv), load_rgb(ref)
            oc_live = over_ceiling(lrgb)
            m = oc_live | over_ceiling(rrgb) if oc_live.shape == rrgb.shape[:2] else over_ceiling(rrgb)
            ll, rl = linear_from_display(lrgb) @ LUM, linear_from_display(rrgb) @ LUM
            near, plain = ref_error(ll, rl, distance(dp) if os.path.exists(dp) else None, mask=m)
            per.append((int(re.search(r"f(\d+)$", stem).group(1)), near, plain, int(oc_live.sum())))
            if ll.shape == rl.shape:
                blots.append(blotch(ll, rl))
                energy.append(float(ll.sum()) / max(float(rl.sum()), 1e-9))
                if n.startswith("slant"):
                    mids.append(mid_err(ll, rl))
        if blots:
            s.update(blotch_mean=round(float(np.mean(blots)), 4), blotch_worst=round(max(blots), 4))
        if energy:
            s.update(energy_mean=round(float(np.mean(energy)), 4), energy_min=round(min(energy), 4))
        if mids:
            s.update(slant_mid_err_mean=round(float(np.mean(mids)), 4), slant_mid_err_worst=round(max(mids), 4))
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
                     ref_err_mean=round(float(np.mean([p[2] for p in per])), 4),
                     over_ceiling_px=max(p[3] for p in per))
            # Saturated live pixels hide unbounded radiance: a live failure.
            s["over_ceiling_ok"] = s["over_ceiling_px"] == 0
            if n == "switch" or n == "lamps-switch":
                settled = per[-1][1]
                s["frames_to_ref"] = next((f for f, e, *_ in per if e <= 1.1 * settled + 1e-4), per[-1][0])
                s["ref_err_near_by_frame"] = {str(f): round(e, 4) for f, e, *_ in per}
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


def _png(path, code, size, hot=None):
    from PIL import Image
    im = Image.new("RGB", size, (code, code, code))
    if hot is not None:
        x, y, hcode = hot
        im.putpixel((x, y), (hcode, hcode, hcode))
    im.save(path)


def ceiling_self_test():
    """score() on the review's pictures. A live code 199 against flat code 40
    must move ref_err, and blotch, energy, slant mid_err and score_spec must
    not see the inverse's ~1099 clamp. Codes 196 and 197 are under the dither
    band, so they stay in the error and are not over_ceiling."""
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        def scene(name, live_code, size=(8, 8), ref_size=None, hot=None, spec=False):
            ref_size = size if ref_size is None else ref_size
            folder = os.path.join(d, "oracle", name)
            os.makedirs(folder)
            _png(os.path.join(folder, "f0001-live.png"), live_code, size, hot)
            _png(os.path.join(folder, "f0001-reference.png"), 40 if hot else live_code, ref_size)
            if spec:
                _png(os.path.join(folder, "f0001-reference-spec.png"), 40, ref_size)
        scene("clip199", 40, hot=(0, 0, 199))
        scene("clip196", 40, hot=(0, 0, 196))
        scene("clip197", 40, hot=(0, 0, 197))
        scene("slant-clip", 40, hot=(4, 4, 199))
        scene("gloss-clip", 40, hot=(0, 0, 199), spec=True)
        scene("clip-big", 40, size=(16, 16), ref_size=(8, 8), hot=(0, 0, 199))
        scene("clip-both", 199)
        got = score(d)
    raw199 = float(linear_from_display(np.float32(199 / 255)))
    rows = {}
    for name, s in got.items():
        rows[name] = {k: s.get(k) for k in (
            "ref_err_mean", "blotch_worst", "energy_mean", "over_ceiling_px",
            "over_ceiling_frac", "slant_mid_err_mean", "spec_err_near_mean")}
    print(json.dumps({"ceiling_score": rows, "linear_from_display_199": round(raw199, 2)}))
    c199, c196, c197 = got["clip199"], got["clip196"], got["clip197"]
    slant, gloss, big, both = got["slant-clip"], got["gloss-clip"], got["clip-big"], got["clip-both"]
    # The inverse clamp stays. The score must not use it.
    inverse_ok = abs(raw199 - 1099.54) < 1.0
    one_sided = (1.0 < c199["ref_err_mean"] < 20.0 and c199["blotch_worst"] < 10.0
                 and c199["energy_mean"] < 20.0 and c199["over_ceiling_px"] == 1
                 and c199.get("over_ceiling_frac") is not None
                 and abs(c199["over_ceiling_frac"] - 1.0 / 64.0) < 1e-3)
    kept = (c196["over_ceiling_px"] == 0 and c196["ref_err_mean"] > 0.5
            and c196.get("over_ceiling_frac") == 0
            and c197["over_ceiling_px"] == 0 and c197["ref_err_mean"] > 0.5)
    others = (1.0 < slant["slant_mid_err_mean"] < 40.0
              and 1.0 < gloss["spec_err_near_mean"] < 30.0
              and 0.1 < big["ref_err_mean"] < 10.0)
    # Both sides at the ceiling are unmeasurable. The fraction says so.
    paired = (both["ref_err_mean"] < 1e-6 and both["over_ceiling_px"] == 64
              and both.get("over_ceiling_frac") == 1.0 and both["blotch_worst"] < 1.0
              and both["energy_mean"] < 2.0)
    ok = inverse_ok and one_sided and kept and others and paired
    print("ceiling score", "ok" if ok else "FAIL")
    return ok


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
    # Engine and reference PNGs store the shoulder tone (mesh::tone,
    # hdr_shoulder.glsl), not sRGB: linear below 0.64, so 128/255 reads back
    # as 128/255 and the shoulder undoes to the light that made it.
    import tempfile
    from PIL import Image
    with tempfile.TemporaryDirectory() as d:
        Image.new("RGB", (2, 2), (128, 128, 128)).save(d + "/a.png")
        Image.new("RGB", (2, 2), (180, 180, 180)).save(d + "/b.png")
        la, lb = float(lin_luma(d + "/a.png").mean()), float(lin_luma(d + "/b.png").mean())
    x = 180 / 255 - 0.64
    tone_ok = abs(la - 128 / 255) < 1e-3 and abs(lb - (0.64 + 1.1 * x / (0.14 - x))) < 1e-2
    print(json.dumps({"lin_luma_128": round(la, 4), "lin_luma_180": round(lb, 4)}))
    # dither.glsl's noise is under one code, so the asymptote (code 198.9)
    # rounds to 198 and above. 196 and 197 are measured. 190 is below the band.
    sat = np.full((4, 4, 3), 0.3, np.float32)
    sat[0, 0] = 198 / 255
    sat[0, 1] = 199 / 255
    sat[1, 0] = 196 / 255
    sat[1, 1] = 197 / 255
    sat[2, 2] = 190 / 255
    oc = over_ceiling(sat)
    flat = np.full((4, 4), 0.3, np.float32)
    hot = flat.copy(); hot[0, 0] = 1100.0
    masked, _ = ref_error(hot, flat, None, mask=np.zeros((4, 4), bool) | (np.arange(16).reshape(4, 4) == 0))
    band_ok = bool(oc[0, 0] and oc[0, 1] and not oc[1, 0] and not oc[1, 1] and not oc[2, 2])
    mask_ok = masked < 1e-6
    print(json.dumps({"over_ceiling": [bool(oc[0, 0]), bool(oc[0, 1]), bool(oc[1, 0]), bool(oc[1, 1]), bool(oc[2, 2])],
                      "masked_err": round(masked, 6)}))
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
    # Blotch: an even bias over the picture does not count, a 16 px blob does;
    # a blob twice as strong scores about twice as much.
    flat = np.full((90, 160), 0.3, np.float32)
    blob = flat.copy(); blob[37:53, 72:88] += 0.06
    blob2 = flat.copy(); blob2[37:53, 72:88] += 0.12
    b_bias, b_blob, b_blob2 = blotch(flat * 1.2, flat), blotch(blob, flat), blotch(blob2, flat)
    ok_blotch = b_bias < 1e-9 and b_blob > 1e-3 and abs(b_blob2 / b_blob - 2.0) < 0.05
    ok_mid = mid_err(blob, flat) > 3 * ref_error(blob, flat)[1] and mid_err(flat, flat) < 1e-9
    scored_ok = ceiling_self_test()
    ok = (ok_ex and ok_blotch and ok_mid and same < 1e-6 and abs(p1 - p2) < 0.05 and n1 > 3 * n2
          and unchanged(still) == 1.0 and unchanged(moving) == 0.0 and tone_ok and band_ok
          and mask_ok and scored_ok)
    print(json.dumps({"near_err_near_bad": round(n1, 4), "near_err_far_bad": round(n2, 4),
                      "plain": [round(p1, 4), round(p2, 4)],
                      "blotch": [b_bias, round(b_blob, 4), round(b_blob2, 4)], "mid": ok_mid}))
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
