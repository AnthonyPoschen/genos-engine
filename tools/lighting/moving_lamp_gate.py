#!/usr/bin/env python3
"""Moving-lamp bounce gate.

Starts genos-lighting headless on the lighting-stress scene, whose lamps move
every frame, and grades the room that no lamp lights directly (x 0..8, z 0..8).
Prints one PASS or FAIL line per check with the measured value, its limit, and
the value the broken build gave. Exit 0 when every check passes, 1 on a
failure, 3 when the gate cannot run (no binary, the program did not start).

  --bin PATH        genos-lighting binary (default target/release/genos-lighting)
  --port N          command port (default 18791)
  --out DIR         where the program writes its files (default genos-debug/moving-lamp-gate)
  --attach          use a program already listening on --port; do not start or stop one
  --only NAMES      comma list of checks to run (default all but leaks)
  --size WxH        picture size of the program it starts (default 1280x720)
  -h, --help        this text

The leaks check grades light from outside the room on its walls. The fault
needs pixels that land on the end of a surface, and at 1280x720 this pose has
none, so `make moving-lamp-gate` runs it alone at 2526x1376.

Needs a GPU. It opens one proof window that takes no focus; run it under a
private headless compositor to keep the desktop clear. Timing is not graded.
"""
import json
import math
import os
import statistics
import subprocess
import sys
import time
import urllib.request

ARGS = {"bin": "target/release/genos-lighting", "port": "18791", "out": "genos-debug/moving-lamp-gate", "only": "", "size": "1280x720"}
FLAGS = set()
argv = sys.argv[1:]
while argv:
    a = argv.pop(0)
    if a in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if a == "--attach":
        FLAGS.add(a[2:])
    elif a[2:] in ARGS and argv:
        ARGS[a[2:]] = argv.pop(0)
    else:
        print(f"unknown argument {a}", file=sys.stderr)
        print(__doc__, file=sys.stderr)
        sys.exit(2)
PORT = int(ARGS["port"])

# The pose of the report: inside the room, looking at its far wall.
ROOM = ([4.75393, 1.7, 7.51477], 0.0466, -0.079)
# Surfaces of that room: the floor, the wall at x = 8, the far corner, the far wall.
POINTS = {"floor": [4.75, 0.25, 5.0], "wall": [7.85, 1.5, 4.0], "corner": [0.2, 0.3, 0.2], "far wall": [4.0, 1.5, 0.15]}


def call(name, arg=None, timeout=900):
    req = urllib.request.Request(f"http://localhost:{PORT}/cmd/{name}", data=json.dumps(arg or {}).encode(), method="POST")
    with urllib.request.urlopen(req, timeout=timeout) as reply:
        return json.loads(reply.read())


RESULTS = []


def grade(name, value, ok, limit, before):
    RESULTS.append(ok)
    print(f"{'PASS' if ok else 'FAIL'} {name}: {value} (limit {limit}; the broken build gave {before})")


def enter_room():
    call("scene.time", {"pause": False})
    call("render.set_hdr_settings", {"exposure_mode": "fixed", "ev100": -1.4})
    call("scene.camera", {"position": ROOM[0], "yaw": ROOM[1], "pitch": ROOM[2]})
    call("scene.wait", {"frames": 400})


def voxel_series(samples, frames):
    """Luminance of each level-0 voxel near POINTS, one value per `frames` scene frames."""
    series = {}
    call("scene.time", {"pause": True})
    try:
        for _ in range(samples):
            call("scene.step", {"frames": frames})
            for where, point in POINTS.items():
                for v in call("lighting.level0_voxels", {"around": point, "radius": 0.4})["voxels"]:
                    key = (where, tuple(v["cell"]), v["level"], v["axis"])
                    series.setdefault(key, []).append(sum(v["radiance"]) / 3.0)
    finally:
        call("scene.time", {"pause": False})
    return {k: v for k, v in series.items() if len(v) == samples}


def noise_of(values):
    """Sample-to-sample noise over the mean. The second difference drops the slow
    change a moving lamp really makes; for independent noise its variance is 6 sigma^2."""
    mean = statistics.fmean(values)
    if mean <= 1e-6 or len(values) < 3:
        return 0.0
    second = [values[i - 1] - 2.0 * values[i] + values[i + 1] for i in range(1, len(values) - 1)]
    return math.sqrt(statistics.fmean(d * d for d in second) / 6.0) / mean


def check_room_voxels():
    # Level-0 voxels of the room no lamp lights directly, while the lamps move.
    enter_room()
    series = voxel_series(samples=24, frames=4)
    noise = sorted(noise_of(v) for v in series.values())
    if len(noise) < 20:
        grade("room voxels while the lamps move", f"only {len(noise)} voxels read", False, ">= 20 voxels", "n/a")
        return
    median = noise[len(noise) // 2]
    worst = noise[int(len(noise) * 0.9)]
    grade("room voxels while the lamps move, median noise", f"{median:.4f} of the mean over {len(noise)} voxels", median < 0.04, "< 0.04", "0.077")
    grade("room voxels while the lamps move, 90th percentile", f"{worst:.4f} of the mean", worst < 0.08, "< 0.08", "0.141")


def check_room_flicker():
    # The bounce picture of that room, lamps moving, exposure held.
    enter_room()
    reply = max((call("capture.flicker", {"frames": 60, "mode": "bounce"}) for _ in range(2)), key=lambda r: r["popped_share"])
    grade("room bounce picture while the lamps move", f"popped share {reply['popped_share']:.5f}, mean step {reply['mean_delta']:.5f}", reply["popped_share"] < 0.0003, "< 0.0003", "0.00075")


def check_room_follows():
    # The other side of the trade: the room must still follow its light. Put
    # the five point lamps out and the room has to darken. The lamps of this
    # scene move, so the settled value wanders by a few percent of the step:
    # the grade is the frame the picture is half way, not the last tenth.
    enter_room()
    lum = lambda: call("capture.luminance", {"mode": "bounce"})["luminance"]
    base = statistics.fmean(lum() for _ in range(4))
    for index in range(5):
        call("lighting.light", {"index": index, "on": False})
    seq = []
    call("scene.time", {"pause": True})
    try:
        for _ in range(60):
            call("scene.step", {"frames": 6})
            seq.append(lum())
    finally:
        call("scene.time", {"pause": False})
    finals = []
    for _ in range(6):
        call("scene.wait", {"frames": 150})
        finals.append(lum())
    final = statistics.fmean(finals[2:])
    for index in range(5):
        call("lighting.light", {"index": index, "reset": True})
    half = [i for i, v in enumerate(seq) if abs(v - final) <= 0.5 * abs(final - base)]
    frames = (half[0] + 1) * 6 if half else 999
    step = abs(final - base) / max(base, 1e-6)
    grade("room bounce follows its lamps going out", f"half way after {frames} frames (step {step:.2f})", step > 0.08 and frames <= 240, "<= 240 frames", "about 30; a drift rate of 1/16 gave about 330")


def check_room_leaks():
    # No lamp lights this room and it has no window, so no pixel of it is far
    # brighter than the rest and no wall voxel is blue. Sky light got in two
    # ways: a wall pixel under the roof read the voxel of a roof seam (a bright
    # streak along the top of the wall), and a voxel at the foot of a wall sat
    # a hair under the floor (a blue band).
    enter_room()
    call("scene.time", {"pause": True})
    try:
        call("scene.wait", {"frames": 120})
        reply = call("capture.luminance", {"mode": "full"})
        seen = {}
        for z in range(0, 17):
            for y in (0.1, 2.9):
                for v in call("lighting.level0_voxels", {"around": [7.85, y, z * 0.5], "radius": 0.4})["voxels"]:
                    seen[(tuple(v["cell"]), v["level"], v["axis"])] = v["radiance"]
    finally:
        call("scene.time", {"pause": False})
    ratio = reply["max"] / max(reply["luminance"], 1e-6)
    grade("room has no bright streak", f"brightest pixel {ratio:.2f} times the mean", ratio < 2.0, "< 2.0", "2.85")
    blue = max((r[2] / max(r[0], 1e-6) for r in seen.values()), default=0.0)
    grade("room wall voxels are not blue", f"largest blue over red {blue:.2f} over {len(seen)} voxels", len(seen) >= 20 and blue < 1.0, "< 1.0", "1.27")


CHECKS = [
    ("leaks", check_room_leaks),
    ("voxels", check_room_voxels),
    ("flicker", check_room_flicker),
    ("follows", check_room_follows),
]


def main():
    only = [n for n in ARGS["only"].split(",") if n]
    proc = None
    if "attach" not in FLAGS:
        if not os.access(ARGS["bin"], os.X_OK):
            print(f"UNSURE: {ARGS['bin']} is not built")
            return 3
        os.makedirs(ARGS["out"], exist_ok=True)
        log = open(os.path.join(ARGS["out"], "run.log"), "w")
        proc = subprocess.Popen([ARGS["bin"], "--headless", "--menu", "hidden", "--scene", "lighting-stress", "--preset", "high", "--size", ARGS["size"], "--mcp-port", str(PORT), "--out", ARGS["out"]], stdout=log, stderr=log)
    try:
        up = False
        for _ in range(600):
            try:
                if call("capture.luminance", timeout=5)["luminance"] > 0.01:
                    up = True
                    break
            except Exception:
                pass
            if proc is not None and proc.poll() is not None:
                break
            time.sleep(1)
        if not up:
            print("UNSURE: the program did not start or drew nothing")
            return 3
        for name, check in CHECKS:
            if name in only or (not only and name != "leaks"):
                check()
        call("render.set_hdr_settings", {"exposure_mode": "auto"})
    finally:
        if proc is not None:
            try:
                call("scene.quit", timeout=5)
            except Exception:
                pass
            try:
                proc.wait(timeout=20)
            except Exception:
                proc.kill()
    if not RESULTS:
        print("UNSURE: no check ran")
        return 3
    print(f"{sum(RESULTS)} of {len(RESULTS)} pass")
    return 0 if all(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())
