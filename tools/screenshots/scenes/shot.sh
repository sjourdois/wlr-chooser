#!/usr/bin/env bash
# Scene: wlr-shot — the frozen interactive region selector (`screenshot -s`).
# Produces docs/assets/wlr-shot/select.{png,gif,webp,apng}: a dimmed frozen
# screen with a bright selection rectangle being dragged out.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

shots_start 2560x1440
shots_visible_desktop

SHOT="$(shots_tool wlr-shot)"

# The selector writes to FILE when you confirm; we cancel (Esc) so nothing is
# written. We drive a drag and stop mid-selection to show the rectangle — here a
# region spanning GitHub and the video in the middle.
ax=320 ay=320 bx=1680 by=1080
select_demo() {
  setsid "$SHOT" screenshot -s /tmp/wlr-shot-demo.png >/dev/null 2>&1 < /dev/null &
  SEL_PID=$!
  shots_settle 1.8
  shots_cursor "$ax" "$ay"; sleep 0.15; shots_press; sleep 0.1
  local i
  for i in $(seq 1 16); do
    shots_cursor "$(( ax + (bx-ax)*i/16 ))" "$(( ay + (by-ay)*i/16 ))"
    sleep 0.05
  done
  shots_settle 0.8        # hold the selection on screen for the animation tail
}
shots_record "$(shots_out wlr-shot select)" 12 select_demo
shots_settle 0.2
shots_grab "$(shots_out wlr-shot select.png)"
shots_release
shots_key Escape
kill "${SEL_PID:-0}" 2>/dev/null
shots_settle 0.8

shots_stop
echo "[shot] done -> $SHOTS_ASSETS/wlr-shot/"
