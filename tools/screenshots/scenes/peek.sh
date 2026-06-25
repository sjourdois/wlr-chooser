#!/usr/bin/env bash
# Scene: wlr-peek — the overlay inspectors, as short videos:
#   color  — the colour picker (pipette) + loupe, sweeping the desktop
#   loupe  — the standalone magnifier, panning and zooming
# Produces docs/assets/wlr-peek/{color,loupe}.{mp4,gif,apng,png}.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

shots_start 2560x1440
shots_visible_desktop

PEEK="$(shots_tool wlr-peek)"

# --- colour picker (pipette): glide slowly over the video, loupe + hex follow -
color_demo() {
  setsid "$PEEK" color >/dev/null 2>&1 < /dev/null &
  P_PID=$!
  shots_settle 2.2
  shots_cursor 1270 360;  shots_settle 1.4        # land on the Factorio video (middle)
  shots_glide 1270 360 1430 280 26; shots_settle 1.4
  shots_glide 1430 280 1130 470 26; shots_settle 1.4
  shots_glide 1130 470 2080 480 36; shots_settle 1.4   # over to phoronix (right)
  shots_settle 0.6
}
shots_record "$(shots_out wlr-peek color)" 12 color_demo
shots_settle 0.2
shots_cursor 1270 360; shots_settle 0.5
shots_grab "$(shots_out wlr-peek color.png)"
shots_key Escape
kill "${P_PID:-0}" 2>/dev/null
shots_settle 1.0

# --- standalone magnifier: pan slowly across windows, scroll to zoom ----------
loupe_demo() {
  setsid "$PEEK" loupe >/dev/null 2>&1 < /dev/null &
  P_PID=$!
  shots_settle 2.2
  shots_cursor 430 420;  shots_settle 1.4         # the GitHub page (left)
  shots_scroll v 3;      shots_settle 1.2         # zoom in
  shots_glide 430 420 1270 360 36; shots_settle 1.4   # pan to the video (middle)
  shots_scroll v 2;      shots_settle 1.2         # zoom more
  shots_glide 1270 360 2080 500 36; shots_settle 1.4  # pan to phoronix (right)
  shots_settle 0.6
}
shots_record "$(shots_out wlr-peek loupe)" 12 loupe_demo
shots_settle 0.2
shots_grab "$(shots_out wlr-peek loupe.png)"
shots_key Escape
kill "${P_PID:-0}" 2>/dev/null
shots_settle 0.8

shots_stop
echo "[peek] done -> $SHOTS_ASSETS/wlr-peek/"
