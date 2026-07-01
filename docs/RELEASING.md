# Releasing wlr-utils

The whole workspace is versioned as one block: every crate shares the
`[workspace.package]` version and they release together. This checklist exists so
nothing gets left behind — especially the docs and CI files that go stale after a
structural change (a new crate, a moved module, a renamed feature).

## 1. Review the docs that drift

After any structural change, walk this list and fix what no longer matches the code.
This is the step that is easy to skip and expensive to miss:

- [ ] **Root [`README.md`](../README.md)** — the tool table, the shared-library
      paragraph (which crates), Requirements, Install, the Documentation links.
- [ ] **Every crate README** — one per crate, they must all exist and be current:
      `crates/wlr-capture`, `crates/wlr-i18n`, `crates/wlr-chooser`, `crates/wlr-shot`,
      `crates/wlr-peek`, `crates/wlr-draw`, `crates/wlr-utils`.
      A **new crate** needs its own `README.md` *and* a `readme = "README.md"` line in
      its `Cargo.toml`.
- [ ] **[`CONTRIBUTING.md`](../CONTRIBUTING.md)** — the workspace crate table, the
      feature-combo list, the Translations and Themes sections.
- [ ] **[`COMPATIBILITY.md`](../COMPATIBILITY.md)** — compositor floors / capability
      matrix, if capture requirements changed.
- [ ] **`docs/`** — `wlr-draw-keys.toml`, `themes/`, `index.md` (the showcase site).

Quick sanity greps for a refactor (adjust to the change):

```sh
# every crate has a README and declares it
for d in crates/*/; do
  test -f "$d/README.md" || echo "MISSING README: $d"
  grep -q '^readme = ' "$d/Cargo.toml" || echo "MISSING readme field: $d/Cargo.toml"
done
# every crate is in the publish order
grep 'for pkg in' .github/workflows/publish.yml
```

## 2. Bump the version

The version lives in `[workspace.package]` **and** in each inter-crate dependency pin
(the `version = "X.Y.Z"` next to `path = "../wlr-…"`). `wlr-capture` and `wlr-i18n`
inherit via `version.workspace = true`; the tool crates pin the engine/i18n version
explicitly, so those pins must move too.

- [ ] `Cargo.toml` → `[workspace.package] version`
- [ ] Every `version = "…"` pin on `wlr-capture` / `wlr-i18n` in the tool crates
      (`wlr-chooser`, `wlr-shot`, `wlr-peek`, `wlr-draw`, `wlr-utils`)
- [ ] `Cargo.lock` (refreshed by any `cargo` build/check)

`cargo set-version X.Y.Z` (from `cargo-edit`) handles the first two; verify the pins.

## 3. CHANGELOG

- [ ] Add a `## X.Y.Z — YYYY-MM-DD` section to
      [`CHANGELOG.md`](../CHANGELOG.md) (Added / Changed / Fixed), referencing the
      issues/PRs it closes.

## 4. Verify (same as CI)

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build -p wlr-utils          # the bundle isn't in the default set
cargo check --locked              # Cargo.lock is up to date
```

When the engine changed, also spot-check its feature combos (see CONTRIBUTING).

## 5. CI workflows to keep in sync

- [ ] **`.github/workflows/publish.yml`** — the `for pkg in …` loop must list every
      publishable crate **in dependency order** (a crate before anything that depends
      on it). Current order:
      `wlr-capture wlr-i18n wlr-chooser wlr-shot wlr-peek wlr-draw wlr-utils`.
- [ ] **`.github/workflows/deb.yml`** — per-distro `.deb` build; only crates with
      `[package.metadata.deb]` ship there.
- [ ] **`.github/workflows/release.yml`** — the prebuilt bundle / installer.

## 6. Tag & publish

Commit the release (`chore(release): X.Y.Z`) as the **last** commit of the PR, then
tag and let CI publish in the order above.
