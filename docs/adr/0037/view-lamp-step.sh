#!/bin/sh
# ADR 0037 viewing test. Usage: V=e0|e1|e3f|e3fF|e3fR|e3fSR sh view-lamp-step.sh
# Builds lighting-v3 34cc80c + hooks-r16.patch in ~/.cache/ge-adr37-view-src16 (once),
# then runs the hall lamp on/off/x0.5 loop. Never uses port 8765.
set -e
D=$(cd "$(dirname "$0")" && pwd); R=$(git -C "$D" rev-parse --show-toplevel)
W=$HOME/.cache/ge-adr37-view-src16
if [ ! -d "$W" ]; then
  git -C "$R" fetch -q origin lighting-v3 || true
  git -C "$R" worktree add -q --detach "$W" 34cc80c
  git -C "$W" apply "$D/hooks-r16.patch"
fi
B="GENOS_DBG_MEAN=3 GENOS_DBG_K=3 GENOS_DBG_NO_YOUNG=1 GENOS_DBG_LAMPOFF=1 GENOS_DBG_TEXOFF=1 GENOS_DBG_SIGCAP=1 GENOS_DBG_CONV=5 GENOS_DBG_FIELD=1 GENOS_DBG_PROBE=1 GENOS_DBG_FIELD2=1 GENOS_DBG_RUN=1 GENOS_DBG_EVT_BUDGET=8192"
case "${V:-e3fF}" in
  e0) E="GENOS_DBG_NONE=1" ;; e1) E="$B GENOS_DBG_EVT=1" ;; e3f) E="$B GENOS_DBG_EVT=3" ;; e3fF) E="$B GENOS_DBG_EVT=3 GENOS_DBG_EVTF=1" ;; e3fR) E="$B GENOS_DBG_EVT=3 GENOS_DBG_RING32=1" ;; e3fSR) E="$B GENOS_DBG_EVT=3 GENOS_DBG_RING32=1 GENOS_DBG_EVT_SPREAD=12" ;;
  *) echo "V must be e0, e1, e3f, e3fF, e3fR or e3fSR"; exit 2 ;;
esac
P=${GENOS_MCP_PORT:-8815}; [ "$P" = 8765 ] && { echo "port 8765 is reserved"; exit 3; }
cd "$W"
echo "variant ${V:-e3fF}, port $P"
export CARGO_TARGET_DIR=$HOME/.cache/ge-adr37-view-target
nice -n 19 cargo build --release --bin genos-stress
exec flock ~/genos-bench.lock timeout 600 nice -n 19 env $E GENOS_MCP_PORT=$P LAVAPIPE=0 "$CARGO_TARGET_DIR/release/genos-stress" --script "$D/view-lamp-step.rhai"
