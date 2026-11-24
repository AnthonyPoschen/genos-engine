#!/usr/bin/env python3
"""Exposure gate: standing in shadow must not blow out the lit view.

Starts genos-lighting headless on Sponza under a memory cap, stands at the
shadow pose (under the arcade, looking out into the sunlit courtyard) and at
the lit pose (in the courtyard, the same direction), lets auto exposure
settle at each, and measures:

  clipped  share of picture pixels with a channel at 250 or more, at the
           shadow pose
  gap      |ev100(lit) - ev100(shadow)|, how far exposure swings between
           the two poses
  opens    ev100(lit) - ev100(dark): a dark place (the dark-corner
           checkpoint) must still open exposure up

PASS when clipped <= --max-clipped, gap <= --max-gap and opens >= --min-opens.
Before the highlight key (25 Nov 2026): clipped 0.038, gap 3.63 stops. The pictures, the
HDR replies and a summary go to --out.

  --bin PATH          genos-lighting binary (default target/release/genos-lighting)
  --port N            command port (default 18796)
  --out DIR           evidence directory (default genos-debug/exposure-gate)
  --max-clipped F     limit for clipped (default 0.01)
  --max-gap F         limit for gap in stops (default 2.0)
  --min-opens F       least opens in stops (default 2.0)
  --shadow X,Y,Z,YAW,PITCH  shadow pose (default -13.39,2.13,-4.41,-4.621,-0.102:
                            under the arcade, sunlit courtyard to the right)
  --lit X,Y,Z,YAW,PITCH     lit pose (default -13,2.1,0,-4.621,-0.102: in the
                            courtyard, the same way)
  --dark X,Y,Z,YAW,PITCH    dark pose (default 8,1.5,3.5,2.356194,-0.1: the
                            dark-corner checkpoint)
  --set KEY=VALUE     an HDR key to set before measuring (repeatable)
  --measure-only      print the numbers, exit 0
  -h, --help          this text

Needs the Sponza asset (git lfs pull --include="assets/built/sponza/**").
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

ARGS = {"bin": "target/release/genos-lighting", "port": "18796", "out": "genos-debug/exposure-gate",
        "max-clipped": "0.01", "max-gap": "2.0",
        "min-opens": "2.0", "shadow": "-13.39,2.13,-4.41,-4.621,-0.102", "lit": "-13,2.1,0,-4.621,-0.102",
        "dark": "8,1.5,3.5,2.356194,-0.1"}
SETS = []
MEASURE_ONLY = False
argv = sys.argv[1:]
while argv:
    a = argv.pop(0)
    if a in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if a == "--measure-only":
        MEASURE_ONLY = True
    elif a == "--set" and argv:
        SETS.append(argv.pop(0))
    elif a.startswith("--") and a[2:] in ARGS and argv:
        ARGS[a[2:]] = argv.pop(0)
    else:
        print(f"unknown argument {a}\n{__doc__}", file=sys.stderr)
        sys.exit(2)
PORT = int(ARGS["port"])
OUT = os.path.abspath(ARGS["out"])
os.makedirs(OUT, exist_ok=True)


def call(name, arg=None, timeout=300):
    req = urllib.request.Request(f"http://localhost:{PORT}/cmd/{name}", data=json.dumps(arg or {}).encode(), method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as reply:
            return json.loads(reply.read())
    except urllib.error.HTTPError as e:
        raise SystemExit(f"UNSURE: {name} {arg}: {e.read().decode()}")


def clipped_share(path):
    from PIL import Image
    img = Image.open(path).convert("RGB")
    px = img.get_flattened_data() if hasattr(img, "get_flattened_data") else img.getdata()
    hot = sum(1 for p in px if max(p) >= 250)
    return hot / max(len(px), 1)


def settle():
    # Auto exposure adapts over seconds of frame time; wait in real time.
    call("wait_settled", {"timeout_frames": 600})
    time.sleep(6)
    call("wait", {"frames": 30})


def measure(checkpoint):
    x, y, z, yaw, pitch = (float(v) for v in ARGS[checkpoint].split(","))
    call("camera", {"position": [x, y, z], "yaw": yaw, "pitch": pitch})
    settle()
    shot = call("capture", {"path": f"{checkpoint}.png"})
    hdr = call("get_hdr")
    hdr.pop("histogram", None)
    with open(os.path.join(OUT, f"{checkpoint}-hdr.json"), "w") as f:
        json.dump(hdr, f, indent=1)
    path = shot["path"] if os.path.isabs(shot["path"]) else os.path.join(OUT, shot["path"])
    return hdr["ev100"], clipped_share(path)


log = open(os.path.join(OUT, "run.log"), "w")
cmd = ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=12G", "-p", "MemorySwapMax=0",
       os.path.abspath(ARGS["bin"]), "--headless", "--menu", "hidden", "--scene", "sponza",
       "--preset", "high", "--size", "1280x720", "--mcp-port", str(PORT), "--out", OUT]
proc = subprocess.Popen(cmd, stdout=log, stderr=log)
try:
    for _ in range(180):
        time.sleep(1)
        try:
            if call("luminance", timeout=3).get("luminance", 0) > 0.01:
                break
        except Exception:
            if proc.poll() is not None:
                sys.exit(f"UNSURE: genos-lighting exited {proc.returncode}; see {OUT}/run.log")
    else:
        sys.exit("UNSURE: Sponza did not load in 180 s")
    for kv in SETS:
        key, value = kv.split("=", 1)
        call("set_hdr_settings", {key: value})
    ev_shadow, clipped = measure("shadow")
    ev_lit, _ = measure("lit")
    ev_dark, _ = measure("dark")
    gap = abs(ev_lit - ev_shadow)
    opens = ev_lit - ev_dark
    line = (f"clipped {clipped:.4f} (limit {ARGS['max-clipped']}), gap {gap:.2f} stops (limit {ARGS['max-gap']}), "
            f"opens {opens:.2f} stops (least {ARGS['min-opens']}), ev100 shadow {ev_shadow:.2f} lit {ev_lit:.2f} dark {ev_dark:.2f}")
    with open(os.path.join(OUT, "summary.txt"), "w") as f:
        f.write(line + "\n")
    call("quit")
finally:
    time.sleep(2)
    if proc.poll() is None:
        proc.terminate()
if MEASURE_ONLY:
    print(line)
    sys.exit(0)
ok = clipped <= float(ARGS["max-clipped"]) and gap <= float(ARGS["max-gap"]) and opens >= float(ARGS["min-opens"])
print(("PASS " if ok else "FAIL ") + line)
sys.exit(0 if ok else 1)
