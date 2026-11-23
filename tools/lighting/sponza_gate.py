#!/usr/bin/env python3
"""Sponza lighting regression gate.

Starts genos-lighting headless on the Sponza scene and checks, over the command
port, each picture fault fixed on 23 Nov 2026. Prints one PASS or FAIL line per
check with the measured value, its limit, and the value the broken build gave.
Exit 0 when every check passes, 1 on a failure, 3 when the gate cannot run
(no binary, no asset, the program did not start).

  --bin PATH        genos-lighting binary (default target/release/genos-lighting)
  --port N          command port (default 18790)
  --out DIR         where the program writes its files (default genos-debug/sponza-gate)
  --attach          use a program already listening on --port; do not start or stop one
  --only NAMES      comma list of checks to run (default all)
  --reference       also grade the atrium against the CPU path trace (about 2 minutes)
  -h, --help        this text

Needs the Sponza asset (git lfs pull --include="assets/built/sponza/**") and a
GPU. It opens one proof window that takes no focus. Timing is not graded.
"""
import json
import math
import os
import subprocess
import sys
import time
import urllib.request

ARGS = {"bin": "target/release/genos-lighting", "port": "18790", "out": "genos-debug/sponza-gate", "only": ""}
FLAGS = set()
argv = sys.argv[1:]
while argv:
    a = argv.pop(0)
    if a in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if a in ("--attach", "--reference"):
        FLAGS.add(a[2:])
    elif a[2:] in ARGS and argv:
        ARGS[a[2:]] = argv.pop(0)
    else:
        print(f"unknown argument {a}", file=sys.stderr)
        print(__doc__, file=sys.stderr)
        sys.exit(2)
PORT = int(ARGS["port"])


def call(name, arg=None, timeout=900):
    req = urllib.request.Request(f"http://localhost:{PORT}/cmd/{name}", data=json.dumps(arg or {}).encode(), method="POST")
    with urllib.request.urlopen(req, timeout=timeout) as reply:
        return json.loads(reply.read())


def pose(position, yaw, pitch, settle=True):
    call("scene.camera", {"position": position, "yaw": yaw, "pitch": pitch})
    if settle:
        call("scene.wait_settled", {"timeout_frames": 900})


def voxels_in_view(position, yaw, pitch):
    f = [math.sin(yaw) * math.cos(pitch), math.sin(pitch), -math.cos(yaw) * math.cos(pitch)]
    seen = {}
    for d in (2, 3, 4, 5, 6):
        for dy in (-1.5, -0.5, 0.5):
            for sx in (-1.5, 0, 1.5):
                q = [position[0] + f[0] * d + sx * math.cos(yaw), position[1] + f[1] * d + dy, position[2] + f[2] * d + sx * math.sin(yaw)]
                for v in call("lighting.level0_voxels", {"around": q, "radius": 1.3})["voxels"]:
                    seen[(tuple(v["cell"]), v["level"], v["axis"])] = v
    return seen


RESULTS = []


def grade(name, value, ok, limit, before):
    RESULTS.append(ok)
    print(f"{'PASS' if ok else 'FAIL'} {name}: {value} (limit {limit}; the broken build gave {before})")


# ---- the checks -------------------------------------------------------------

def check_leaf_holes():
    # A masked hole left an empty G-buffer record: a white normal, at full emission.
    # No unit normal encodes to white, so a white pixel in the normal view is one.
    pose([-11.6, 1.6, -0.4], 0.38051, -0.45979)
    top = call("capture.luminance", {"mode": "normal"})["max"]
    grade("leaf holes keep the surface behind them", f"normal view max {top:.3f}", top < 0.97, "< 0.97", "1.000")


def check_leaves_and_floor_colour():
    # Leaves shaded black (metallic factor 1 with no texel); the tiled floor drew
    # untextured and black where u was negative.
    call("scene.jump_checkpoint", {"name": "atrium"})
    call("scene.wait", {"frames": 30})
    share = call("capture.luminance", {"mode": "albedo"})["black_share"]
    grade("atrium albedo has no black leaves", f"black share {share:.4f}", share < 0.03, "< 0.03", "0.054")
    call("scene.jump_checkpoint", {"name": "dark-corner"})
    call("scene.wait", {"frames": 30})
    share = call("capture.luminance", {"mode": "albedo", "region": [0.3, 0.75, 0.6, 0.25]})["black_share"]
    grade("dark-corner floor is textured", f"black share {share:.4f}", share < 0.02, "< 0.02", "about 1.0")


def check_still_flicker():
    # Patches of bounce light that came and went on a still picture.
    for name, limit, before in (("atrium", 0.004, "0.001 to 0.009"), ("dark-corner", 0.01, "0.03 to 0.11"), ("curtains", 0.02, "0.10 to 0.30")):
        call("render.set_hdr_settings", {"exposure_mode": "auto"})
        call("scene.jump_checkpoint", {"name": name})
        call("scene.wait", {"frames": 240})
        # Hold the exposure the meter chose, so the grade is the light and not
        # the exposure still easing in after the jump.
        call("render.set_hdr_settings", {"exposure_mode": "fixed", "ev100": call("render.get_hdr")["ev100"]})
        call("scene.wait", {"frames": 60})
        worst = max(call("capture.flicker", {"frames": 60, "mode": "bounce"})["popped_share"] for _ in range(2))
        call("render.set_hdr_settings", {"exposure_mode": "auto"})
        grade(f"still camera, bounce view, {name}", f"popped share {worst:.4f}", worst < limit, f"< {limit}", before)


def check_pan():
    # While the camera turned, each voxel re-picked its seat and its light changed.
    p, yaw, pitch = [-6.47298, 1.8, 0.14095], -1.5312, -0.03
    probe = [p[0] + 3.2 * math.sin(yaw), p[1] + 2.4, p[2] - 3.2 * math.cos(yaw)]
    pose(p, yaw, pitch)
    call("scene.time", {"pause": True})
    prev, kept, seated, changed = None, 0, 0, 0
    try:
        for k in list(range(0, 13)) + list(range(11, -1, -1)):
            call("scene.camera", {"position": p, "yaw": yaw + 0.006 * k, "pitch": pitch})
            call("scene.step", {"frames": 1})
            vox = {(tuple(v["cell"]), v["level"], v["axis"]): (v["position"], sum(v["radiance"]) / 3) for v in call("lighting.level0_voxels", {"around": probe, "radius": 1.0})["voxels"]}
            if prev is not None:
                for key, (at, light) in vox.items():
                    if key not in prev:
                        continue
                    kept += 1
                    seated += max(abs(a - b) for a, b in zip(at, prev[key][0])) < 1e-4
                    changed += abs(light - prev[key][1]) / max(light, prev[key][1], 1e-6) > 0.1
            prev = vox
    finally:
        call("scene.time", {"pause": False})
    if kept == 0:
        grade("pan under the vault", "no voxels read", False, "some voxels", "-")
        return
    grade("pan: voxels keep their seat", f"{seated} of {kept} ({seated / kept:.3f})", seated / kept > 0.95, "> 0.95", "0.095")
    grade("pan: voxel light holds", f"{changed} of {kept} changed by a tenth ({changed / kept:.3f})", changed / kept < 0.04, "< 0.04", "0.17")


def check_sky_voxels():
    # Indoor voxels with no usable parent read the sky colour (blue over red 1.67).
    for p, yaw, pitch, before in (([11.79419, 6.53626, 0.32577], -0.3132, -0.222, "0.180"), ([10.69568, 6.61347, -3.47993], -1.1412, -0.28, "0.112"), ([8, 1.5, 3.5], 2.356, -0.1, "cool patches")):
        pose(p, yaw, pitch)
        seen = voxels_in_view(p, yaw, pitch)
        sky = sum(1 for v in seen.values() if v["radiance"][2] / max(v["radiance"][0], 1e-6) > 0.6)
        share = sky / max(len(seen), 1)
        grade(f"indoor voxels do not read the sky at {[round(x, 1) for x in p]}", f"{sky} of {len(seen)} ({share:.3f})", len(seen) > 100 and share < 0.02, "< 0.02", before)


def check_void():
    # Inside the wall, the backs of one-sided surfaces showed the other side's light.
    pose([13.63523, 10.04472, -7.5], 4.2438, -0.441)
    share = call("capture.luminance", {"mode": "full"})["black_share"]
    grade("the void behind the gallery is dark", f"black share {share:.3f}", share > 0.4, "> 0.4", "about 0")


def check_lamp_response():
    # Bounce has to follow a moved lamp, with no view switch in the measurement.
    call("render.set_hdr_settings", {"exposure_mode": "fixed", "ev100": 0})
    call("scene.jump_checkpoint", {"name": "atrium"})
    call("capture.view", {"mode": "bounce"})
    call("scene.wait", {"frames": 300})
    lum = lambda: call("capture.luminance")["luminance"]
    base = lum()
    call("scene.time", {"pause": True})
    try:
        call("lighting.light", {"index": 1, "position": [4, 6, -0.4]})
        seq = []
        for _ in range(30):
            call("scene.step", {"frames": 3})
            seq.append(lum())
    finally:
        call("scene.time", {"pause": False})
    call("scene.wait", {"frames": 600})
    final = lum()
    call("lighting.light", {"index": 1, "reset": True})
    call("capture.view", {"mode": "full"})
    call("render.set_hdr_settings", {"exposure_mode": "auto"})
    within = [i for i, v in enumerate(seq) if abs(v - final) <= 0.1 * abs(final - base) + 1e-4]
    frames = (within[0] + 1) * 3 if within else 999
    step = abs(final - base) / max(base, 1e-6)
    grade("bounce follows a moved lamp", f"within a tenth of the step after {frames} frames (step {step:.2f})", step > 0.1 and frames <= 30, "<= 30 frames", "69")


def check_jump_exposure():
    # A jump kept the old exposure: dark corner to atrium started four times over.
    call("scene.jump_checkpoint", {"name": "dark-corner"})
    call("scene.wait_settled", {"timeout_frames": 900})
    call("scene.wait", {"frames": 120})
    call("scene.time", {"pause": True})
    try:
        call("scene.jump_checkpoint", {"name": "atrium"})
        seen = []
        for _ in range(20):
            call("scene.step", {"frames": 2})
            seen.append(call("render.get_hdr")["exposure"])
    finally:
        call("scene.time", {"pause": False})
    call("scene.wait", {"frames": 240})
    settled = call("render.get_hdr")["exposure"]
    peak = max(seen) / max(settled, 1e-6)
    grade("exposure after a jump rises to the new view", f"peak {peak:.2f} of the settled exposure", peak <= 1.05, "<= 1.05", "3.9")


def check_reference():
    call("scene.jump_checkpoint", {"name": "atrium"})
    call("scene.wait_settled", {"timeout_frames": 900})
    call("scene.time", {"pause": True})
    try:
        d = call("capture.compare", {"prefix": "gate-atrium", "seconds": 120, "width": 320, "height": 180})
    finally:
        call("scene.time", {"pause": False})
    grade("atrium against the path trace", f"error {d['mean_rel']:.3f}, coverage {d['coverage']}", d["coverage"] > 0.9 and d["mean_rel"] < 0.25, "< 0.25", "0.38")


CHECKS = [
    ("leaf-holes", check_leaf_holes),
    ("colour", check_leaves_and_floor_colour),
    ("still", check_still_flicker),
    ("pan", check_pan),
    ("sky", check_sky_voxels),
    ("void", check_void),
    ("lamp", check_lamp_response),
    ("jump", check_jump_exposure),
]
if "reference" in FLAGS:
    CHECKS.append(("reference", check_reference))


def main():
    only = [n for n in ARGS["only"].split(",") if n]
    proc = None
    if "attach" not in FLAGS:
        if not os.access(ARGS["bin"], os.X_OK):
            print(f"UNSURE: {ARGS['bin']} is not built")
            return 3
        asset = "assets/built/sponza/sponza.gltf"
        if not os.path.exists(asset) or os.path.getsize(asset) < 1024:
            print(f"UNSURE: {asset} is missing or is a Git LFS pointer")
            return 3
        os.makedirs(ARGS["out"], exist_ok=True)
        log = open(os.path.join(ARGS["out"], "run.log"), "w")
        proc = subprocess.Popen([ARGS["bin"], "--headless", "--menu", "hidden", "--scene", "sponza", "--checkpoint", "atrium", "--preset", "high", "--size", "1280x720", "--mcp-port", str(PORT), "--out", ARGS["out"]], stdout=log, stderr=log)
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
            print("UNSURE: the program did not start or the scene did not load")
            return 3
        for name, check in CHECKS:
            if only and name not in only:
                continue
            try:
                check()
            except Exception as err:
                RESULTS.append(False)
                print(f"FAIL {name}: the check did not finish: {err}")
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
    print(f"{sum(RESULTS)} of {len(RESULTS)} checks passed")
    return 0 if RESULTS and all(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())
