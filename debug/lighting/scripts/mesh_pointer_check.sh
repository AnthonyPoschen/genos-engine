#!/usr/bin/env bash
# Check the mesh viewer's pointer with a real pointer: a click selects a face,
# a drag orbits, the wheel zooms, a middle drag pans, a right drag draws a box
# (so does Control and a drag), Alt and a drag draws a lasso, and a click on the projection's name changes
# the projection.
#
# This moves the desktop's own pointer and clicks with it, as the pointer-lock
# test in crates/window does: it focuses the editor's window, moves the cursor
# with hyprctl, clicks with ydotool, then puts the cursor and the focus back.
# It needs Hyprland, ydotool and a built genos-editor.
#
# RUN IT ON PURPOSE, AT THE KEYBOARD. It takes the pointer and the focus for
# about twenty seconds. An agent does not run this: it hands it to the person.
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
pointer: clicks the cube, drags to orbit, turns the wheel, middle-drags to pan,
draws a box (right drag, and Control) and a lasso (Alt), clicks the projection's name, and clicks the top bar.
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
  ydotool key 29:0 56:0 >/dev/null 2>&1
  ydotool click 0x80 0x81 0x82 >/dev/null 2>&1
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
# 3. The wheel zooms: the camera moves and does not turn.
POSITION=$(call scene.camera '{}' | field "[round(v,3) for v in d['position']]")
for _ in 1 2 3; do ydotool mousemove -w -x 0 -y 1 >/dev/null; /usr/bin/sleep 0.1; done; /usr/bin/sleep 0.6
POSITION_NOW=$(call scene.camera '{}' | field "[round(v,3) for v in d['position']]")
YAW_AFTER=$(call scene.camera '{}' | field "round(d['yaw'],3)")
[ "$POSITION" != "$POSITION_NOW" ] && [ "$YAW_AFTER" = "$YAW_NOW" ] && report ok "the real wheel moved the camera ($POSITION to $POSITION_NOW)" || report no "the real wheel: position $POSITION to $POSITION_NOW, yaw $YAW_NOW to $YAW_AFTER"
# 4. A middle drag pans: the camera moves and does not turn.
ydotool click 0x42 >/dev/null; /usr/bin/sleep 0.2
for _ in 1 2 3 4 5 6; do ydotool mousemove -x 20 -y 10 >/dev/null; /usr/bin/sleep 0.08; done
ydotool click 0x82 >/dev/null; /usr/bin/sleep 0.6
POSITION_PAN=$(call scene.camera '{}' | field "[round(v,3) for v in d['position']]")
YAW_AFTER=$(call scene.camera '{}' | field "round(d['yaw'],3)")
[ "$POSITION_PAN" != "$POSITION_NOW" ] && [ "$YAW_AFTER" = "$YAW_NOW" ] && report ok "a real middle drag panned the camera ($POSITION_NOW to $POSITION_PAN)" || report no "a real middle drag: position $POSITION_NOW to $POSITION_PAN, yaw $YAW_NOW to $YAW_AFTER"
# 5 to 7. A right drag draws a box, Control and a left drag a box, Alt and a
# left drag a lasso. Each starts from the front view with nothing selected,
# goes across the cube from its top left to its bottom right, and must select
# the front face (2) and not the back one (3), without moving the camera.
# Key 29 is Control, 56 is Alt; button 0 is the left one, 1 the right one.
region() { # region BUTTON KEY NAME
  call mesh.view '{"look":"+z"}' >/dev/null
  call mesh.selection '{"clear":true}' >/dev/null
  /usr/bin/sleep 0.8
  local pose faces
  pose=$(call scene.camera '{}' | field "[round(v,3) for v in d['position']+[d['yaw']]]")
  move $((AT_X + W * 25 / 100)) $((AT_Y + H * 20 / 100))
  [ -n "$2" ] && { ydotool key "$2:1" >/dev/null; /usr/bin/sleep 0.2; }
  ydotool click "0x4$1" >/dev/null; /usr/bin/sleep 0.2
  # Down the diagonal, then back along the top: a box needs its far corner
  # only, a lasso needs a path with room inside.
  local step
  for step in "$((W * 24 / 100)) $((H * 60 / 100))" "0 -$((H * 60 / 100))"; do
    [ "$3" != "Alt (lasso)" ] && [ "${step% *}" = 0 ] && break
    for _ in 1 2 3 4 5 6; do ydotool mousemove -x $((${step% *} / 6)) -y $((${step#* } / 6)) >/dev/null; /usr/bin/sleep 0.08; done
  done
  ydotool click "0x8$1" >/dev/null; /usr/bin/sleep 0.2
  [ -n "$2" ] && ydotool key "$2:0" >/dev/null
  /usr/bin/sleep 0.6
  faces=$(call mesh.selection '{}' | field "d['faces']")
  case "$faces" in *"[2"*|*" 2,"*|*" 2]"*) seen=yes ;; *) seen=no ;; esac
  case "$faces" in *"3"*) back=yes ;; *) back=no ;; esac
  [ "$seen" = yes ] && [ "$back" = no ] && [ "$(call scene.camera '{}' | field "[round(v,3) for v in d['position']+[d['yaw']]]")" = "$pose" ] \
    && report ok "a real $3 drag selected faces $faces" \
    || report no "a real $3 drag selected faces $faces (camera was $pose)"
}
region 1 "" "right (box)"
region 0 29 "Control (box)"
region 0 56 "Alt (lasso)"
# 8. A click on the projection's name, under the gizmo in the top right corner.
# The gizmo stands under the top bar, which is 30 pixels tall.
move $((AT_X + W - 70)) $((AT_Y + 160))
press; /usr/bin/sleep 0.8
PROJECTION=$(call mesh.view '{}' | field "d['projection']")
[ "$PROJECTION" = orthographic ] && report ok "a real click on the projection's name gave $PROJECTION" || report no "the projection is $PROJECTION after a click on its name"
# 9. The top bar: a click on the first button opens its page as the drawer,
# and a second click on it closes the drawer.
move $((AT_X + 20)) $((AT_Y + 15))
press; /usr/bin/sleep 0.8
OPEN=$(call editor.get_menu_settings '{}' | field "str(d['visible'])+' '+d['tab']")
press; /usr/bin/sleep 0.8
SHUT=$(call editor.get_menu_settings '{}' | field "str(d['visible'])")
case "$OPEN" in "True "?*) opened=yes ;; *) opened=no ;; esac
[ "$opened" = yes ] && [ "$SHUT" = False ] && report ok "a real click on the top bar opened the page ${OPEN#True } and a second click closed it" || report no "the top bar: after one click '$OPEN', after two visible=$SHUT"
exit $FAILED
