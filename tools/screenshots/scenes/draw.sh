#!/usr/bin/env bash
# Scene: wlr-draw — annotate live on screen (a code-review style moment).
# Produces docs/assets/wlr-draw/{annotate.png, annotate.gif, annotate.webp}.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

shots_start 2560x1440

# A real desktop to annotate over (browsers, a video, a calculator).
shots_visible_desktop

DRAW="$(shots_tool wlr-draw)"
shots_spawn "$DRAW"
shots_settle 1.4
"$DRAW" on >/dev/null 2>&1
shots_settle 0.5

# The annotation choreography, replayed for the GIF and left on screen for the still.
# Layout: github (left) · video (top-mid) / calculator (bottom-mid) · phoronix (right).
draw_demo() {
  # 1) red arrow pointing at the video (top-middle)
  "$DRAW" tool arrow >/dev/null 2>&1; "$DRAW" color red >/dev/null 2>&1; "$DRAW" width 10 >/dev/null 2>&1
  shots_drag 1780 640 1410 420 14
  shots_settle 0.5
  # 2) yellow highlight box around the GitHub window (left column)
  "$DRAW" tool rect >/dev/null 2>&1; "$DRAW" color yellow >/dev/null 2>&1; "$DRAW" width 8 >/dev/null 2>&1
  shots_drag 40 250 840 1230 14
  shots_settle 0.5
  # 3) freehand red loop circling the calculator (bottom-middle, dwell-snaps to ellipse)
  "$DRAW" tool pen >/dev/null 2>&1; "$DRAW" color red >/dev/null 2>&1; "$DRAW" width 7 >/dev/null 2>&1
  shots_cursor 1640 1080; sleep 0.12; shots_press
  for a in 0 30 60 90 120 150 180 210 240 270 300 330 360; do
    x=$(awk "BEGIN{printf \"%d\", 1270 + 380*cos($a*3.14159/180)}")
    y=$(awk "BEGIN{printf \"%d\", 1080 + 290*sin($a*3.14159/180)}")
    shots_cursor "$x" "$y"; sleep 0.03
  done
  sleep 0.1; shots_release
  shots_settle 0.5
  # 4) a cyan text label (over the phoronix column)
  "$DRAW" tool text >/dev/null 2>&1; "$DRAW" color cyan >/dev/null 2>&1; "$DRAW" width 8 >/dev/null 2>&1
  shots_cursor 1900 1320; sleep 0.15; shots_click; sleep 0.25
  shots_type "ship it"
  shots_settle 0.4
  shots_key Return            # Return commits the label (Escape would discard it)
  "$DRAW" tool pen >/dev/null 2>&1   # switching tools also commits -> no stray caret
  shots_settle 0.6
}

# Record the choreography as a looping GIF + WebP …
shots_record "$(shots_out wlr-draw annotate)" 12 draw_demo
# … then grab the final composed frame as the hero still.
shots_settle 0.3
shots_grab "$(shots_out wlr-draw annotate.png)"

"$DRAW" quit >/dev/null 2>&1
shots_stop
echo "[draw] done -> $SHOTS_ASSETS/wlr-draw/"
