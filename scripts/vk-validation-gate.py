#!/usr/bin/env python3
"""Run genos-camera and the gpu_frame suite under the Khronos validation layer
and fail on any validation error not listed in lint/vk-device-limits.txt.

Test pass/fail is not graded here (software-GPU results are unproven); only
Vulkan API misuse is. Needs a Wayland display and a Vulkan driver in the env.
Usage: scripts/vk-validation-gate.py [--self-test] [LOG...]
"""
import os
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOW = ROOT / "lint" / "vk-device-limits.txt"
ERROR = re.compile(r"Validation Error: \[ ([^\]\s]+) \]")


def allowed() -> set[str]:
    out = set()
    for line in ALLOW.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.add(line)
    return out


def errors(text: str) -> Counter:
    return Counter(ERROR.findall(text))


def grade(found: Counter, allow: set[str]) -> list[str]:
    return sorted(vuid for vuid in found if vuid not in allow)


def run(cmd: list[str]) -> str:
    env = dict(os.environ, VK_INSTANCE_LAYERS="VK_LAYER_KHRONOS_validation")
    print("+", " ".join(cmd), flush=True)
    proc = subprocess.run(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    print(f"  exit {proc.returncode}", flush=True)
    return proc.stdout


def self_test() -> None:
    sample = (
        "Validation Error: [ VUID-vkCmdDraw-None-09600 ] | MessageID = 0x1\n"
        "Validation Error: [ VUID-vkCmdDispatch-groupCountX-00386 ] | MessageID = 0x2\n"
        "Validation Error: [ UNASSIGNED-Threading-MultipleThreads ] | MessageID = 0x3\n"
    )
    found = errors(sample)
    assert grade(found, allowed()) == ["UNASSIGNED-Threading-MultipleThreads", "VUID-vkCmdDraw-None-09600"], found
    assert grade(errors("no errors\n"), allowed()) == []
    print("vk-validation-gate self-test ok")


def main() -> int:
    args = sys.argv[1:]
    if args == ["--self-test"]:
        self_test()
        return 0
    if args:
        text = "".join(Path(p).read_text(errors="replace") for p in args)
    else:
        text = run(["cargo", "run", "--locked", "-q", "-p", "genos-camera", "--", "--frames", "60"])
        if "Validation Error" not in text and "genos-camera frames=" not in text:
            print(text[-4000:])
            print("genos-camera did not draw its frames", file=sys.stderr)
            return 1
        text += run(["cargo", "test", "--locked", "-p", "genos-render", "--test", "gpu_frame", "--", "--test-threads=1"])
    found = errors(text)
    allow = allowed()
    for vuid, n in sorted(found.items()):
        print(f"{n:6} {vuid}{'  (device limit, allowed)' if vuid in allow else ''}")
    bad = grade(found, allow)
    if bad:
        print(f"FAIL: {len(bad)} Vulkan validation error kind(s): {', '.join(bad)}", file=sys.stderr)
        return 1
    print("vk validation: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
