#!/usr/bin/env python3
"""Attribute the scored slant gate, not a mean-absolute stand-in.

Premise (shared, failed, do not build the next fix on it): the r14 rise in
``slant_mid_err_mean`` is the axis-aligned ground plane, because that plane is
most of the middle-half pixels and because a568763f changes only the
axis-aligned display seed.

That premise treats a pixel count as a share of the gate, and it treats a
downward miss as the ground plane. The attached PNG split already moves on
turned faces as well. Its 0.037 scale is mean |live-ref| luma. The gate is
about 0.253 because it divides by the reference sum. Those are different
reductions of the same PNGs.

The gate, from ``oracle_metrics.score``:

    slant_mid_err_mean = round(mean(mid_err(lin_luma(live), lin_luma(ref))), 4)
    mid_err = sum(|block2(live)-block2(ref)|) / (sum(block2(ref)) + 1e-6)

over the middle half of ``oracle/<scene>/f*-live.png`` and the matching
``f*-reference.png``. ``lin_luma`` is sRGB-decoded Rec.709. This is not
``compare_linear`` on the path tracer's float radiance, and it is not the
mean absolute luma.

Decisive measurement (this script, rerun on the saved r14 trees):

    python3 scripts/lighting/slant_mid_split.py MASTER_OUT CUT_OUT

Acceptance on those trees: slant ``slant_mid_err_mean`` prints 0.2532 for
master and 0.2583 for the cut (the harness rounding). Class ``contrib`` values
sum to that metric. ``contrib_minus_master`` sums to the metric delta. A class
other than ``floor`` holding part of the +0.0051 above the 0.0002 repeat noise
keeps the premise dead. The premise would be alive only if that delta sat on
``floor`` alone after depth classification.

Result on the saved r14 trees, depth classification, both repeats: slant
prints 0.2532 and 0.2583. Mean ``contrib_minus_master`` is floor +0.0035,
box_slant +0.0015 (each repeat above the 0.0002 noise), box_top +0.0002,
edge about 0, other about 0. Mixed blocks are +0.0001. The floor-only
premise is dead. Do not rerun this split to decide the next edit. The axis
display seed is not the turned-face path: a ray hit reads the cell mean, and
a slanted screen pixel shows the probes. The claim inside ``lighting_sh_read``
is the coupling that read does have, and ``lighting_sh_read.rs`` locks that a
slanted read does not claim.

Hits come from the depth PNG the harness already writes (``1 - byte/255`` times
50 m, the same channel ``oracle_metrics.distance`` reads). A point near the
ground plane is ``floor``. A point on the turned box is ``box_slant`` or
``box_top``. A wall is ``other``. The old projection remains only when depth
is missing, and it still calls every downward miss ``floor``.

``proxy_abs_mean`` is the old mean |luma| number. It is not the gate.

a568763f removed ``lighting_sc_display`` from the lattice gate inside
``lighting_cache_irradiance``. A slanted screen pixel does not take that
function: ``lighting_world_irradiance_in`` returns ``lighting_sh_read``, which
is empty unless a normal component has magnitude 1, and compose keeps the
probes. A turned-face delta in this report is still real scored error. It does
not by itself mean the seed branch ran on that pixel. Do not answer it by
putting the neighbour blend back, and do not add a compensation term.
"""

from __future__ import annotations

import glob
import json
import math
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

# examples/stress/src/building.rs section_movers, the red box nearest (12.5, 20).
BOX = (10.5, 0.0, 19.5)
BOX_YAW = 0.6
BOX_HALF = (0.5, 0.5, 0.5)  # size 1, height 1; collider center is half the height
# lighting_oracle.rhai slant(): eye (p + (1.4, 1.3, 2)), look (p + (0, 0.35, 0)).
EYE = (BOX[0] + 1.4, BOX[1] + 1.3, BOX[2] + 2.0)
LOOK = (BOX[0], BOX[1] + 0.35, BOX[2])
WIDTH, HEIGHT = 240, 135
FOV_Y = math.radians(60.0)
AXIS_MIN = 0.999  # LIGHTING_SH_HIT_SLANT; a component at least this is an axis
# Depth view is 8-bit over 50 m (lighting_compose.comp). One step is ~0.196 m.
DEPTH_TOL = 0.25
CLASS_ORDER = ("floor", "box_slant", "box_top", "edge", "other", "miss")
PREMISE = (
    "r14 slant_mid_err_mean is not known to move only on the ground plane. "
    "A pixel majority in the middle half is not a share of the scored numerator."
)


def _norm(v):
    length = math.sqrt(sum(c * c for c in v))
    return tuple(c / length for c in v)


def _basis():
    forward = _norm(tuple(LOOK[i] - EYE[i] for i in range(3)))
    # look_direction's right is forward × Y (camera.rs view_matrix).
    right = _norm((-forward[2], 0.0, forward[0]))
    up = (
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    )
    return forward, right, up


def _ray(px, py, width, height, forward, right, up):
    # lamp_pick.rs reproject_pixel: ndc_y +1 is the top of the picture.
    ndc_x = ((px + 0.5) / width) * 2.0 - 1.0
    ndc_y = 1.0 - ((py + 0.5) / height) * 2.0
    tan_y = math.tan(FOV_Y * 0.5)
    aspect = width / height
    direction = tuple(
        forward[i] + right[i] * ndc_x * tan_y * aspect + up[i] * ndc_y * tan_y for i in range(3)
    )
    return _norm(direction)


def _to_local(point, direction, sine, cosine):
    dx, dy, dz = point[0] - BOX[0], point[1] - (BOX[1] + BOX_HALF[1]), point[2] - BOX[2]
    origin = (cosine * dx - sine * dz, dy, sine * dx + cosine * dz)
    local_dir = (
        cosine * direction[0] - sine * direction[2],
        direction[1],
        sine * direction[0] + cosine * direction[2],
    )
    return origin, local_dir


def _world_normal(local_normal, sine, cosine):
    x, y, z = local_normal
    return (cosine * x + sine * z, y, -sine * x + cosine * z)


def _box_hit(origin, direction, sine, cosine):
    """Nearest hit on the yawed box. Returns the world normal, or None."""
    local_o, local_d = _to_local(origin, direction, sine, cosine)
    t_hit = None
    for axis in range(3):
        component = local_d[axis]
        if abs(component) < 1.0e-12:
            if abs(local_o[axis]) > BOX_HALF[axis]:
                return None
            continue
        for sign, plane in ((-1.0, -BOX_HALF[axis]), (1.0, BOX_HALF[axis])):
            t = (plane - local_o[axis]) / component
            if t < 1.0e-4:
                continue
            point = tuple(local_o[i] + local_d[i] * t for i in range(3))
            if any(abs(point[i]) > BOX_HALF[i] + 1.0e-4 for i in range(3) if i != axis):
                continue
            if t_hit is None or t < t_hit[0]:
                normal = [0.0, 0.0, 0.0]
                normal[axis] = sign
                t_hit = (t, tuple(normal))
    if t_hit is None:
        return None
    return _world_normal(t_hit[1], sine, cosine)


def project(width=WIDTH, height=HEIGHT):
    """Per-pixel class from the pose alone: box_slant, box_top, or floor.

    A downward ray that misses the box is called floor. A wall in front of the
    ground plane is counted here too. Prefer ``classes_from_depth`` on a saved
    run. This projection is the fallback when the depth PNG is absent.
    """
    forward, right, up = _basis()
    sine, cosine = math.sin(BOX_YAW), math.cos(BOX_YAW)
    classes = []
    for py in range(height):
        for px in range(width):
            direction = _ray(px, py, width, height, forward, right, up)
            normal = _box_hit(EYE, direction, sine, cosine)
            if normal is None:
                classes.append("floor" if direction[1] < 0.0 else "miss")
                continue
            dominant = max(abs(c) for c in normal)
            classes.append("box_top" if dominant >= AXIS_MIN else "box_slant")
    return classes


def _local_point(point):
    sine, cosine = math.sin(BOX_YAW), math.cos(BOX_YAW)
    dx = point[0] - BOX[0]
    dy = point[1] - (BOX[1] + BOX_HALF[1])
    dz = point[2] - BOX[2]
    return (cosine * dx - sine * dz, dy, sine * dx + cosine * dz)


def classify_point(point):
    """Class of a world hit. ``other`` is a wall, the ceiling, or empty air.

    The shell is one depth step thick so an 8-bit distance still lands on the
    face it hit. The box foot is ``edge``: it is on the box and on the ground
    plane, and it is not given to either class.
    """
    local = _local_point(point)
    on_box = all(abs(local[i]) <= BOX_HALF[i] + DEPTH_TOL for i in range(3)) and any(
        abs(local[i]) >= BOX_HALF[i] - DEPTH_TOL for i in range(3)
    )
    on_ground = abs(point[1]) <= DEPTH_TOL
    if on_box and on_ground:
        return "edge"
    if on_box:
        axis = min(range(3), key=lambda i: abs(BOX_HALF[i] - abs(local[i])))
        return "box_top" if axis == 1 else "box_slant"
    if on_ground:
        return "floor"
    return "other"


def classes_from_depth(dist):
    """Classify every pixel from eye distance along the slant ray."""
    height, width = dist.shape
    forward, right, up = _basis()
    classes = []
    for py in range(height):
        for px in range(width):
            distance = float(dist[py, px])
            if not 1.0e-3 < distance < 49.9:
                classes.append("miss")
                continue
            direction = _ray(px, py, width, height, forward, right, up)
            point = tuple(EYE[i] + direction[i] * distance for i in range(3))
            classes.append(classify_point(point))
    return classes


def mid_classes(classes, width, height):
    y0, y1 = height // 4, height - height // 4
    x0, x1 = width // 4, width - width // 4
    out = []
    for py in range(y0, y1):
        for px in range(x0, x1):
            out.append(classes[py * width + px])
    return out


def shares(classes):
    total = len(classes)
    counts = {}
    for name in classes:
        counts[name] = counts.get(name, 0) + 1
    names = list(dict.fromkeys(list(CLASS_ORDER) + list(counts)))
    return {name: counts.get(name, 0) / total for name in names}, counts


def _metrics():
    import oracle_metrics

    return oracle_metrics


def attribute(live, ref, classes):
    """Each class's share of ``mid_err``.

    A 2x2 block that straddles classes splits its |block error| by how many of
    its four pixels sit in each class. The denominator is the whole middle
    half, so the shares add up to ``mid_err``. ``pure_contrib`` is the part
    from blocks of one class. ``mixed_contrib`` is the rest. Adding those two
    also recovers the metric. ``proxy_abs_mean`` is mean |pixel luma| and is
    a different number.
    """
    om = _metrics()
    metric = float(om.mid_err(live, ref))
    height, width = ref.shape
    y0, y1 = height // 4, height - height // 4
    x0, x1 = width // 4, width - width // 4
    blocks_l = om.block(live[y0:y1, x0:x1])
    blocks_r = om.block(ref[y0:y1, x0:x1])
    diff = abs(blocks_l - blocks_r)
    den = float(blocks_r.sum() + 1.0e-6)
    num = {}
    pure = {}
    mixed = 0.0
    signed = {}
    absolute = {}
    pixels = {}
    for iy in range(diff.shape[0]):
        for ix in range(diff.shape[1]):
            block_diff = float(diff[iy, ix])
            tally = {}
            for dy in range(2):
                for dx in range(2):
                    py = y0 + iy * 2 + dy
                    px = x0 + ix * 2 + dx
                    kind = classes[py * width + px]
                    tally[kind] = tally.get(kind, 0) + 1
                    delta = float(live[py, px] - ref[py, px])
                    signed[kind] = signed.get(kind, 0.0) + delta
                    absolute[kind] = absolute.get(kind, 0.0) + abs(delta)
                    pixels[kind] = pixels.get(kind, 0) + 1
            if len(tally) == 1:
                kind = next(iter(tally))
                pure[kind] = pure.get(kind, 0.0) + block_diff
            else:
                mixed += block_diff
            for kind, count in tally.items():
                num[kind] = num.get(kind, 0.0) + block_diff * (count / 4.0)
    # block() drops a trailing odd row or column. The proxy uses those same pixels.
    scored = int(diff.shape[0] * diff.shape[1] * 4)
    if scored == 0:
        raise SystemExit("the middle half has no 2x2 block")
    proxy = 0.0
    for kind, total in absolute.items():
        proxy += total
    return {
        "metric": metric,
        "from_blocks": float(diff.sum()) / den,
        "contrib": {kind: value / den for kind, value in num.items()},
        "pure_contrib": {kind: value / den for kind, value in pure.items()},
        "mixed_contrib": mixed / den,
        "proxy_abs_mean": proxy / scored,
        "proxy_abs": {kind: absolute[kind] / pixels[kind] for kind in pixels},
        "signed_mean": {kind: signed[kind] / pixels[kind] for kind in pixels},
        "pixels": pixels,
        "scored_pixels": scored,
    }


def _source(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as handle:
        return handle.read()


def _guard_sources():
    rhai = _source("examples/stress/scripts/lighting_oracle.rhai")
    building = _source("examples/stress/src/building.rs")
    gpu = _source("crates/render/src/lighting_gpu.rs")
    shader = _source("crates/render/shaders/lighting_cache.glsl")
    if "p[0] + 1.4, p[1] + 1.3, p[2] + 2.0" not in rhai:
        raise SystemExit("slant camera offset changed; update EYE")
    if "look_at: [p[0], p[1] + 0.35, p[2]]" not in rhai:
        raise SystemExit("slant look target changed; update LOOK")
    if "for k in [30, 120, 300]" not in rhai:
        raise SystemExit("slant measure frames changed")
    if "solid(Shape::Square, 10.5, 19.5, 1.0, 1.0," not in building:
        raise SystemExit("the red hall box moved; update BOX")
    keep = re.search(r"const LIGHTING_MOVER_KEEP: u32 = (\d+);", gpu)
    if keep is None:
        raise SystemExit("LIGHTING_MOVER_KEEP not found")
    keep_frames = int(keep.group(1))
    if not (30 < keep_frames and 120 < keep_frames and 300 > keep_frames):
        raise SystemExit(f"mover keep {keep_frames} no longer splits the slant frames")
    if "lighting_sc_display || slanted" in shader:
        raise SystemExit("the display flag selects the lattice blend again")
    if "if (slanted && hsize < LIGHTING_SH_HIT_SPAN * LIGHTING_SH_CELL)" not in shader:
        raise SystemExit("the slanted lattice gate moved")
    if "lighting_sh_on() && lighting_obj_salt < 0.5" not in shader:
        raise SystemExit("the mover salt no longer skips the SH read")
    # The screen and seed read is the eight-corner blend at the true normal.
    # A corner is found with claim false, then accepted, and only then stamped.
    # The old axis-only skip is not this read. The loop lives in
    # lighting_sh_gather. The compose compile memoizes that value; every other
    # compile returns it directly. Neither claims a slot.
    read_at = shader.find("vec4 lighting_sh_read(vec3 p, vec3 n, bool claim)")
    owner_at = shader.find("bool lighting_sh_detail_owner_lit", read_at if read_at >= 0 else 0)
    if read_at < 0 or owner_at < 0:
        raise SystemExit("lighting_sh_read moved")
    wrapper = shader[read_at:owner_at]
    if "return lighting_sh_gather(p, n, claim).value;" not in wrapper:
        raise SystemExit("lighting_sh_read no longer returns the gather value")
    if "lighting_cache_find_sized" in wrapper:
        raise SystemExit("lighting_sh_read claims a slot itself")
    start = shader.find("LightingShField lighting_sh_gather(vec3 p, vec3 n, bool claim)")
    if start < 0 or start > read_at:
        raise SystemExit("lighting_sh_gather moved")
    body = shader[start:read_at]
    if "if (abs(n[ax]) < 1.0)" in body:
        raise SystemExit("lighting_sh_read still drops a non-axis normal")
    if "lighting_cache_find_sized(p, n, true, cs, p)" in body:
        raise SystemExit("lighting_sh_read claims the query point")
    accepted = -1
    for piece in (
        "lighting_cache_find_sized(centre, an, false, cs, p)",
        "if (facing <= 0.0)",
        "if (plane > LIGHTING_CELL_HALF * cs)",
        "if (v.w < LIGHTING_CACHE_HAS_W)",
        "if (lighting_sc_stale(s))",
        "if (claim && LIGHTING_SC_WRITES)",
    ):
        found = body.find(piece)
        if found < 0 or found < accepted:
            raise SystemExit("a neighbour corner is claimed before it is accepted")
        accepted = found
    if "return lighting_sh_read(pos, n, false);" not in shader:
        raise SystemExit("a slanted screen read does not use the neighbour blend")
    metrics = _source("scripts/lighting/oracle_metrics.py")
    if "mids.append(mid_err(ll, rl))" not in metrics:
        raise SystemExit("slant_mid_err_mean is no longer the mean of mid_err")
    if "slant_mid_err_mean=round(float(np.mean(mids)), 4)" not in metrics:
        raise SystemExit("the harness no longer rounds slant_mid_err_mean to 4 decimals")
    return keep_frames


def _point_classes():
    ground = (BOX[0] + 3.0, 0.0, BOX[2] + 3.0)
    wall = (BOX[0] + 3.0, 1.5, BOX[2] + 3.0)
    if classify_point(ground) != "floor":
        raise SystemExit(f"ground plane classified as {classify_point(ground)}")
    if classify_point(wall) != "other":
        raise SystemExit(f"a wall at y=1.5 classified as {classify_point(wall)}")
    sine, cosine = math.sin(BOX_YAW), math.cos(BOX_YAW)
    dx = cosine * 0.5
    dz = -sine * 0.5
    side = (BOX[0] + dx, BOX[1] + BOX_HALF[1], BOX[2] + dz)
    if classify_point(side) != "box_slant":
        raise SystemExit(f"turned face classified as {classify_point(side)}")
    top = (BOX[0], BOX[1] + BOX_HALF[1] + 0.5, BOX[2])
    if classify_point(top) != "box_top":
        raise SystemExit(f"box top classified as {classify_point(top)}")
    foot = (BOX[0], 0.0, BOX[2])
    if classify_point(foot) != "edge":
        raise SystemExit(f"box foot classified as {classify_point(foot)}")


def _metric_identity():
    import numpy as np

    ref = np.full((32, 32), 0.144, np.float32)
    live = ref.copy()
    live[8:24, 8:15] = 0.144 + 0.02
    live[8:24, 15:24] = 0.144 + 0.08
    classes = ["other"] * (32 * 32)
    for py in range(8, 24):
        for px in range(8, 15):
            classes[py * 32 + px] = "floor"
        for px in range(15, 24):
            classes[py * 32 + px] = "box_slant"
    got = attribute(live, ref, classes)
    if abs(got["metric"] - got["from_blocks"]) > 1.0e-5:
        raise SystemExit(f"mid_err {got['metric']} disagrees with the block sum {got['from_blocks']}")
    share = sum(got["contrib"].values())
    if abs(share - got["metric"]) > 1.0e-5:
        raise SystemExit(f"class shares {share} do not add up to mid_err {got['metric']}")
    parts = sum(got["pure_contrib"].values()) + got["mixed_contrib"]
    if abs(parts - got["metric"]) > 1.0e-5:
        raise SystemExit(f"pure plus mixed {parts} do not add up to mid_err {got['metric']}")
    if got["contrib"].get("floor", 0.0) <= 0.0 or got["contrib"].get("box_slant", 0.0) <= 0.0:
        raise SystemExit(f"a two-class error collapsed onto one class: {got['contrib']}")
    if got["mixed_contrib"] <= 0.0:
        raise SystemExit("the straddling column produced no mixed block")
    if abs(got["proxy_abs_mean"] - got["metric"]) < 0.05:
        raise SystemExit(
            f"mean |luma| {got['proxy_abs_mean']} reproduced mid_err {got['metric']}"
        )
    only = ref.copy()
    only[8:24, 8:24] = 0.144 + 0.04
    floor_classes = [
        "floor" if 8 <= py < 24 and 8 <= px < 24 else "other"
        for py in range(32)
        for px in range(32)
    ]
    owned = attribute(only, ref, floor_classes)
    if owned["contrib"].get("box_slant", 0.0) > 1.0e-6:
        raise SystemExit("a ground-only numerator was given to the box")
    if owned["contrib"].get("floor", 0.0) <= 0.1:
        raise SystemExit("a ground-only numerator missed the ground plane")
    json.dumps(plain({"metric": np.float32(got["metric"]), "proxy": np.float64(got["proxy_abs_mean"])}))


def self_test():
    keep = _guard_sources()
    classes = project()
    mid = mid_classes(classes, WIDTH, HEIGHT)
    ratio, counts = shares(mid)
    if ratio["floor"] <= ratio["box_slant"]:
        raise SystemExit(f"the mid window's pixel count is not mostly the ground plane: {counts}")
    if ratio["floor"] < 0.55:
        raise SystemExit(f"ground-plane pixel share {ratio['floor']:.3f} of the mid window")
    if ratio["box_top"] > 0.10:
        raise SystemExit(f"box top share {ratio['box_top']:.3f}")
    if counts.get("miss", 0):
        raise SystemExit(f"a mid-window ray missed the box and went upward: {counts}")
    _point_classes()
    try:
        import numpy  # noqa: F401
    except ImportError:
        print("metric identity skipped (needs numpy)")
    else:
        _metric_identity()
    print(
        f"slant pose {WIDTH}x{HEIGHT} mid {len(mid)} pixels: "
        f"floor {counts['floor']} ({ratio['floor']:.3f}), "
        f"box_slant {counts['box_slant']} ({ratio['box_slant']:.3f}), "
        f"box_top {counts['box_top']} ({ratio['box_top']:.3f}); "
        f"pixel counts, not the gate; "
        f"mover keep {keep} holds frames 30 and 120, not 300"
    )
    print("slant_mid_split: self-test ok")


def _luma(path):
    try:
        return _metrics().lin_luma(path)
    except ImportError as err:
        raise SystemExit(f"reading oracle pngs needs numpy and pillow ({err})") from err


def _frames(out, name):
    pattern = os.path.join(out, "oracle", name, "f*-live.png")
    found = sorted(glob.glob(pattern))
    if not found:
        found = sorted(glob.glob(os.path.join(out, "**", "oracle", name, "f*-live.png"), recursive=True))
    pairs = []
    for live in found:
        ref = live[: -len("-live.png")] + "-reference.png"
        if os.path.exists(ref):
            pairs.append((live, ref))
    return pairs


def _mean_field(frames, field):
    keys = set()
    for frame in frames:
        keys.update(frame[field])
    out = {}
    for key in keys:
        if field in ("proxy_abs", "signed_mean"):
            present = [frame[field][key] for frame in frames if key in frame[field]]
            if present:
                out[key] = sum(present) / len(present)
        else:
            out[key] = sum(frame[field].get(key, 0.0) for frame in frames) / len(frames)
    return out


def _recorded(out, scene):
    path = os.path.join(out, "metrics.json")
    if not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
    return data.get(scene, {}).get("slant_mid_err_mean")


def score_scene(out, scene):
    import numpy as np

    pairs = _frames(out, scene)
    if not pairs:
        raise SystemExit(f"no oracle/{scene} frames under {out}")
    frames = []
    sources = []
    names = []
    for live_path, ref_path in pairs:
        live = _luma(live_path)
        ref = _luma(ref_path)
        if live.shape != ref.shape:
            raise SystemExit(f"{live_path} and its reference differ in size")
        depth_path = live_path[: -len("-live.png")] + "-depth.png"
        if os.path.exists(depth_path):
            dist = _metrics().distance(depth_path)
            if dist.shape != live.shape:
                raise SystemExit(f"{depth_path} size {dist.shape} != picture {live.shape}")
            classes = classes_from_depth(dist)
            sources.append("depth")
        else:
            classes = project(live.shape[1], live.shape[0])
            sources.append("projection")
        got = attribute(live, ref, classes)
        if abs(got["metric"] - got["from_blocks"]) > 1.0e-4:
            raise SystemExit(f"{live_path}: mid_err drifted from the block sum")
        if abs(sum(got["contrib"].values()) - got["metric"]) > 1.0e-4:
            raise SystemExit(f"{live_path}: class shares do not add up to mid_err")
        frames.append(got)
        names.append(os.path.basename(live_path))
    mids = [frame["metric"] for frame in frames]
    mean = float(np.mean(mids))
    rounded = round(mean, 4)
    recorded = _recorded(out, scene)
    if recorded is not None and abs(float(recorded) - rounded) > 5.0e-5:
        raise SystemExit(
            f"{out} {scene}: recomputed {rounded} != metrics.json {recorded}"
        )
    contrib = _mean_field(frames, "contrib")
    if abs(sum(contrib.values()) - mean) > 1.0e-4:
        raise SystemExit(f"{scene}: mean class shares do not add up to the mean metric")
    source = sources[0] if len(set(sources)) == 1 else "mixed"
    pixels = _mean_field(frames, "pixels")
    pure = _mean_field(frames, "pure_contrib")
    proxy = _mean_field(frames, "proxy_abs")
    signed = _mean_field(frames, "signed_mean")
    return {
        "slant_mid_err_mean": rounded,
        "slant_mid_err_mean_full": mean,
        "frames": [
            {"file": name, "slant_mid_err": round(frame["metric"], 6)}
            for name, frame in zip(names, frames)
        ],
        "class_source": source,
        "depth_step_m": 50.0 / 255.0,
        "depth_tolerance_m": DEPTH_TOL,
        "scored_pixels": frames[0]["scored_pixels"],
        "mixed_block_contrib": sum(frame["mixed_contrib"] for frame in frames) / len(frames),
        "proxy_abs_mean": sum(frame["proxy_abs_mean"] for frame in frames) / len(frames),
        "classes": {
            kind: {
                "pixels": pixels.get(kind, 0.0),
                "contrib": contrib.get(kind, 0.0),
                "pure_contrib": pure.get(kind, 0.0),
                "proxy_abs_mean": proxy.get(kind, 0.0),
                "signed_mean": signed.get(kind, 0.0),
            }
            for kind in CLASS_ORDER
        },
        "recorded_metrics_json": recorded,
    }


def _deltas(report):
    for scene in ("slant", "slant-day"):
        master = report["master"][scene]
        cut = report["cut"][scene]
        cut["slant_mid_err_minus_master"] = (
            cut["slant_mid_err_mean_full"] - master["slant_mid_err_mean_full"]
        )
        total = 0.0
        for kind in CLASS_ORDER:
            delta = cut["classes"][kind]["contrib"] - master["classes"][kind]["contrib"]
            cut["classes"][kind]["contrib_minus_master"] = delta
            total += delta
        if abs(total - cut["slant_mid_err_minus_master"]) > 1.0e-4:
            raise SystemExit(f"{scene}: class deltas {total} do not add up to the metric delta")


def plain(value):
    """JSON numbers. ``json.dumps`` rejects numpy floats."""
    if isinstance(value, dict):
        return {str(key): plain(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in value]
    if isinstance(value, bool) or value is None:
        return value
    try:
        import numpy as np
    except ImportError:
        np = None
    if np is not None and isinstance(value, np.floating):
        return float(value)
    if np is not None and isinstance(value, np.integer):
        return int(value)
    if isinstance(value, float):
        return float(value)
    if isinstance(value, int):
        return int(value)
    return value


def main(argv):
    if not argv or argv == ["--self-test"]:
        self_test()
        return 0
    if len(argv) not in (1, 2):
        raise SystemExit("usage: slant_mid_split.py [--self-test] [master_out] [cut_out]")
    report = {
        "premise": PREMISE,
        "gate": (
            "slant_mid_err_mean = round(mean(oracle_metrics.mid_err), 4) on "
            "sRGB-decoded Rec.709 luma of f*-live.png and f*-reference.png, "
            "middle half, 2x2 blocks, sum(|live-ref|) / (sum(ref) + 1e-6). "
            "proxy_abs_mean is not this number."
        ),
        "acceptance": (
            "On the r14 trees this prints slant 0.2532 (master) and 0.2583 (cut). "
            "Any other number means these are not the scored pictures."
        ),
    }
    labels = ("run",) if len(argv) == 1 else ("master", "cut")
    for label, path in zip(labels, argv):
        report[label] = {
            "slant": score_scene(path, "slant"),
            "slant-day": score_scene(path, "slant-day"),
        }
    if len(argv) == 2:
        _deltas(report)
    print(json.dumps(plain(report), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
