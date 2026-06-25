#!/usr/bin/env bash
# Scene: wlr-peek CLI subcommands — a terminal cast of the non-overlay tools
# (grep, ocr, watch) running against a real page on screen. Produces
# docs/assets/wlr-peek/cli.{mp4,gif,apng,png}.
set -u
cd "$(dirname "$0")/.."
. ./lib.sh

# Smaller output: full-screen OCR (grep) is O(pixels), so 1600x900 keeps the cast
# snappy while staying readable.
shots_start 1600x900
PEEK="$(shots_tool wlr-peek)"

# Left: a real page to read/search (GitHub). Right: the terminal running the cast.
shots_chromium "https://github.com/sjourdois/wlr-utils"
shots_settle 9

# Build the cast as a real script (avoids fragile inline quoting), then run it.
CAST="$(mktemp --suffix=-peekcast.sh)"
cat > "$CAST" <<CASTEOF
#!/usr/bin/env bash
P='$PEEK'
run(){ printf '\n\033[38;5;114m❯\033[0m \033[1mwlr-peek %s\033[0m\n' "\$*"; sleep 0.6; "\$P" "\$@" 2>&1; }
sleep 1.5
echo '# OCR a region of the page (Tesseract)'
run ocr -g '60,150 700x140' | sed '/^[[:space:]]*\$/d' | head -4
sleep 2.2; echo
echo '# block until a region stops changing, then act (CI-style)'
run watch -o HEADLESS-1 --on idle --for 2 --timeout 8 && echo '  -> screen is idle'
sleep 2.2
CASTEOF

shots_term "wlr-peek" "bash '$CAST'; exec sleep 100"
shots_settle 1.0
shots_park

cli_demo() { shots_settle 12; }   # let the cast play out
shots_record "$(shots_out wlr-peek cli)" 10 cli_demo
shots_grab "$(shots_out wlr-peek cli.png)"

rm -f "$CAST"
shots_stop
echo "[peek-cli] done -> $SHOTS_ASSETS/wlr-peek/cli.*"
