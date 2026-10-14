# Scripted verdicts

Agents were re-judging the same GPU pictures and frame times in prose (about 33
hand-judged verify runs on 14 Oct). A verdict script makes that judgement once.
The agent reads one line. The pictures, diffs, and numbers stay on disk.

These checks are on **probation**. Review a verdict against its evidence
directory before trusting it. A miss, a false pass, or a false fail becomes a
test in `scripts/verdict/tests/test_misses.py` before the threshold changes.
A check graduates only after a run of correct verdicts.

## Contract

Every script prints one line and nothing else on stdout:

```text
PASS|FAIL|UNSURE <short reason> evidence=<dir>
```

| Exit | Meaning |
|---|---|
| 0 | PASS |
| 1 | FAIL |
| 2 | UNSURE. Read the evidence. Ask a person only if that still does not decide. |
| 3 | The check broke (`UNSURE check-broke …`). Not a product fail. |

Evidence (screenshots, diff images, CSV, logs) goes to
`./genos-debug/verdict/<kind>/<sha>/`, or to `--out`. The line is also
`verdict.txt` in that directory. Stdout does not dump the evidence.

Intent comes from the command line. `GENOS_VERDICT_OUT`, `GENOS_VERDICT_DISPLAY`,
`GENOS_VERDICT_SHA`, `GENOS_VERDICT_CACHE_DIR`, `GENOS_VERDICT_BASE`, and
`GENOS_VERDICT_HEAD` are mirrors: each is read only when its flag was omitted.
A flag wins when both are set.

`--dry-run` checks the arguments, writes `args.txt`, and exits 0 with
`PASS dry-run`. It does not launch a window.

A window is never placed on the session compositor. A script that draws
requires `--display` (a Wayland display name). It passes that name to the
binary, sets the child `WAYLAND_DISPLAY` to it, and drops the inherited
session display. The binary opens with `--proof` or `--headless` (the proof
window floats and does not take focus). Quit is MCP `quit`, then SIGTERM and
a wait. The scripts do not send SIGKILL.

`./genos-debug/` is gitignored. Nothing is written under `$HOME`.

## Precheck

`scripts/verdict/precheck.py` runs before anything uses the GPU.

```sh
python3 scripts/verdict/precheck.py --base gi-v2 --key noise-floor --display wayland-1
```

1. `git merge-tree --write-tree` against `--base`. A conflict is `FAIL`.
2. GPU and lock probe. Another `genos-stress` process, a live pid in
   `./genos-debug/verdict/gpu.lock`, or a GPU busier than `--gpu-busy-percent`
   (default 50) is `UNSURE busy`. An unreadable sensor is recorded and is not
   itself busy.
3. Verdict cache, keyed on the commit sha plus the hash of `--key` and
   `--input` files. A hit reprints that verdict and skips the rerun. The
   judging scripts write the cache. Precheck only reads it, and only after
   the merge and the busy probe are clear.

`--process-file` and `--gpu-util-file` are fixtures for the probe. The live
probe reads `/proc` and, when present, `gpu_busy_percent` or `nvidia-smi`.
