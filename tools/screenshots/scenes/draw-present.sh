#!/usr/bin/env bash
# Scene: wlr-draw presenter spotlight — hold Shift to darken the screen except a
# flashlight that follows the cursor, then pose a fixed spotlight on a window.
# Produces docs/assets/wlr-draw/spotlight.{mp4,gif,png}.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

shots_start 2560x1440
shots_visible_desktop

DRAW="$(shots_tool wlr-draw)"
shots_spawn "$DRAW"
shots_settle 1.5
"$DRAW" on >/dev/null 2>&1
shots_settle 0.6

present_demo() {
  shots_cursor 430 520; shots_settle 0.6
  # Hold Shift -> flashlight; glide it smoothly across the windows.
  shots_kdown 42; shots_settle 0.9
  shots_glide 430 520 1270 360 30          # github -> the video (top-middle)
  shots_settle 0.6
  shots_scroll v 3; shots_settle 0.6       # widen the flashlight
  shots_glide 1270 360 1270 1080 26        # -> the calculator (down the column)
  shots_settle 0.7
  shots_glide 1270 1080 2080 520 32        # -> phoronix (right)
  shots_settle 0.6
  shots_glide 2080 520 430 520 36          # back to GitHub
  shots_settle 0.4
  # Still holding Shift, drag a fixed spotlight rectangle over GitHub.
  "$DRAW" tool rect >/dev/null 2>&1
  shots_drag 40 210 840 1240 18
  shots_settle 0.6
  shots_kup 42                             # release Shift; the posed spotlight stays
  shots_settle 1.2
}
shots_record "$(shots_out wlr-draw spotlight)" 12 present_demo
shots_settle 0.3
shots_grab "$(shots_out wlr-draw spotlight.png)"

"$DRAW" quit >/dev/null 2>&1
shots_stop
echo "[draw-present] done -> $SHOTS_ASSETS/wlr-draw/spotlight.*"
