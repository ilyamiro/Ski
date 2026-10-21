# Releasing

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`, which builds Ski and publishes a GitHub release with the `.tar.gz`, `.deb`, `.rpm` and `SHA256SUMS`. The same tag also makes `docker.yml` push the container image. Nothing is built or uploaded by hand.

## Steps

1. Start from a clean, up-to-date `master` (`git status`, `git pull`). Release only what is committed and pushed.
2. Pick the version (semver; while below 1.0, a new feature bumps the minor, a fix the patch).
3. Set it in all three places, which must agree:
   - `Cargo.toml`: `version`
   - `Cargo.lock`: the `ski` entry. Cargo updates it on the next `cargo build --release`; check that `git diff Cargo.lock` shows only that one line.
   - `default.nix`: `version` in `common`
4. Check it builds the way CI does: `cargo build --release --locked`.
5. Commit with `chore: release vX.Y.Z` and push `master`.
6. Optional dry run, which builds and packages but publishes nothing: `gh workflow run release.yml --ref master`, then `gh run watch`.
7. Tag and push:
   ```sh
   git tag -a vX.Y.Z -m "Ski X.Y.Z"
   git push origin vX.Y.Z
   ```
8. Wait for the run (about 8 minutes): `gh run list --workflow=release.yml --limit 1`, then `gh run watch <id> --exit-status`.
9. Confirm the release has four assets: `gh release view vX.Y.Z --json assets -q '.assets[].name'`. The notes are generated from the commits since the last tag; edit them with `gh release edit` if they need tidying.

## Things that go wrong

- The workflow fails at "Check the tag matches Cargo.toml": the tag and `version` differ. Fix the version, commit, then move the tag (`git tag -d`, `git push origin :refs/tags/vX.Y.Z`, tag again), as long as the release was not published yet.
- Never reuse a tag for a release that has already been published; cut a new patch version.
- New files that the packages must contain go into `packaging/stage.sh` and then into both `[package.metadata.deb]` and `[package.metadata.generate-rpm]` in `Cargo.toml`. A new run-time library goes into the `depends` of the deb and the `requires` of the rpm, since neither is detected automatically.
- Builds are x86_64 only, on `ubuntu-22.04` so that the binary runs on older glibc. Do not move the runner to a newer image without a reason.
- Do not add a `Co-Authored-By: Claude` line to commits (see `AGENTS.md`).
