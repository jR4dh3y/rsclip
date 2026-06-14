# rsclip-bin

This AUR package repackages the GitHub release archive produced by this
repository.

## Release flow

1. Bump the crate versions in `crates/*/Cargo.toml` when needed.
2. Push a matching tag such as `v0.1.9`.
3. GitHub Actions uploads `rsclip-0.1.9-x86_64.tar.zst` and its `.sha256`
   file to the release.
4. GitHub Actions updates `PKGBUILD` and `.SRCINFO`, then publishes them to the
   AUR as `rsclip-bin`.

The workflow expects an `AUR_SSH_PRIVATE_KEY` repository secret and verifies the
`aur.archlinux.org` host keys against pinned fingerprints before pushing.
