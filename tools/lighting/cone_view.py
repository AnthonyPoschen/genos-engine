#!/usr/bin/env python3
"""One cone-mode view, headless: its per-pass GPU milliseconds, its frame
time, and optionally captures and pixel picks.

Starts genos-lighting headless under a memory cap, pauses scene time, stands
at the pose, turns cone_cascades on, waits for the fine field, lets the light
settle, then prints `cone_stats.pass_ms` (the mean of --samples frames) and
`timing`, and writes them to --out/view.json. A measurement, not a gate:
exits 0 unless the program could not be driven (UNSURE, exit 2).

  --scene NAME         scene (default sponza)
  --pose X,Y,Z,YAW,PITCH  the camera (default: Anthony's Sponza editor pose)
  --size WxH           window size (default 1920x1080)
  --set KEY=VALUE      a lighting setting applied with cone mode (repeatable)
  --sun on|off         the suns (default on)
  --cone on|off        cone mode (default on; off captures the old path at
                       the same view, with no cone stage times)
  --ev EV100           hold the exposure fixed at this EV100 (default: the
                       scene's own metering; a fixed value makes two runs'
                       captures comparable)
  --settle SECONDS     wall seconds to settle before measuring (default 3)
  --samples N          cone_stats samples to average (default 20)
  --frames N           frames `timing` averages (default 120)
  --capture MODE       capture this mode to --out/<MODE>.png (repeatable;
                       full, albedo, depth, or any `capture` mode)
  --view NAME          also capture this cone_cascades.view (coverage, dark)
                       to --out/view-<NAME>.png (repeatable)
  --pick X,Y           print `cone_pick` for this pixel (repeatable)
  --bin PATH           genos-lighting binary (default target/release/genos-lighting)
  --port N             command port (default 18797)
  --out DIR            evidence directory (default genos-debug/cone-view)
  -h, --help           this text
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

ARGS = {"scene": "sponza", "pose": "11.35,1.8,-0.98,-1.435,0", "size": "1920x1080", "sun": "on", "cone": "on", "ev": "", "settle": "3",
        "samples": "20", "frames": "120", "bin": "target/release/genos-lighting", "port": "18797", "out": "genos-debug/cone-view"}
SETS, CAPTURES, PICKS, VIEWS = {}, [], [], []
argv = sys.argv[1:]
while argv:
    a = argv.pop(0)
    if a in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if a == "--set" and argv:
        key, _, value = argv.pop(0).partition("=")
        SETS[key] = value
    elif a == "--capture" and argv:
        CAPTURES.append(argv.pop(0))
    elif a == "--view" and argv:
        VIEWS.append(argv.pop(0))
    elif a == "--pick" and argv:
        PICKS.append([int(v) for v in argv.pop(0).split(",")])
    elif a.startswith("--") and a[2:] in ARGS and argv:
        ARGS[a[2:]] = argv.pop(0)
    else:
        print(f"unknown argument {a}\n{__doc__}", file=sys.stderr)
        sys.exit(2)
X, Y, Z, YAW, PITCH = map(float, ARGS["pose"].split(","))
PORT = int(ARGS["port"])
OUT = os.path.abspath(ARGS["out"])
os.makedirs(OUT, exist_ok=True)


def call(name, arg=None, timeout=300):
    req = urllib.request.Request(f"http://localhost:{PORT}/cmd/{name}", data=json.dumps(arg or {}).encode(), method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as reply:
            return json.loads(reply.read())
    except urllib.error.HTTPError as e:
        print(f"UNSURE: {name}: {e.read().decode()}")
        sys.exit(2)


log = open(os.path.join(OUT, "run.log"), "w")
cmd = ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=12G", "-p", "MemorySwapMax=0",
       os.path.abspath(ARGS["bin"]), "--headless", "--menu", "hidden", "--scene", ARGS["scene"], "--size", ARGS["size"],
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
    call("camera", {"position": [X, Y, Z], "yaw": YAW, "pitch": PITCH})
    if ARGS["sun"] == "off":
        for i, l in enumerate(call("lights")):
            if l.get("directional"):
                call("light", {"index": i, "on": False})
    if ARGS["ev"]:
        call("set_hdr_settings", {"exposure_mode": "fixed", "ev100": ARGS["ev"]})
    call("lighting.set_settings", dict(SETS, cone_cascades=ARGS["cone"]))
    for _ in range(600 if ARGS["cone"] == "on" else 0):
        reply = call("cone_stats")
        if reply["cascades"] and reply["cascades"][0]["probes"] > 0 and not reply.get("field", "").startswith("coarse"):
            break
        call("wait", {"frames": 5})
        time.sleep(0.5)
    else:
        if ARGS["cone"] == "on":
            sys.exit("UNSURE: the cone field never became resident")
    time.sleep(float(ARGS["settle"]))
    sums = {}
    samples = int(ARGS["samples"])
    for _ in range(samples):
        call("wait", {"frames": 2})
        for k, v in call("cone_stats").get("pass_ms", {}).items():
            sums[k] = sums.get(k, 0.0) + v / samples
    stats = call("cone_stats")
    call("wait", {"frames": int(ARGS["frames"])})
    timing = call("timing", {"frames": int(ARGS["frames"])})
    result = {"args": ARGS, "sets": SETS, "pass_ms": sums, "timing": timing, "cascades": stats["cascades"], "field": stats.get("field", "")}
    print(f"{ARGS['scene']} {ARGS['size']} pose {ARGS['pose']} {SETS}")
    print("  pass ms: " + "  ".join(f"{k} {v:.2f}" for k, v in sums.items()))
    print("  timing: " + ", ".join(f"{k} {v:.2f}" for k, v in timing.items() if isinstance(v, (int, float))))
    print("  probes " + " ".join(f"c{s['cascade']}={s['probes']}" for s in stats["cascades"])
          + "  traced " + " ".join(str(s["cones_traced"]) for s in stats["cascades"])
          + "  closed " + " ".join(str(s["cones_closed"]) for s in stats["cascades"])
          + "  seat fallbacks " + " ".join(str(s["seat_fallbacks"]) for s in stats["cascades"]))
    print(f"  field: {stats.get('field', '')}")
    print(f"  ev100: {call('get_hdr').get('ev100')}")
    for mode in CAPTURES:
        r = call("capture", {"path": f"{mode}.png", "mode": mode})
        print(f"  capture {mode}: {r.get('path')}")
    picks = []
    for x, y in PICKS:
        r = call("cone_pick", {"x": x, "y": y})
        for _ in range(10):
            if not r.get("pending"):
                break
            call("wait", {"frames": 2})
            r = call("cone_pick", {"x": x, "y": y})
        picks.append({"x": x, "y": y, "pick": r})
        print(f"  pick {x},{y}: lo {r.get('lo')} hi {r.get('hi')} hi_weight {r.get('hi_weight', 0):.2f} face {r.get('face')} "
              f"irradiance {[round(v, 4) for v in r.get('irradiance', [])]} field_sun {r.get('field_sun', 0):.2f}")
        for level in r.get("levels", []):
            print(f"    level {level['lod']}: kept {level['kept']}")
            for q in level["probes"]:
                print(f"      hash {int(q['in_hash'])} seated {int(q['seated'])} own {int(q['own'])} surface {int(q['surface'])} "
                      f"weight {q['weight']:.3f} passed {q['passed']:.2f} dark {q['dark_dirs']} lit {q['lit_dirs']} luma {q['mean_luma']:.4f} open {q.get('mean_open', 0):.2f} seat {[round(v, 2) for v in q['seat']]}")
    for view in VIEWS:
        call("lighting.set_settings", {"cone_cascades.view": view})
        call("wait", {"frames": 4})
        r = call("capture", {"path": f"view-{view}.png", "mode": "full"})
        print(f"  view {view}: {r.get('path')}")
    result["picks"] = picks
    with open(os.path.join(OUT, "view.json"), "w") as f:
        json.dump(result, f, indent=1)
    call("quit")
finally:
    time.sleep(2)
    if proc.poll() is None:
        proc.terminate()
