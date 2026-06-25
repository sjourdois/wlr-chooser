#!/usr/bin/env bash
# tools/screenshots/capture.sh — regenerate the showcase assets.
#
#   ./capture.sh             # build the tools and (re)generate every scene
#   ./capture.sh draw peek   # only the named scenes
#   SKIP_BUILD=1 ./capture.sh # reuse the binaries already in target/release
#
# Each scene spins up an isolated, headless nested sway (virtual outputs, never
# touches your real screens), drives the tool and captures stills + animations
# into ../../docs/assets/<tool>/. Safe to run alongside a live session.
set -u
cd "$(dirname "$0")"
. ./lib.sh

ALL=(draw draw-present switcher chooser shot peek peek-cli mirror)
SCENES=("$@"); [ ${#SCENES[@]} -eq 0 ] && SCENES=("${ALL[@]}")

# Fetch uBlock Origin Lite (unpacked) into vendor/ubol if missing, so the demo
# browsers block ads in the captures. Best-effort: scenes still run without it.
if [ ! -f "$SHOTS_UBO/manifest.json" ] && command -v gh >/dev/null 2>&1; then
  shots_msg "fetching uBlock Origin Lite…"
  mkdir -p "$SHOTS_DIR/vendor"
  if gh release download --repo uBlockOrigin/uBOL-home \
       --pattern 'uBOLite_*.chromium.zip' -D "$SHOTS_DIR/vendor" --clobber 2>/dev/null; then
    rm -rf "$SHOTS_UBO"; mkdir -p "$SHOTS_UBO"
    unzip -q "$SHOTS_DIR"/vendor/uBOLite_*.chromium.zip -d "$SHOTS_UBO" 2>/dev/null
  fi
fi

# Build the tools, each in its OWN cargo invocation. Building them together lets
# Cargo feature-unification enable wlr-capture/gpu (pulled in by another crate's
# defaults); wlr-peek would then capture via dma-buf and crash its overlay with
# `eglCreateWindowSurface: BadAlloc`. Isolated builds keep every tool on the shm
# capture path, which the nested compositor needs.
if [ -z "${SKIP_BUILD:-}" ]; then
  # The virtual pointer + keyboard injector (drives the overlays; holds Shift for
  # wlr-draw's spotlight and Tab for the exposé selection).
  [ -x "$SHOTS_POINTER" ] || ( cd "$SHOTS_DIR/pointer" && cargo build --release ) \
    || { shots_msg "injector build FAILED"; exit 1; }
  for crate in wlr-draw wlr-chooser wlr-shot wlr-peek; do
    shots_msg "build $crate"
    ( cd "$SHOTS_REPO" && cargo build --release -p "$crate" ) \
      || { shots_msg "build $crate FAILED"; exit 1; }
  done
fi
export SHOTS_BIN="$SHOTS_REPO/target/release"

# wlr-pip is a deprecation stub; it has no showcase scene.
for s in "${SCENES[@]}"; do
  if [ ! -f "scenes/$s.sh" ]; then shots_msg "no scene '$s' — skipping"; continue; fi
  shots_msg "=== scene: $s ==="
  bash "scenes/$s.sh" || shots_msg "scene $s FAILED (continuing)"
done

shots_msg "assets:"
find "$SHOTS_ASSETS" -type f \( -name '*.png' -o -name '*.gif' -o -name '*.webp' -o -name '*.apng' \) \
  -printf '  %p (%k KB)\n' 2>/dev/null | sort
shots_msg "done."
