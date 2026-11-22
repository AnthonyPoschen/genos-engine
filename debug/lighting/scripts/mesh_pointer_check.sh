#!/usr/bin/env bash
# Check the mesh viewer's pointer with a real pointer: a click selects a face,
# a drag orbits, and a click on the projection's name changes the projection.
#
# This moves the desktop's own pointer and clicks with it, as the pointer-lock
# test in crates/window does: it focuses the editor's window, moves the cursor
# with hyprctl, clicks with ydotool, then puts the cursor and the focus back.
# It needs Hyprland, ydotool and a built genos-editor.
#
# RUN IT ON PURPOSE, AT THE KEYBOARD. It takes the pointer and the focus for
# about ten seconds. An agent does not run this: it hands it to the person.
set -u
PORT=18765
BIN=target/release/genos-editor
OUT=genos-debug
usage() {
  cat <<'HELP'
mesh_pointer_check.sh [--port PORT] [--bin PATH] [--out DIR]

  --port PORT   MCP port the editor is started on (default 18765)
  --bin PATH    the genos-editor binary (default target/release/genos-editor)
  --out DIR     where the scratch workspace and the editor's files go
                (default genos-debug)
  -h, --help    this text

Starts genos-editor on a copy of the sample workspace, then with the real
pointer: clicks the cube, drags to orbit, and clicks the projection's name.
Prints PASS or FAIL for each and exits 1 on a failure. The pointer and the
focused window are put back.
HELP
}
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument $1" >&2; usage >&2; exit 2 ;;
  esac
done
for tool in hyprctl ydotool curl python3; do
  command -v "$tool" >/dev/null || { echo "UNSURE: $tool is not installed"; exit 3; }
done
[ -x "$BIN" ] || { echo "UNSURE: $BIN is not built"; exit 3; }
ROOT=$(pwd)
WS="$ROOT/$OUT/mesh-pointer-check"
rm -rf "$WS" && mkdir -p "$WS" && cp debug/scenes/mesh/sample/project.json "$WS/"
call() { curl -s -m 10 -X POST "localhost:$PORT/cmd/$1" -d "$2"; }
field() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }
"$BIN" --mcp-port "$PORT" --out "$ROOT/$OUT" --scene mesh --mesh-workspace "$WS" > "$ROOT/$OUT/mesh-pointer-check.log" 2>&1 &
EDITOR_PID=$!
for _ in $(seq 1 40); do call scene.status '{}' >/dev/null 2>&1 && break; /usr/bin/sleep 0.5; done
call editor.set_menu_settings '{"visible":"hidden"}' >/dev/null
call mesh.view '{"panel":false,"look":"+z"}' >/dev/null
/usr/bin/sleep 1.5
read -r AT_X AT_Y W H ADDRESS < <(hyprctl clients -j | python3 -c "
import json,sys
for c in json.load(sys.stdin):
    if c.get('pid') == $EDITOR_PID: print(c['at'][0], c['at'][1], c['size'][0], c['size'][1], c['address'])")
CURSOR=$(hyprctl cursorpos | tr -d ' ')
BEFORE=$(hyprctl activewindow -j | python3 -c "import json,sys; print(json.load(sys.stdin).get('address',''))")
restore() {
  hyprctl dispatch "hl.dsp.cursor.move({ x = ${CURSOR%,*}, y = ${CURSOR#*,} })" >/dev/null
  [ -n "$BEFORE" ] && hyprctl dispatch "hl.dsp.focus({ window = \"address:$BEFORE\" })" >/dev/null
  call scene.quit '{}' >/dev/null 2>&1
}
trap restore EXIT
# A warp alone sends the window no motion: nudge the pointer so the window
# knows where it is before the press, as it does under a hand.
move() {
  hyprctl dispatch "hl.dsp.cursor.move({ x = $1, y = $2 })" >/dev/null; /usr/bin/sleep 0.2
  ydotool mousemove -x 1 -y 0 >/dev/null; /usr/bin/sleep 0.1
  ydotool mousemove -x -1 -y 0 >/dev/null; /usr/bin/sleep 0.3
}
# A click as a hand makes it: down, a tenth of a second, up.
press() { ydotool click 0x40 >/dev/null; /usr/bin/sleep "${HOLD:-0.1}"; ydotool click 0x80 >/dev/null; }
FAILED=0
report() { if [ "$1" = ok ]; then echo "PASS $2"; else echo "FAIL $2"; FAILED=1; fi; }
hyprctl dispatch "hl.dsp.focus({ window = \"address:$ADDRESS\" })" >/dev/null
# 1. A click on the cube's front face: from +Z it is left of the middle (the
# middle of the picture is the gap between the cube and the bracket).
move $((AT_X + W * 45 / 100)) $((AT_Y + H / 2))
press; /usr/bin/sleep 0.8
FACES=$(call mesh.selection '{}' | field "d['faces']")
[ "$FACES" = "[2]" ] && report ok "a real click selected face 2 (selection $FACES)" || report no "a real click selected $FACES, not [2]"
# 2. A drag: press, move, release. The camera turns and the selection stays.
YAW=$(call scene.camera '{}' | field "round(d['yaw'],3)")
ydotool click 0x40 >/dev/null; /usr/bin/sleep 0.2
for _ in 1 2 3 4 5 6; do ydotool mousemove -x 30 -y 0 >/dev/null; /usr/bin/sleep 0.08; done
ydotool click 0x80 >/dev/null; /usr/bin/sleep 0.6
YAW_NOW=$(call scene.camera '{}' | field "round(d['yaw'],3)")
FACES=$(call mesh.selection '{}' | field "d['faces']")
[ "$YAW" != "$YAW_NOW" ] && [ "$FACES" = "[2]" ] && report ok "a real drag turned the camera (yaw $YAW to $YAW_NOW) and kept the selection" || report no "a real drag: yaw $YAW to $YAW_NOW, selection $FACES"
# 3. A click on the projection's name, under the gizmo in the top right corner.
move $((AT_X + W - 70)) $((AT_Y + 130))
press; /usr/bin/sleep 0.8
PROJECTION=$(call mesh.view '{}' | field "d['projection']")
[ "$PROJECTION" = orthographic ] && report ok "a real click on the projection's name gave $PROJECTION" || report no "the projection is $PROJECTION after a click on its name"
exit $FAILED
