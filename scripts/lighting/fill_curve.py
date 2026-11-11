#!/usr/bin/env python3
"""Score a still-hall fill curve and a lamp-3 move against the path reference.

Near-weighted relative luminance, the same blocks and weight as
oracle_metrics.py: sum(w |live - ref|) / sum(w ref) on 2x2 means, linear
Rec.709 luma, w = 1 / max(distance, 0.5 m). Sky pixels (reference sRGB within
2 codes of (28, 36, 48), or a depth at the 50 m far plane) are left out.

`lightingsc` lines supply rays and frames-to-95% (`full / near` >= 0.95).

usage:
  fill_curve.py --self-test
  fill_curve.py score OUT --reference PNG --label NAME --log LOG --json FILE
  fill_curve.py lamp OFF_DIR ON_DIR --reference PNG
"""
import json, os, re, sys
import numpy as np

LUM = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
DEPTH_RANGE = 50.0
SKY = np.array([28, 36, 48], dtype=np.float32) / 255.0
MARK = re.compile(r"^-?\s*MARK (\S+) frame=([0-9.]+)")
LAMP = re.compile(r"^-?\s*LAMP3 index=(\d+)")
SC = re.compile(
    r"lightingsc \S+ frame (\d+) .* extra (\d+) partial (\d+) sets (\d+) rays (\d+) .* near (\d+) full (\d+)"
)


def srgb_to_linear(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def load_rgb(path):
    from PIL import Image
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0


def lin_luma(path):
    return srgb_to_linear(load_rgb(path)) @ LUM


def distance_from_depth(path):
    v = load_rgb(path)[..., 0]
    return DEPTH_RANGE * (1.0 - v)


def block(a, b=2):
    h, w = a.shape[0] // b * b, a.shape[1] // b * b
    return a[:h, :w].reshape(h // b, b, w // b, b).mean(axis=(1, 3))


def sky_mask(ref_srgb, dist):
    """True where the pixel is sky and should not enter the error."""
    sky_colour = np.max(np.abs(ref_srgb - SKY), axis=-1) <= (2.0 / 255.0)
    far = dist >= (DEPTH_RANGE - 0.5)
    if far.shape != sky_colour.shape:
        from PIL import Image
        far_img = Image.fromarray((far.astype(np.uint8) * 255))
        far = np.asarray(far_img.resize(sky_colour.shape[::-1], Image.NEAREST)) > 0
    return sky_colour | far


def ref_error(live_luma, ref_luma, dist, sky):
    """Near-weighted and plain relative error. sky is a bool image of live's shape."""
    if live_luma.shape != ref_luma.shape:
        from PIL import Image
        live_luma = np.asarray(
            Image.fromarray(live_luma).resize(ref_luma.shape[::-1], Image.BILINEAR)
        )
    if dist.shape != ref_luma.shape:
        from PIL import Image
        dist = np.asarray(Image.fromarray(dist).resize(ref_luma.shape[::-1], Image.NEAREST))
    if sky.shape != ref_luma.shape:
        from PIL import Image
        sky = np.asarray(
            Image.fromarray(sky.astype(np.uint8) * 255).resize(ref_luma.shape[::-1], Image.NEAREST)
        ) > 0
    # A block is sky when at least half its pixels are.
    keep = block(sky.astype(np.float32)) < 0.5
    l, r = block(live_luma), block(ref_luma)
    d = np.abs(l - r)
    w = 1.0 / np.maximum(block(dist), 0.5)
    d = np.where(keep, d, 0.0)
    r_kept = np.where(keep, r, 0.0)
    w = np.where(keep, w, 0.0)
    plain = float(d.sum() / (r_kept.sum() + 1e-6))
    near = float((w * d).sum() / ((w * r_kept).sum() + 1e-6))
    return near, plain


def parse_log(text):
    marks = []
    lamp = None
    rows = {}
    for line in text.splitlines():
        m = MARK.match(line.strip())
        if m:
            marks.append({"tag": m.group(1), "frame": int(float(m.group(2)))})
            continue
        m = LAMP.match(line.strip())
        if m:
            lamp = int(m.group(1))
            continue
        m = SC.search(line)
        if not m:
            continue
        frame = int(m.group(1))
        row = {
            "frame": frame,
            "extra": int(m.group(2)),
            "partial": int(m.group(3)),
            "sets": int(m.group(4)),
            "rays": int(m.group(5)),
            "near": int(m.group(6)),
            "full": int(m.group(7)),
        }
        # Two queues can print one frame. Keep the line that traced rays.
        prev = rows.get(frame)
        if prev is None or row["rays"] >= prev["rays"]:
            rows[frame] = row
    return marks, lamp, [rows[k] for k in sorted(rows)]


def frames_to_95(rows, before=None):
    """First engine frame with full/near >= 0.95, optionally before a lamp move."""
    for row in rows:
        if before is not None and row["frame"] >= before:
            break
        if row["near"] > 0 and row["full"] / row["near"] >= 0.95:
            return row["frame"]
    return None


def ray_window(rows, lo, hi):
    picked = [r for r in rows if lo <= r["frame"] <= hi]
    if not picked:
        return None
    rays = [r["rays"] for r in picked]
    above = [r for r in rays if r >= 1000]
    return {
        "frames": len(picked),
        "peak": max(rays),
        "mean": int(round(sum(rays) / len(rays))),
        "mean_above_1k": int(round(sum(above) / len(above))) if above else 0,
        "partial": sum(r["partial"] for r in picked),
        "sets": sum(r["sets"] for r in picked),
        "extra": sum(r["extra"] for r in picked),
    }


def find_capture(out, tag, kind):
    """tag-fNNNN-kind.png, the first of that tag."""
    hits = []
    for name in os.listdir(out):
        if name.startswith(tag + "-f") and name.endswith("-" + kind + ".png"):
            hits.append(name)
    if not hits:
        return None
    hits.sort()
    # Several still shots: caller passes an index via the filename list.
    return hits


def score_dir(out, reference, log_text):
    ref = load_rgb(reference)
    ref_l = srgb_to_linear(ref) @ LUM
    marks, lamp, rows = parse_log(log_text)
    shots = []
    for mark in marks:
        if mark["tag"] == "albedo":
            continue
        prefix = f"{mark['tag']}-f{mark['frame']:04d}"
        # The script pads to 4. A frame under 1000 matches. Fall back to a search.
        full = os.path.join(out, prefix + "-full.png")
        depth = os.path.join(out, prefix + "-depth.png")
        direct = os.path.join(out, prefix + "-direct.png")
        if not os.path.exists(full):
            continue
        live = lin_luma(full)
        dist = distance_from_depth(depth) if os.path.exists(depth) else np.full(live.shape, 5.0, np.float32)
        sky = sky_mask(ref, dist if dist.shape == ref.shape[:2] else np.full(ref.shape[:2], 5.0, np.float32))
        near, plain = ref_error(live, ref_l, dist, sky)
        item = {
            "tag": mark["tag"],
            "frame": mark["frame"],
            "near": round(near, 4),
            "plain": round(plain, 4),
            "full": os.path.basename(full),
        }
        if os.path.exists(direct):
            item["direct"] = os.path.basename(direct)
        shots.append(item)
    return {
        "marks": marks,
        "lamp_index": lamp,
        "frames_to_95_near": frames_to_95(rows),
        "rows": len(rows),
        "shots": shots,
        "trace": rows,
    }


def still_summary(scored):
    still = [s for s in scored["shots"] if s["tag"] == "still"]
    by = {s["frame"]: s["near"] for s in still}

    def closest(target):
        if not still:
            return None
        shot = min(still, key=lambda s: abs(s["frame"] - target))
        return {"frame": shot["frame"], "near": shot["near"], "asked": target}

    return {
        "frames_to_95_near": scored["frames_to_95_near"],
        "f40": closest(40),
        "f80": closest(80),
        "f160": closest(160),
        "still": [{"frame": s["frame"], "near": s["near"]} for s in still],
    }


def codes_darker(live_path, other_path, depth_path, albedo_path):
    """Share of near (<6 m) red-box pixels where live is >= 20 luma codes under other."""
    live = load_rgb(live_path)
    other = load_rgb(other_path)
    if other.shape != live.shape:
        from PIL import Image
        other = np.asarray(Image.fromarray((other * 255).astype(np.uint8)).resize(live.shape[1::-1], Image.BILINEAR)) / 255.0
    dist = distance_from_depth(depth_path) if depth_path and os.path.exists(depth_path) else None
    alb = load_rgb(albedo_path) if albedo_path and os.path.exists(albedo_path) else None
    if alb is not None and alb.shape != live.shape:
        from PIL import Image
        alb = np.asarray(Image.fromarray((alb * 255).astype(np.uint8)).resize(live.shape[1::-1], Image.NEAREST)) / 255.0
    ly = srgb_to_linear(live) @ LUM
    oy = srgb_to_linear(other) @ LUM
    # 8-bit Rec.709 luma of the stored sRGB. "20 codes" is this display step.
    w = np.array([0.2126, 0.7152, 0.0722])
    live8 = np.clip(np.round((live @ w) * 255.0), 0, 255)
    other8 = np.clip(np.round((other @ w) * 255.0), 0, 255)
    # Softer includes both sides. Darker is the one-sided count.
    gap = np.abs(other8 - live8)
    darker = (other8 - live8) >= 20
    near = np.ones(live.shape[:2], dtype=bool) if dist is None else dist < 6.0
    if alb is None:
        red = np.ones(live.shape[:2], dtype=bool)
    else:
        red = (alb[..., 0] > 0.45) & (alb[..., 1] < 0.35) & (alb[..., 2] < 0.35)
    sel = near & red
    n = int(sel.sum())
    hit = int((sel & darker).sum())
    return {
        "red_near_pixels": n,
        "darker_by_20": hit,
        "apart_by_20": int((sel & (gap >= 20)).sum()),
        "share_apart_20": round(float((sel & (gap >= 20)).sum()) / n, 4) if n else None,
        "share": round(hit / n, 4) if n else None,
        "mean_code_delta": round(float((other8 - live8)[sel].mean()), 2) if n else None,
        "mean_abs_codes": round(float(gap[sel].mean()), 2) if n else None,
        "near_luma_live": round(float(ly[sel].mean()), 4) if n else None,
        "near_luma_other": round(float(oy[sel].mean()), 4) if n else None,
    }


def write_heatmap(live_path, other_path, depth_path, albedo_path, out_path):
    """Grey picture. Red where the near red box is 20 or more luma codes apart."""
    from PIL import Image
    live = load_rgb(live_path)
    other = load_rgb(other_path)
    if other.shape != live.shape:
        other = np.asarray(Image.fromarray((other * 255).astype(np.uint8)).resize(live.shape[1::-1], Image.BILINEAR)) / 255.0
    w = np.array([0.2126, 0.7152, 0.0722])
    live8 = np.clip(np.round((live @ w) * 255.0), 0, 255)
    other8 = np.clip(np.round((other @ w) * 255.0), 0, 255)
    dist = distance_from_depth(depth_path) if depth_path and os.path.exists(depth_path) else None
    alb = load_rgb(albedo_path) if albedo_path and os.path.exists(albedo_path) else None
    if alb is not None and alb.shape != live.shape:
        alb = np.asarray(Image.fromarray((alb * 255).astype(np.uint8)).resize(live.shape[1::-1], Image.NEAREST)) / 255.0
    near = np.ones(live.shape[:2], dtype=bool) if dist is None else dist < 6.0
    red = np.ones(live.shape[:2], dtype=bool) if alb is None else (alb[..., 0] > 0.45) & (alb[..., 1] < 0.35) & (alb[..., 2] < 0.35)
    darker = np.abs(other8 - live8) >= 20
    img = np.stack([live8, live8, live8], axis=-1).astype(np.uint8)
    sel = near & red & darker
    img[sel] = (220, 30, 30)
    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    Image.fromarray(img).save(out_path)


def self_test():
    rng = np.random.default_rng(3)
    ref = rng.uniform(0.1, 0.5, (32, 32)).astype(np.float32)
    dist = np.full((32, 32), 20.0, np.float32)
    dist[:, :16] = 1.0
    sky = np.zeros((32, 32), dtype=bool)
    sky[:, -4:] = True
    near_bad = ref.copy()
    near_bad[:, :16] *= 1.5
    far_bad = ref.copy()
    far_bad[:, 16:] *= 1.5
    n1, _ = ref_error(near_bad, ref, dist, sky)
    n2, _ = ref_error(far_bad, ref, dist, sky)
    same, _ = ref_error(ref, ref, dist, sky)
    rows = [
        {"frame": 1, "near": 100, "full": 10, "rays": 100, "partial": 0, "sets": 1, "extra": 1},
        {"frame": 4, "near": 100, "full": 96, "rays": 50, "partial": 3, "sets": 1, "extra": 0},
    ]
    text = "\n".join([
        "MARK still frame=40",
        "LAMP3 index=3 dist2=0",
        "lightingsc gfx frame 1 still 0 relit 0 claimed 0 restarted 0 freed 0 atcap 0 live 1 thr 0 extra 1 partial 0 sets 1 rays 100 due 0 waiting 0 scouts 0 trickle 0 boost 0 checks 0 changed 0 near 100 full 10",
        "lightingsc gfx frame 4 still 0 relit 0 claimed 0 restarted 0 freed 0 atcap 0 live 1 thr 0 extra 0 partial 3 sets 1 rays 50 due 0 waiting 0 scouts 0 trickle 0 boost 0 checks 0 changed 0 near 100 full 96",
        "lightingsc gfx frame 4 still 0 relit 0 claimed 0 restarted 0 freed 0 atcap 0 live 1 thr 0 extra 0 partial 0 sets 0 rays 0 due 0 waiting 0 scouts 0 trickle 0 boost 0 checks 0 changed 0 near 0 full 0",
    ])
    marks, lamp, parsed = parse_log(text)
    ok = (
        same < 1e-6
        and n1 > 3 * n2
        and frames_to_95(rows) == 4
        and frames_to_95(parsed) == 4
        and parsed[1]["rays"] == 50
        and marks[0]["tag"] == "still"
        and lamp == 3
        and ray_window(parsed, 1, 4)["partial"] == 3
    )
    print(json.dumps({"near_bad": round(n1, 4), "far_bad": round(n2, 4), "ok": ok}))
    print("self-test", "ok" if ok else "FAIL")
    return 0 if ok else 1


def main(argv):
    if len(argv) >= 2 and argv[1] == "--self-test":
        return self_test()
    if len(argv) >= 2 and argv[1] == "score":
        out = argv[2]
        args = argv[3:]
        ref = args[args.index("--reference") + 1]
        label = args[args.index("--label") + 1]
        log_path = args[args.index("--log") + 1]
        json_path = args[args.index("--json") + 1]
        log_text = open(log_path, encoding="utf-8", errors="replace").read()
        report = os.path.join(out, "report.md")
        if os.path.exists(report):
            log_text += "\n" + open(report, encoding="utf-8", errors="replace").read()
        scored = score_dir(out, ref, log_text)
        summary = still_summary(scored)
        move = [s for s in scored["shots"] if s["tag"] != "still"]
        move_start = next((m["frame"] for m in scored["marks"] if m["tag"] == "move_start"), None)
        # 95% is the still fill. A later lamp move can drop the ratio.
        summary["frames_to_95_near"] = frames_to_95(scored["trace"], before=move_start)
        payload = {
            "label": label,
            "still": summary,
            "move_shots": [{k: s[k] for k in ("tag", "frame", "near", "full") if k in s} for s in move],
            "lamp_index": scored["lamp_index"],
            "trace_rows": scored["rows"],
            "move_start": move_start,
        }
        if move_start is not None:
            payload["move_trace"] = ray_window(scored["trace"], move_start, move_start + 40)
            payload["still_trace"] = ray_window(scored["trace"], 0, move_start)
            payload["move_frames"] = [
                {k: r[k] for k in ("frame", "partial", "sets", "extra", "rays", "near", "full")}
                for r in scored["trace"]
                if move_start <= r["frame"] <= move_start + 40
            ]
        else:
            payload["still_trace"] = ray_window(scored["trace"], 0, 10**9)
        # Drop the per-frame trace from the committed summary; the log keeps it.
        os.makedirs(os.path.dirname(json_path) or ".", exist_ok=True)
        with open(json_path, "w", encoding="utf-8") as f:
            json.dump(payload, f, indent=2)
            f.write("\n")
        print(json.dumps(payload, indent=2))
        return 0
    if len(argv) >= 2 and argv[1] == "lamp":
        off, on = argv[2], argv[3]
        ref = argv[argv.index("--reference") + 1]
        report = {"reference": ref, "pairs": []}
        # Pair mid/stop/after/hold full images by tag.
        def shots(directory):
            found = {}
            for name in os.listdir(directory):
                if not name.endswith("-full.png"):
                    continue
                tag = name.split("-f")[0]
                found.setdefault(tag, name)
            return found
        a, b = shots(off), shots(on)
        albedo = None
        for directory in (off, on):
            for name in os.listdir(directory):
                if name.startswith("albedo-") and name.endswith("-albedo.png"):
                    albedo = os.path.join(directory, name)
        for tag in ("mid", "stop", "after", "hold", "still"):
            if tag not in a or tag not in b:
                continue
            # still has several; compare each matching frame suffix.
            if tag == "still":
                continue
            live = os.path.join(on, b[tag])
            other = os.path.join(off, a[tag])
            depth = live.replace("-full.png", "-depth.png")
            item = {
                "tag": tag,
                "on": b[tag],
                "off": a[tag],
                "red_box": codes_darker(live, other, depth, albedo),
            }
            direct = live.replace("-full.png", "-direct.png")
            direct_off = other.replace("-full.png", "-direct.png")
            if os.path.exists(direct) and os.path.exists(direct_off):
                item["direct_red_box"] = codes_darker(direct, direct_off, depth, albedo)
            report["pairs"].append(item)
            if tag == "mid" and "--heatmap" in argv:
                heat = argv[argv.index("--heatmap") + 1]
                write_heatmap(live, other, depth, albedo, heat)
                report["heatmap"] = heat
        print(json.dumps(report, indent=2))
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
