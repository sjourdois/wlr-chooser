# Releasing

This project distributes via **GitHub Releases** (binaries, built by
[cargo-dist]), **crates.io**, **AUR**, and a **`.deb`** (built by [cargo-deb]).
The whole workspace is versioned as one block: every crate shares the
`[workspace.package]` version and they release together.

## One-time setup

```sh
cargo install cargo-dist cargo-deb

# Generate the release workflow (.github/workflows/release.yml) from the
# [workspace.metadata.dist] config; commit it.
dist init --yes
git add .github/workflows/release.yml Cargo.toml && git commit -m "ci: cargo-dist release workflow"
```

Repository secrets needed on GitHub:

- `CARGO_REGISTRY_TOKEN` — a crates.io API token; the `publish` workflow uses it to
  publish on tag. Set it with `gh secret set CARGO_REGISTRY_TOKEN`.

## Before you tag: review the docs that drift

After any structural change (a new crate, a moved module, a renamed feature), walk
this list and fix what no longer matches the code. This is the step that is easy to
skip and expensive to miss:

- [ ] **Root [`README.md`](README.md)** — the tool table, the shared-library paragraph
      (which crates), Requirements, Install, the Documentation links.
- [ ] **Every crate README** — one per crate, they must all exist and be current:
      `crates/wlr-capture`, `crates/wlr-i18n`, `crates/wlr-chooser`, `crates/wlr-shot`,
      `crates/wlr-peek`, `crates/wlr-draw`, `crates/wlr-utils`. A **new crate** needs its
      own `README.md` *and* a `readme = "README.md"` line in its `Cargo.toml`.
- [ ] **[`CONTRIBUTING.md`](CONTRIBUTING.md)** — the workspace crate table, the
      feature-combo list, the Translations and Themes sections.
- [ ] **[`COMPATIBILITY.md`](COMPATIBILITY.md)** — compositor floors / capability matrix.
- [ ] **`docs/`** — `wlr-draw-keys.toml`, `themes/`, `index.md` (the showcase site).

Quick sanity greps (adjust to the change):

```sh
# every crate has a README and declares it
for d in crates/*/; do
  test -f "$d/README.md" || echo "MISSING README: $d"
  grep -q '^readme = ' "$d/Cargo.toml" || echo "MISSING readme field: $d/Cargo.toml"
done
grep 'for pkg in' .github/workflows/publish.yml   # every crate in the publish order
```

## Cutting a release `vX.Y.Z`

1. **Bump the version.** It lives in `[workspace.package]` **and** in each inter-crate
   dependency pin (the `version = "X.Y.Z"` next to `path = "../wlr-…"`). `wlr-capture`
   and `wlr-i18n` inherit via `version.workspace = true`; the tool crates pin the
   engine/i18n version explicitly, so those pins must move too. `cargo set-version X.Y.Z`
   (from `cargo-edit`) handles both; verify the pins and refresh `Cargo.lock`.
2. **Update [`CHANGELOG.md`](CHANGELOG.md)** — a `## X.Y.Z — YYYY-MM-DD` section
   (Added / Changed / Fixed), referencing the issues/PRs it closes. Commit
   (`chore(release): X.Y.Z`) as the last commit of the release PR.
3. **Tag and push:**
   ```sh
   git tag vX.Y.Z
   git push --tags
   ```
   - The `publish` workflow publishes each crate to **crates.io** in dependency order
     (`for pkg in …` in `.github/workflows/publish.yml`: a crate before anything that
     depends on it — currently
     `wlr-capture wlr-i18n wlr-chooser wlr-shot wlr-peek wlr-draw wlr-utils`).
   - The cargo-dist `release` workflow builds the binaries + installer and creates
     the GitHub Release.
   - The `deb` workflow builds the `.deb` per distro and attaches it to that release
     (only crates with `[package.metadata.deb]` ship there).
4. **AUR** (once an AUR account exists): in `packaging/aur/`, bump `pkgver`, run
   `updpkgsums` to fill the `sha256sums`, regenerate `.SRCINFO`
   (`makepkg --printsrcinfo > .SRCINFO`), and push to the `wlr-utils` and
   `wlr-utils-bin` AUR repositories (both build the whole suite of binaries). The
   `-bin` package's `package()` paths may need adjusting to match the actual
   cargo-dist archive layout.

## Checks before tagging

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build -p wlr-utils          # the bundle isn't in the default set
cargo check --locked              # Cargo.lock is up to date
```

When the engine changed, also spot-check its feature combos (see CONTRIBUTING).

[cargo-dist]: https://opensource.axo.dev/cargo-dist/
[cargo-deb]: https://github.com/kornelski/cargo-deb
