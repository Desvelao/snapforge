# Releasing

Releases are built and published by `.github/workflows/release.yml`.

1. Set the new `version` in `app.toml` (the single source) and the same value in the workspace `Cargo.toml`, then update `Cargo.lock`. A test fails if the two differ.
2. In `CHANGELOG.md`, rename *Unreleased* to the version (`## 0.2.0`) and start a new *Unreleased*.
3. Commit, then tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.

The workflow checks that the tag matches the crate version, runs `make package` and
`make package-check` in Docker, and creates a GitHub release with the x86_64 and aarch64
tarballs, `.deb` and `.rpm` files, `SHA256SUMS` and `scripts/get.sh` (the `curl | sh` installer, which must stay a release asset). The release notes are the matching
CHANGELOG section (or *Unreleased* if there is none).

Running the workflow by hand (*Actions → Release → Run workflow*) builds everything and
uploads it as a workflow artifact, without creating a release.
