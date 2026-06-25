#!/usr/bin/env bash
# Scene: wlr-switcher — Alt-Tab with LIVE window previews (the differentiator).
# A realistic desktop (two browser windows + terminals); the strip cycles slowly
# with Tab so the moving highlight reads clearly. Bonus: the rofi-like card.
# Produces docs/assets/wlr-switcher/{altab,card}.{png,gif,webp,apng}.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

shots_start 2560x1440
shots_rich_desktop

SW="$(shots_tool wlr-switcher)"

# --- strip: the macOS-style Alt-Tab row, cycling visibly with Tab -------------
strip_demo() {
  setsid "$SW" --layout strip --live all --no-hold >/dev/null 2>&1 < /dev/null &
  SW_PID=$!
  shots_settle 2.2                       # show the initial selection
  for _ in 1 2 3 4; do
    shots_tab
    shots_settle 0.85                     # dwell on each window so the move is clear
  done
  shots_settle 0.6
}
shots_record "$(shots_out wlr-switcher altab)" 12 strip_demo
shots_settle 0.3
shots_tab; shots_settle 0.4          # land on a mid-strip window for the still
shots_grab "$(shots_out wlr-switcher altab.png)"
kill "$SW_PID" 2>/dev/null; shots_settle 1.0

# --- exposé: the full-screen mission-control grid -----------------------------
# The windows live on three workspaces (only ws1 is visible), so launching the
# exposé clearly REVEALS all of them — incl. the occluded ones it captures live.
expose_demo() {
  shots_settle 1.0                       # a beat on the plain ws1 desktop…
  setsid "$SW" --layout grid --live all --no-hold >/dev/null 2>&1 < /dev/null &
  SW_PID=$!                              # …then the exposé reveals every window
  shots_settle 2.6                       # ease-in + live thumbnails load
  for _ in 1 2 3; do
    shots_tab
    shots_settle 0.85
  done
  shots_settle 0.5
}
shots_record "$(shots_out wlr-switcher expose)" 12 expose_demo
shots_settle 0.3
shots_grab "$(shots_out wlr-switcher expose.png)"
kill "$SW_PID" 2>/dev/null; shots_settle 1.0

# --- card: the centred, rofi-like picker with tabs + search (bonus still) ----
setsid "$SW" --layout card --live all --no-hold >/dev/null 2>&1 < /dev/null &
SW_PID=$!
shots_settle 2.4
shots_grab "$(shots_out wlr-switcher card.png)"
kill "$SW_PID" 2>/dev/null; shots_settle 0.8

shots_stop
echo "[switcher] done -> $SHOTS_ASSETS/wlr-switcher/"
