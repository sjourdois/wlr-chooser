#!/usr/bin/env bash
# Scene: wlr-chooser — the graphical window & screen picker for the screencast
# portal. A still only (the picker is essentially static, and an animated clip
# would duplicate wlr-switcher's card). Produces docs/assets/wlr-chooser/picker.png.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

shots_start 2560x1440
shots_rich_desktop

CH="$(shots_tool wlr-chooser)"
setsid "$CH" --both >/dev/null 2>&1 < /dev/null &
CH_PID=$!
shots_settle 2.8
shots_cursor 1280 560; shots_settle 0.6   # hover a card so one tile reads as highlighted
shots_grab "$(shots_out wlr-chooser picker.png)"
kill "$CH_PID" 2>/dev/null
shots_settle 0.6

shots_stop
echo "[chooser] done -> $SHOTS_ASSETS/wlr-chooser/"
