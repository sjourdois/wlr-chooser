#!/usr/bin/env bash
# Scene: wlr-peek mirror — a live, always-on-top picture-in-picture window that
# mirrors (and zooms) a region of the screen. Produces
# docs/assets/wlr-peek/mirror.png.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

shots_start 2560x1440
shots_visible_desktop

PEEK="$(shots_tool wlr-peek)"
# Mirror the (colourful) video region (middle column) at 2x; the floating mirror
# sits top-right (per the nested config's float rule), clear of the mirrored area.
setsid "$PEEK" mirror -g "920,80 440x330" --zoom 2 >/dev/null 2>&1 < /dev/null &
M_PID=$!
shots_settle 3.0
shots_grab "$(shots_out wlr-peek mirror.png)"
kill "$M_PID" 2>/dev/null
shots_settle 0.6

shots_stop
echo "[mirror] done -> $SHOTS_ASSETS/wlr-peek/mirror.png"
