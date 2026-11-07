#!/usr/bin/env python3
"""What the oracle slant window actually contains.

`slant_mid_err` is the unweighted error on the middle half of the picture.
At the slant pose (the red hall box, yaw 0.6, camera in lighting_oracle.rhai)
that window is mostly the axis-aligned floor, then the box's turned faces,
then the box top.

a568763f changes one read: `lighting_cache_irradiance` while compose's
display flag is set, the normal is axis-aligned, and the hit is not in a
mover frame. That is the floor. A slanted hit still takes the lattice
attempt it took before that commit. The box is a mover for
LIGHTING_MOVER_KEEP frames, so the measures at 30 and 120 read its faces
in local space (those normals are axis-aligned) with the mover salt set,
and that salt skips the SH branch. The empty slanted-screen read is not
the path those frames shade.

`--self-test` projects the pose and checks those shares. It does not run
the renderer. Passing two oracle output directories splits saved slant
frames by this projection (no new shader, no new binary):

    python3 scripts/lighting/slant_mid_split.py \\
        /workspace/s2-runs/r14/<master-out> /workspace/s2-runs/r14/<cut-out>
"""

from __future__ import annotations

import glob
import json
import math
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

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
    """Per-pixel class for the slant pose: box_slant, box_top, or floor.

    A downward ray that misses the box is the floor. A wall in front of that
    plane would be counted here too, and that wall is also axis-aligned.
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


def mid_slice(width, height):
    return slice(height // 4, height - height // 4), slice(width // 4, width - width // 4)


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
    return {name: counts.get(name, 0) / total for name in ("floor", "box_slant", "box_top", "miss")}, counts


def split_error(classes, live, ref):
    """Mean |live-ref| and mean(live-ref) per class. `live` and `ref` are linear luma."""
    buckets = {}
    for kind, a, b in zip(classes, live, ref):
        acc = buckets.setdefault(kind, [0.0, 0.0, 0])
        delta = a - b
        acc[0] += abs(delta)
        acc[1] += delta
        acc[2] += 1
    return {
        kind: {"abs": total_abs / n, "signed": total_signed / n, "n": n}
        for kind, (total_abs, total_signed, n) in buckets.items()
        if n
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
    return keep_frames


def self_test():
    keep = _guard_sources()
    classes = project()
    mid = mid_classes(classes, WIDTH, HEIGHT)
    ratio, counts = shares(mid)
    if ratio["floor"] <= ratio["box_slant"]:
        raise SystemExit(f"the mid window is not mostly floor: {counts}")
    if ratio["floor"] < 0.55:
        raise SystemExit(f"floor share {ratio['floor']:.3f} of the mid window")
    if ratio["box_top"] > 0.10:
        raise SystemExit(f"box top share {ratio['box_top']:.3f}")
    if counts.get("miss", 0):
        raise SystemExit(f"a mid-window ray missed the floor and the box: {counts}")
    # The floor carries the error; the turned faces do not. This is the
    # production claim the pictures have to confirm.
    live = [0.2 if kind == "floor" else 1.0 for kind in mid]
    ref = [1.0 for _ in mid]
    split = split_error(mid, live, ref)
    if split["floor"]["abs"] < 0.1 or split["box_slant"]["abs"] > 1.0e-9:
        raise SystemExit(f"the split attributed floor error to the box: {split}")
    if split["box_top"]["abs"] > 1.0e-9:
        raise SystemExit(f"box top absorbed floor error: {split}")
    print(
        f"slant pose {WIDTH}x{HEIGHT} mid {len(mid)}: "
        f"floor {counts['floor']} ({ratio['floor']:.3f}), "
        f"box_slant {counts['box_slant']} ({ratio['box_slant']:.3f}), "
        f"box_top {counts['box_top']} ({ratio['box_top']:.3f}); "
        f"mover keep {keep} holds frames 30 and 120, not 300"
    )
    print("slant_mid_split: self-test ok")


def _luma(path):
    try:
        from PIL import Image
        import numpy as np
    except ImportError as err:
        raise SystemExit(f"reading oracle pngs needs numpy and pillow ({err})") from err

    rgb = np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0
    linear = np.where(rgb <= 0.04045, rgb / 12.92, ((rgb + 0.055) / 1.055) ** 2.4)
    weights = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
    return linear @ weights


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


def score_dir(out, name):
    pairs = _frames(out, name)
    if not pairs:
        raise SystemExit(f"no oracle/{name} frames under {out}")
    sample = _luma(pairs[0][0])
    height, width = sample.shape
    classes = mid_classes(project(width, height), width, height)
    totals = {}
    for live_path, ref_path in pairs:
        live = _luma(live_path)
        ref = _luma(ref_path)
        y0, y1 = height // 4, height - height // 4
        x0, x1 = width // 4, width - width // 4
        split = split_error(classes, live[y0:y1, x0:x1].ravel(), ref[y0:y1, x0:x1].ravel())
        for kind, row in split.items():
            acc = totals.setdefault(kind, [0.0, 0.0, 0])
            acc[0] += row["abs"]
            acc[1] += row["signed"]
            acc[2] += 1
    return {
        kind: {"abs": a / n, "signed": s / n, "frames": n}
        for kind, (a, s, n) in totals.items()
    }


def main(argv):
    if not argv or argv == ["--self-test"]:
        self_test()
        return 0
    if len(argv) not in (1, 2):
        raise SystemExit("usage: slant_mid_split.py [--self-test] [master_out] [cut_out]")
    report = {}
    for label, path in zip(("master", "cut"), argv):
        report[label] = {"slant": score_dir(path, "slant"), "slant-day": score_dir(path, "slant-day")}
    if len(argv) == 2:
        for scene in ("slant", "slant-day"):
            for kind in report["master"][scene]:
                master = report["master"][scene][kind]["abs"]
                cut = report["cut"][scene].get(kind, {}).get("abs")
                if cut is not None:
                    report["cut"][scene][kind]["abs_minus_master"] = cut - master
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
