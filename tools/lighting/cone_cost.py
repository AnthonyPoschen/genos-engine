#!/usr/bin/env python3
"""Frame cost of the cone-traced cascades against the old path at one view.

Starts genos-lighting headless under a memory cap, pauses scene time, and
reads `timing` over --frames frames with the old path, then with
cone_cascades=on once its field is resident. Also prints the seconds from
turning cone mode on to its first probes (the field's build and upload). Prints the GPU and CPU frame
milliseconds of each and the cone probe counts (`cone_stats`), and writes
them to --out/cost.json. A measurement, not a gate: it always exits 0 unless
the program could not be driven (UNSURE, exit 2).

  --view hall|sponza   the view (default hall; see cone_stability_gate.py)
  --size WxH           window size (default 1280x720)
  --frames N           frames to time (default 120)
  --bin PATH           genos-lighting binary (default target/release/genos-lighting)
  --port N             command port (default 18799)
  --out DIR            evidence directory (default genos-debug/cone-cost/<view>-<size>)
  -h, --help           this text
"""
import json
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
ARGS = {"view": "hall", "size": "1280x720", "frames": "120", "bin": "target/release/genos-lighting", "port": "18799", "out": ""}
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
SCENE, POS, YAW, PITCH = VIEWS[ARGS["view"]]
PORT = int(ARGS["port"])
OUT = os.path.abspath(ARGS["out"] or f"genos-debug/cone-cost/{ARGS['view']}-{ARGS['size']}")
os.makedirs(OUT, exist_ok=True)
FRAMES = int(ARGS["frames"])


def call(name, arg=None, timeout=300):
    req = urllib.request.Request(f"http://localhost:{PORT}/cmd/{name}", data=json.dumps(arg or {}).encode(), method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as reply:
            return json.loads(reply.read())
    except urllib.error.HTTPError as e:
        print(f"UNSURE: {name}: {e.read().decode()}")
        sys.exit(2)


def timed():
    call("wait", {"frames": FRAMES})
    return call("timing", {"frames": FRAMES})


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
                print(f"UNSURE: genos-lighting exited {proc.returncode}")
                sys.exit(2)
    call("time", {"pause": True})
    call("camera", {"position": POS, "yaw": YAW, "pitch": PITCH})
    call("wait_settled", {"timeout_frames": 900})
    old = timed()
    call("lighting.set_settings", {"cone_cascades": "on"})
    switched = time.time()
    for _ in range(4000):
        st = call("cone_stats")["cascades"]
        if st and st[0]["probes"] > 0:
            break
        call("wait", {"frames": 1})
    ready = time.time() - switched
    cone = timed()
    stats = call("cone_stats")["cascades"]
    result = {"view": ARGS["view"], "size": ARGS["size"], "old": old, "cone": cone, "cone_stats": stats, "ready_seconds": ready}
    with open(os.path.join(OUT, "cost.json"), "w") as f:
        json.dump(result, f, indent=1)
    keys = [k for k in old if isinstance(old[k], (int, float))]
    print(f"{ARGS['view']} {ARGS['size']}: " + ", ".join(f"{k} old {old[k]:.2f} cone {cone[k]:.2f}" for k in keys))
    print(f"  cone mode on to first probes: {ready:.2f} s")
    print("  probes " + " ".join(f"c{s['cascade']}={s['probes']}" for s in stats)
          + "  seat fallbacks " + " ".join(f"c{s['cascade']}={s['seat_fallbacks']}" for s in stats))
    call("quit")
finally:
    time.sleep(2)
    if proc.poll() is None:
        proc.terminate()
