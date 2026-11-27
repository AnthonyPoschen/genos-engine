#!/usr/bin/env python3
"""Cone-traced cascades stability gate (plan section 18, step 3).

Starts genos-lighting headless under a memory cap at one view, with scene time
paused so nothing moves but the camera, and grades cone mode
(cone_cascades=on):

  A  still camera: the share of pixels that pop frame to frame over 60 frames
     is at most the old path's at the same view (--flicker-margin added)
  B  nothing dropped: every cascade's dropped_compact and dropped_insert is 0
     (lighting.cone_stats)
  C  stable seats: after a 5 degree yaw, and after a 10 cm step, every probe of
     cascade 0 that has the same world cell (lod and cell) at both poses keeps
     its seat within 1 mm (lighting.cone_probes)
  D  return to pose: turn 30 degrees away and back, with exposure held fixed,
     and let the bounce's short history settle (1.5 s each way); no pixel
     differs by more than two display codes, and at most one in 10,000 by
     two (the bounce is stored in halves: after a history its light can rest
     a half's step from where it would have). The old path's
     count is printed for reference only: its caches carry the turn.
  E  a lamp follows in one frame: move a point lamp 0.5 m; the frame drawn
     right after holds at least half of the change (summed luminance) the
     picture shows a second later (its direct light does not wait; its
     bounce, a short history, settles over a few tenths of a second), and the
     lamp hash dropped no point (lighting.cone_stats lamp_dropped)
  F  moving camera: step the camera sideways 1 cm a frame for --steps steps,
     exposure held fixed and the suns off (a sun's hard pixel-ray shadow edge
     aliases as it slides; that is the picture's edge, not the light popping); count pixels whose second difference over three
     steps, |f(i+1) - 2 f(i) + f(i-1)|, is over 6 codes where neither depth,
     albedo nor normal changed (a slide over a gradient is near 0, a pop is
     not; the normal view sees a metal's edges, which its black diffuse
     albedo does not).
     The mean over the walk is at most --pop-ratio times the old path's on
     the same walk. Before section 18 step 4b: 5 to 7 times in the hall
  G  a clip rebuild does not blink: walk 4 m forward in 10 cm steps, a frame
     each, past the 2 m at which the fine levels are built again about the
     eye; the field is rebuilt (cone_stats field changes) and no frame's
     mean luminance falls under 80% of the mean of the frames either side

Views: --view hall (lighting-stress, the hall checkpoint) or --view sponza
(the arcade pose of section 18). Each gate prints PASS or FAIL; the exit code
is 0 only when all pass. Evidence (pictures, stats, probe dumps, a summary)
goes to --out.

  --view hall|sponza   the view (default hall)
  --size WxH           window size (default 1280x720)
  --bin PATH           genos-lighting binary (default target/release/genos-lighting)
  --port N             command port (default 18797)
  --out DIR            evidence directory (default genos-debug/cone-stability/<view>-<size>)
  --only A,B,C,D,E,F,G run these gates only
  --steps N            gate F's 1 cm steps (default 30)
  --pop-ratio F        gate F's limit against the old path (default 2.5)
  --flicker-margin F   added to the old path's popped share in gate A (default 0.002)
  --reflections on|off screen reflections during the gates (default on)
  --fog on|off         froxel fog during the gates (default off). Fog keeps its own
                       history and in Sponza moves a few pixels by one code from
                       frame to frame; the gates grade the cone lighting
  -h, --help           this text
"""
import json
import math
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

VIEWS = {
    "hall": ("lighting-stress", [14.5, 1.7, 23.5], -0.3554, 0.08),
    "sponza": ("sponza", [-13.39384, 2.12628, -4.41117], -4.62119, -0.102),
}
ARGS = {"view": "hall", "size": "1280x720", "bin": "target/release/genos-lighting", "port": "18797", "out": "",
        "only": "A,B,C,D,E,F,G", "flicker-margin": "0.002", "steps": "30", "pop-ratio": "2.5", "fog": "off", "reflections": "on"}
argv = sys.argv[1:]
while argv:
    a = argv.pop(0)
    if a in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if a.startswith("--") and a[2:] in ARGS and argv:
        ARGS[a[2:]] = argv.pop(0)
    else:
        print(f"unknown argument {a}\n{__doc__}", file=sys.stderr)
        sys.exit(2)
if ARGS["view"] not in VIEWS:
    sys.exit(f"--view is hall or sponza, not {ARGS['view']}")
SCENE, POS, YAW, PITCH = VIEWS[ARGS["view"]]
PORT = int(ARGS["port"])
OUT = os.path.abspath(ARGS["out"] or f"genos-debug/cone-stability/{ARGS['view']}-{ARGS['size']}")
ONLY = set(ARGS["only"].split(","))
os.makedirs(OUT, exist_ok=True)
RESULTS = []


def call(name, arg=None, timeout=300):
    req = urllib.request.Request(f"http://localhost:{PORT}/cmd/{name}", data=json.dumps(arg or {}).encode(), method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as reply:
            return json.loads(reply.read())
    except urllib.error.HTTPError as e:
        raise SystemExit(f"UNSURE: {name} {arg}: {e.read().decode()}")


def save(name, value):
    with open(os.path.join(OUT, name), "w") as f:
        json.dump(value, f, indent=1)


def grade(name, ok, detail):
    RESULTS.append(ok)
    print(f"{'PASS' if ok else 'FAIL'} {ARGS['view']} {ARGS['size']} {name}: {detail}", flush=True)


def pose(yaw=YAW, pos=POS, frames=20):
    call("camera", {"position": pos, "yaw": yaw, "pitch": PITCH})
    call("wait", {"frames": frames})


def shot(name):
    r = call("capture", {"path": f"{name}.png"})
    return r["path"] if os.path.isabs(r["path"]) else os.path.join(OUT, r["path"])


def cells(probes):
    return {(p["lod"], p.get("face", 0), tuple(p["cell"])): p["seat"] for p in probes}


def seat_moves(a, b):
    shared = set(a) & set(b)
    worst = max((math.dist(a[k], b[k]) for k in shared), default=0.0)
    moved = sum(1 for k in shared if math.dist(a[k], b[k]) > 0.001)
    return len(shared), moved, worst


def differing(path_a, path_b, over=1):
    """Pixels that differ at all, and by more than `over` display codes."""
    from PIL import Image, ImageChops
    a, b = Image.open(path_a).convert("RGB"), Image.open(path_b).convert("RGB")
    diff = ImageChops.difference(a, b)
    data = diff.get_flattened_data() if hasattr(diff, "get_flattened_data") else diff.getdata()
    return sum(1 for p in data if max(p) > 0), sum(1 for p in data if max(p) > over), a.width * a.height


def settle(seconds=1.5):
    """Draw for `seconds` of real time: the cone bounce's short history, and
    other passes' (reflections, fog), step by frame time."""
    started = time.time()
    while time.time() - started < seconds:
        call("wait", {"frames": 10})


def turn_back_changes(label):
    # Let the earlier gates' camera moves settle out before the first shot.
    pose(frames=120)
    settle()
    before = shot(f"d-{label}-before")
    albedo_before = call("capture", {"path": f"d-{label}-albedo-before.png", "mode": "albedo"})["path"]
    pose(yaw=YAW + math.radians(30))
    settle()
    pose(frames=60)
    settle()
    after = shot(f"d-{label}-after")
    albedo_after = call("capture", {"path": f"d-{label}-albedo-after.png", "mode": "albedo"})["path"]
    full = os.path.join
    changed, far, total = differing(before, after)
    _, very_far, _ = differing(before, after, over=2)
    albedo, _, _ = differing(*(p if os.path.isabs(p) else full(OUT, p) for p in (albedo_before, albedo_after)))
    return changed, far, total, albedo, very_far


def moving_pops(label):
    """Gate F: the mean count of popping pixels a step on a sideways walk,
    with the suns off: a sun's exact pixel ray gives a hard shadow edge that
    aliases as it slides, which is the picture's edge, not the light popping
    (Anthony, 25 Nov 2026)."""
    import numpy as np
    from PIL import Image
    suns = [i for i, l in enumerate(call("lights")) if l.get("directional") and l.get("on", True)]
    for i in suns:
        call("light", {"index": i, "on": False})
    # The suns' bounce fades over the bounce's history, in real time: let it
    # go before the walk, or the first step counts the fade as pops (a frame
    # of 9 ms left the walk's first frames still darkening, 25 Nov 2026).
    settle()
    right = [math.cos(YAW), 0.0, math.sin(YAW)]
    frames = []
    for i in range(int(ARGS["steps"])):
        pose(pos=[POS[k] + right[k] * 0.01 * i for k in range(3)], frames=3)
        views = []
        for mode in ("full", "depth", "albedo", "normal"):
            r = call("capture", {"path": f"f-{label}-{i:03}-{mode}.png", "mode": mode})["path"]
            path = r if os.path.isabs(r) else os.path.join(OUT, r)
            views.append(np.asarray(Image.open(path).convert("L"), dtype=np.int16))
        frames.append(views)
    counts = []
    for i in range(1, len(frames) - 1):
        (a, da, aa, na), (b, db, ab, nb), (c, dc, ac, nc) = frames[i - 1], frames[i], frames[i + 1]
        second = np.abs(c - 2 * b + a)
        # The normal view too: a metal's albedo view is black (its diffuse
        # albedo is base colour times one less its metallic), so a thin metal
        # rod's highlight sliding over a wall of like depth was not seen to
        # move, and counted as popping light (Sponza, a curtain rod: 5,500
        # pixels a step of 11,300).
        moved = (np.abs(db - da) > 2) | (np.abs(dc - db) > 2) | (np.abs(ab - aa) > 2) | (np.abs(ac - ab) > 2) | (np.abs(nb - na) > 2) | (np.abs(nc - nb) > 2)
        counts.append(int(((second > 6) & ~moved).sum()))
    for i in suns:
        call("light", {"index": i, "on": True})
    pose(frames=30)
    return sum(counts) / max(len(counts), 1), max(counts, default=0)


def wait_cone_ready():
    # The first field holds the coarse levels alone; wait for the fine ones.
    for _ in range(240):
        reply = call("cone_stats")
        stats = reply["cascades"]
        if stats and stats[0]["probes"] > 0 and not reply.get("field", "").startswith("coarse"):
            call("wait", {"frames": 5})
            return stats
        call("wait", {"frames": 10})
        time.sleep(1)
    raise SystemExit("UNSURE: the cone field never became resident")


log = open(os.path.join(OUT, "run.log"), "w")
cmd = ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=12G", "-p", "MemorySwapMax=0",
       os.path.abspath(ARGS["bin"]), "--headless", "--menu", "hidden", "--scene", SCENE, "--size", ARGS["size"],
       "--mcp-port", str(PORT), "--out", OUT]
proc = subprocess.Popen(cmd, stdout=log, stderr=log)
try:
    for _ in range(240):
        time.sleep(1)
        try:
            if call("luminance", timeout=3).get("luminance", 0) > 0.01:
                break
        except Exception:
            if proc.poll() is not None:
                sys.exit(f"UNSURE: genos-lighting exited {proc.returncode}; see {OUT}/run.log")
    else:
        sys.exit("UNSURE: the scene did not load")
    call("time", {"pause": True})
    if ARGS["fog"] == "off":
        call("set_fog_settings", {"enabled": "off"})
    if ARGS["reflections"] == "off":
        call("set_reflection_settings", {"enabled": "off"})
    pose(frames=60)
    old_turn = None
    if "D" in ONLY or "E" in ONLY or "F" in ONLY:
        # Exposure held fixed: D and E grade the lighting, not the meter.
        ev = call("get_hdr")["ev100"]
        call("set_hdr_settings", {"exposure_mode": "fixed", "ev100": str(ev)})
    if "D" in ONLY:
        old_turn = turn_back_changes("old")
    old_pops = moving_pops("old") if "F" in ONLY else None
    old_flicker = None
    if "A" in ONLY:
        call("wait_settled", {"timeout_frames": 900})
        old_flicker = call("flicker", {"frames": 60, "heatmap": "old-flicker.png"})
    call("lighting.set_settings", {"cone_cascades": "on"})
    wait_cone_ready()
    pose(frames=30)
    stats = call("cone_stats")["cascades"]
    save("cone-stats.json", stats)
    if "A" in ONLY:
        cone = call("flicker", {"frames": 60, "heatmap": "cone-flicker.png"})
        limit = old_flicker["popped_share"] + float(ARGS["flicker-margin"])
        grade("A still camera", cone["popped_share"] <= limit,
              f"cone popped share {cone['popped_share']:.4f}, old path {old_flicker['popped_share']:.4f} (limit {limit:.4f})")
    if "B" in ONLY:
        dropped = [(s["cascade"], s["dropped_compact"], s["dropped_insert"], s["probes"], s["cap"]) for s in stats]
        bad = [d for d in dropped if d[1] or d[2]]
        detail = "; ".join(f"c{c}: {p} probes (cap {cap}), {dc} past the cap, {di} insert misses" for c, dc, di, p, cap in dropped)
        grade("B nothing dropped", not bad, detail)
    if "C" in ONLY:
        base = cells(call("cone_probes", {"cascade": 0})["probes"])
        pose(yaw=YAW + math.radians(5))
        turned = cells(call("cone_probes", {"cascade": 0})["probes"])
        right = [math.cos(YAW), 0.0, math.sin(YAW)]
        pose()
        pose(pos=[POS[0] + 0.1 * right[0], POS[1], POS[2] + 0.1 * right[2]])
        stepped = cells(call("cone_probes", {"cascade": 0})["probes"])
        save("seats.json", {"base": len(base), "turned": len(turned), "stepped": len(stepped)})
        for label, other in (("5 degree yaw", turned), ("10 cm step", stepped)):
            shared, moved, worst = seat_moves(base, other)
            grade(f"C seats after a {label}", shared > 0 and moved == 0,
                  f"{moved} of {shared} shared cells moved their seat over 1 mm (worst {worst * 1000:.1f} mm)")
        pose()
    if "D" in ONLY:
        changed, far, total, albedo, very_far = turn_back_changes("cone")
        grade("D return to pose", very_far == 0 and far * 10_000 <= total,
              f"{far} of {total} pixels differ by more than one code after turning away and back, {very_far} by more than two "
              f"(limits: one in 10,000, none; the bounce is stored in halves, and after a history its light can rest a half's "
              f"step from where it would have; {changed} at all; the albedo view changed {albedo}; old path {old_turn[0]})")
    if "E" in ONLY:
        from PIL import Image, ImageChops
        lamps = [i for i, l in enumerate(call("lights")) if not l.get("directional")]
        if not lamps:
            grade("E lamp follows in one frame", False, "the view has no point lamp")
        else:
            i = lamps[0]
            # Fog keeps a history of its own; let the turn of gate D settle out.
            call("wait", {"frames": 60})
            p = call("lights")[i]["position"]
            still = shot("e-before")
            call("light", {"index": i, "position": [p[0] + 0.5, p[1], p[2]]})
            call("wait", {"frames": 1})
            first = shot("e-first")
            # The bounce is a short history stepped by frame time: let a second pass.
            settle(1.0)
            later = shot("e-later")
            import numpy as np
            from PIL import Image as _Image
            def lum(path):
                return np.asarray(_Image.open(path).convert("L"), dtype=np.float64)
            a, b, c = lum(still), lum(first), lum(later)
            total = np.abs(c - a).sum()
            at_once = np.abs(b - a).sum()
            share = at_once / total if total > 0 else 0.0
            _, moved, _ = differing(still, later)
            dropped = call("cone_stats").get("lamp_dropped", 0)
            grade("E lamp follows in one frame", moved > 0 and share >= 0.5 and dropped == 0,
                  f"{moved} pixels changed with the move; the first frame after it holds {share:.0%} of the change (at least half: its direct light does not wait, its bounce settles over a few tenths of a second); {dropped} lamp points dropped")
    if "F" in ONLY:
        cone_pops = moving_pops("cone")
        limit = old_pops[0] * float(ARGS["pop-ratio"])
        grade("F moving camera", cone_pops[0] <= limit,
              f"cone {cone_pops[0]:.0f} popping pixels a 1 cm step (worst {cone_pops[1]}), old path {old_pops[0]:.0f} (worst {old_pops[1]}); limit {limit:.0f}")
    if "G" in ONLY:
        # Forward, down the open view: sideways runs into a column in both views.
        ahead = [math.sin(YAW), 0.0, -math.cos(YAW)]  # look_direction (camera.rs)
        pose(frames=30)
        before = call("cone_stats").get("field", "")
        lum = []
        for i in range(41):
            call("camera", {"position": [POS[k] + ahead[k] * 0.1 * i for k in range(3)], "yaw": YAW, "pitch": PITCH})
            call("wait", {"frames": 1})
            lum.append(call("luminance")["luminance"])
        call("wait", {"frames": 60})
        after = call("cone_stats").get("field", "")
        dips = [i for i in range(1, len(lum) - 1) if lum[i] < 0.8 * 0.5 * (lum[i - 1] + lum[i + 1])]
        save("g-luminance.json", lum)
        grade("G clip rebuild", after != before and not dips,
              f"rebuilt: {after != before}; frames dipping under 80% of their neighbours: {dips}; luminance {min(lum):.4f} to {max(lum):.4f}")
        pose()
    call("quit")
finally:
    time.sleep(2)
    if proc.poll() is None:
        proc.terminate()
sys.exit(0 if RESULTS and all(RESULTS) else 1)
