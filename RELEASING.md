# Releasing

This project distributes via **GitHub Releases** (binaries, built by
[cargo-dist]), **crates.io**, **AUR**, and a **`.deb`** (built by [cargo-deb]).

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

## Cutting a release `vX.Y.Z`

1. Bump `version` in `Cargo.toml`, update `CHANGELOG.md`, commit.
2. Tag and push:
   ```sh
   git tag vX.Y.Z
   git push --tags
   ```
   - The `publish` workflow publishes the crate to **crates.io** automatically.
   - The cargo-dist `release` workflow builds the binaries + installer and creates
     the GitHub Release.
   - The `deb` workflow builds the `.deb` and attaches it to that release.
3. **AUR** (once an AUR account exists): in `packaging/aur/`, bump `pkgver`, run
   `updpkgsums` to fill the
   `sha256sums`, regenerate `.SRCINFO` (`makepkg --printsrcinfo > .SRCINFO`), and
   push to the `wlr-chooser` and `wlr-chooser-bin` AUR repositories. The
   `-bin` package's `package()` paths may need adjusting to match the actual
   cargo-dist archive layout.

## Checks before tagging

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --locked
```

[cargo-dist]: https://opensource.axo.dev/cargo-dist/
[cargo-deb]: https://github.com/kornelski/cargo-deb
