#!/usr/bin/env python3
"""Read lightingage lines and the red-box picture delta.

    shown_age.py ages LOG
    shown_age.py pair OFF_DIR ON_DIR

ages prints one row per MARK. pair scores mid/stop/after red-box luma.
"""

import os
import re
import sys

MARK = re.compile(r"MARK (\w+) frame=(\d+)")
AGE = re.compile(
    r"lightingage \S+ frame (\d+) red (\d+) never (\d+) age0 (\d+) "
    r"age1_3 (\d+) age4_7 (\d+) age8_15 (\d+) age16 (\d+) "
    r"predates (\d+) postdates (\d+) sum_age (\d+) nostamp (\d+)"
)


def read_text(path):
    text = open(path, encoding="utf-8", errors="replace").read()
    report = os.path.join(os.path.dirname(path), "report.md")
    if os.path.exists(report) and os.path.abspath(report) != os.path.abspath(path):
        text += "\n" + open(report, encoding="utf-8", errors="replace").read()
    return text


def ages(path):
    rows = []
    for line in read_text(path).splitlines():
        m = AGE.search(line)
        if not m:
            continue
        n = [int(x) for x in m.groups()]
        committed = n[1] - n[2]
        rows.append(
            {
                "frame": n[0],
                "red": n[1],
                "never": n[2],
                "age0": n[3],
                "age1_3": n[4],
                "age4_7": n[5],
                "age8_15": n[6],
                "age16": n[7],
                "predates": n[8],
                "postdates": n[9],
                "sum_age": n[10],
                "nostamp": n[11],
                "mean_age": (n[10] / committed) if committed else None,
            }
        )
    return rows


def marks(path):
    out = []
    for line in read_text(path).splitlines():
        m = MARK.search(line)
        if m:
            out.append((m.group(1), int(m.group(2))))
    return out


def nearest(rows, frame):
    if not rows:
        return None
    return min(rows, key=lambda r: abs(r["frame"] - frame))


def share(part, whole):
    if not whole:
        return None
    return part / whole


def print_ages(log_path):
    rows = ages(log_path)
    print(f"log {log_path} age_lines {len(rows)}")
    for tag, frame in marks(log_path):
        row = nearest(rows, frame)
        if row is None:
            print(f"{tag} frame {frame} no age line")
            continue
        red = row["red"]
        committed = red - row["never"]
        print(
            f"{tag} mark {frame} age_frame {row['frame']} red {red} "
            f"mean_age {row['mean_age']} "
            f"age0 {share(row['age0'], committed)} "
            f"age1_3 {share(row['age1_3'], committed)} "
            f"age4_7 {share(row['age4_7'], committed)} "
            f"age8_15 {share(row['age8_15'], committed)} "
            f"age16 {share(row['age16'], committed)} "
            f"predates {share(row['predates'], committed)} "
            f"postdates {share(row['postdates'], committed)} "
            f"nostamp {share(row['nostamp'], committed)} never {row['never']}"
        )


def main(argv):
    if len(argv) == 3 and argv[1] == "ages":
        print_ages(argv[2])
        return 0
    if len(argv) == 4 and argv[1] == "pair":
        # Imported lazily so `ages` runs without numpy.
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        from fill_curve import codes_darker

        def find(directory, tag):
            hits = []
            for name in os.listdir(directory):
                if name.startswith(tag + "-") and name.endswith("-full.png"):
                    hits.append(name)
            if not hits:
                return None
            hits.sort()
            return os.path.join(directory, hits[-1])

        off, on = argv[2], argv[3]
        for tag in ("mid", "stop", "after"):
            a = find(off, tag)
            b = find(on, tag)
            depth = None
            albedo = None
            for name in os.listdir(off):
                if name.startswith(tag + "-") and name.endswith("-depth.png"):
                    depth = os.path.join(off, name)
                if name.startswith("albedo-") and name.endswith("-albedo.png"):
                    albedo = os.path.join(off, name)
            if not a or not b:
                print(f"{tag} missing picture")
                continue
            row = codes_darker(a, b, depth, albedo)
            print(tag, row)
        return 0
    print("usage: shown_age.py ages LOG | pair OFF_DIR ON_DIR", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
