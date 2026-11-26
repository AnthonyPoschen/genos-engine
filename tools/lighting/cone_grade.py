#!/usr/bin/env python3
"""Grade cone mode and the old path against a converged reference
(plan section 18, step 6; section 13 has the prototype's numbers).

For each view: starts genos-lighting headless under a memory cap, pauses
scene time, stands at the checkpoint, and runs `compare` (a CPU path trace of
the frame, unlimited bounce, reused from target/reference-cache when it is as
converged) on the old path, then with cone_cascades=on once its fine field is
resident and has settled. Prints each view's mean_rel (the share of the light
that is wrong) and bias, writes them to --out/grade.json, and keeps the
reference, live and heat-map pictures. Prints PASS or FAIL against --limit
for cone mode in each view (section 13's gate is 10%); exits 0 only when all
pass, 2 when the program could not be driven.

  --views LIST       comma-separated: hall, room-a, atrium, dark-corner
                     (default all four)
  --width N          reference width in pixels (default 160; at most 320)
  --spp N            paths a pixel each pass (default 16)
  --noise F          stop when the reference's mean relative noise is under
                     this (default 0.05)
  --seconds N        most seconds a reference traces (default 600)
  --block N          compare block side in pixels (default 2)
  --limit F          cone mode's mean_rel limit (default 0.10)
  --bin PATH         genos-lighting binary (default target/release/genos-lighting)
  --port N           command port (default 18782)
  --out DIR          evidence directory (default genos-debug/cone-grade)
  --set KEY=VALUE    a lighting setting for cone mode, e.g.
                     cone_cascades.probe_px=16 (repeatable)
  -h, --help         this text
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

VIEWS = {
    "hall": ("lighting-stress", "hall"),
    "room-a": ("lighting-stress", "room-a"),
    "atrium": ("sponza", "atrium"),
    "dark-corner": ("sponza", "dark-corner"),
}
ARGS = {"views": ",".join(VIEWS), "width": "160", "spp": "16", "noise": "0.05", "seconds": "600", "block": "2",
        "limit": "0.10", "bin": "target/release/genos-lighting", "port": "18782", "out": "genos-debug/cone-grade"}
SETS = {}
argv = sys.argv[1:]
while argv:
    a = argv.pop(0)
    if a in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if a == "--set" and argv:
        key, _, value = argv.pop(0).partition("=")
        SETS[key] = value
    elif a.startswith("--") and a[2:] in ARGS and argv:
        ARGS[a[2:]] = argv.pop(0)
    else:
        print(f"unknown argument {a}\n{__doc__}", file=sys.stderr)
        sys.exit(2)
PORT = int(ARGS["port"])
OUT = os.path.abspath(ARGS["out"])
os.makedirs(OUT, exist_ok=True)


def call(name, arg=None, timeout=1800):
    req = urllib.request.Request(f"http://localhost:{PORT}/cmd/{name}", data=json.dumps(arg or {}).encode(), method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as reply:
            return json.loads(reply.read())
    except urllib.error.HTTPError as e:
        print(f"UNSURE: {name}: {e.read().decode()}")
        sys.exit(2)


def settle(seconds):
    started = time.time()
    while time.time() - started < seconds:
        call("wait", {"frames": 10})


def grade(view):
    scene, checkpoint = VIEWS[view]
    out = os.path.join(OUT, view)
    os.makedirs(out, exist_ok=True)
    log = open(os.path.join(out, "run.log"), "w")
    cmd = ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=12G", "-p", "MemorySwapMax=0",
           os.path.abspath(ARGS["bin"]), "--headless", "--menu", "hidden", "--scene", scene, "--checkpoint", checkpoint,
           "--size", "1280x720", "--mcp-port", str(PORT), "--out", out]
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
        settle(2.0)
        ask = {"width": int(ARGS["width"]), "spp": int(ARGS["spp"]), "noise": float(ARGS["noise"]),
               "seconds": int(ARGS["seconds"]), "block": int(ARGS["block"]), "frames": 2}
        old = call("compare", dict(ask, prefix="old"))
        call("lighting.set_settings", dict(SETS, cone_cascades="on"))
        for _ in range(4000):
            r = call("cone_stats")
            if r["cascades"] and r["cascades"][0]["probes"] > 0 and not r.get("field", "").startswith("coarse"):
                break
            call("wait", {"frames": 1})
        # The bounce's short history steps by frame time.
        settle(2.0)
        cone = call("compare", dict(ask, prefix="cone"))
        call("quit")
        return old, cone
    finally:
        time.sleep(2)
        if proc.poll() is None:
            proc.terminate()


results = {}
ok = True
for view in ARGS["views"].split(","):
    old, cone = grade(view)
    results[view] = {"old": old, "cone": cone}
    passed = cone["mean_rel"] <= float(ARGS["limit"])
    ok = ok and passed
    print(f"{'PASS' if passed else 'FAIL'} {view}: cone mean_rel {cone['mean_rel']:.1%} bias {cone['bias']:+.1%}; "
          f"old path {old['mean_rel']:.1%} bias {old['bias']:+.1%} (limit {float(ARGS['limit']):.0%})", flush=True)
    with open(os.path.join(OUT, "grade.json"), "w") as f:
        json.dump(results, f, indent=1)
sys.exit(0 if ok else 1)
