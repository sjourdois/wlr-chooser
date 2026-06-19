# Contributing to wlr-chooser

Thanks for your interest! Bug reports, translations, themes and patches are all
welcome.

## Building & checks

```sh
cargo build
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Please make sure `fmt`, `clippy` and `test` are clean before opening a pull
request. CI runs the same checks.

### Testing the UI without disturbing your screen

The picker is a layer-shell overlay, so it covers your screen. To iterate without
that, run it inside a nested **headless** Sway and screenshot it:

```sh
env -u WAYLAND_DISPLAY -u DISPLAY WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
    WLR_HEADLESS_OUTPUTS=4 sway -c headless.conf
```

where `headless.conf` sets `output HEADLESS-1 resolution 1400x900` and an `exec`
that launches `wlr-chooser`, sleeps, runs `grim`, then `swaymsg exit`. Use a GPU
renderer (not `pixman`) since EGL is required. The overlay does not appear in
`swaymsg -t get_tree`; find the output it rendered on by picking the non-black
`grim`.

## Translations

Catalogs are Fluent files under `i18n/<lang>/wlr_chooser.ftl`. To add a language,
copy `i18n/en/wlr_chooser.ftl`, translate the values (keep the `{ $name }`
placeables), and add the file. `cargo test` checks that every catalog parses; the
English catalog is the source of truth and the per-message fallback.

CJK languages render via an auto-detected CJK font (e.g. Noto Sans CJK).

## Themes

A theme is a `theme.toml` of colours (and optional fonts). Add new palettes to
`docs/themes/`. Keys are documented in `src/theme.rs` and the README.

## Commit messages & license

Conventional-commit style (`feat:`, `fix:`, `docs:` …) is appreciated. By
contributing, you agree that your contributions are dual-licensed under
Apache-2.0 and MIT, the same terms as the project.
