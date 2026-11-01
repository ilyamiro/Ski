# Releasing

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`, which builds Ski and publishes a GitHub release with the `.tar.gz`, `.deb`, `.rpm` and `SHA256SUMS`. The same tag also makes `docker.yml` push the container image. Nothing is built or uploaded by hand.

A release has three parts: test, tag and publish, then bring the nixpkgs package up to the new version. A patch release takes about 15 minutes in total, most of it waiting for builds.

## Before you start

- **Push over SSH, never HTTPS.** An HTTPS remote makes git ask for a GitHub password, which doesn't work. Ski's `origin` is `git@github.com:ilyamiro/Ski.git`. In the nixpkgs checkout (`~/Projects/nixpkgs`), push to `git@github.com:ilyamiro/nixpkgs.git` rather than through an HTTPS remote. If you see `Permission denied (publickey)`, run `gitavail` to load the key and retry.
- **Run cargo inside the Nix shell:** `nix-shell --run "cargo …"`. Outside it, the build has no system libraries, and the binary fails with `NoWaylandLib`.
- **Use timeouts.** Give long commands a `timeout`, so a stuck build fails instead of hanging. A clean release build takes about 5 minutes and an incremental one 1–2 minutes. Never wait for a build with `pgrep -f "cargo build"`: the waiting shell's own command line matches the pattern, so the loop waits forever.
- **Merge pull requests first**, for example translations, with `gh pr merge <n> --squash`. Then `git pull`.

## 1. Test

1. Start from a clean, up-to-date `master`. `git pull`, then `git status -sb` must show `## master...origin/master` with no `[ahead]` or `[behind]`. Release only what is committed and pushed.
2. Run the whole test suite: `nix-shell --run "cargo test --locked"`. All tests must pass.
   - `app::tests::rename_trash_restore_delete` sometimes fails while removing its temporary folder (`src/app.rs`, the last `remove_dir_all`) when the tests run in parallel. If it is the only failure, rerun it alone (`cargo test rename_trash_restore_delete`) and then the whole suite again. Any other failure blocks the release.
   - The `i18n` tests check every translation file (`src/i18n/<code>.json`): each must parse, cover every key in the sources, keep the `{}` holes, have enough plural forms, and contain no duplicate or unused entries.
3. Smoke-test the release binary for a few seconds, in a non-English language too:
   ```sh
   nix-shell --run "LANG=vi_VN.UTF-8 timeout 5 ./target/release/ski"
   ```
   Exit code 124 (stopped by `timeout`) with no output is a pass. A message like `the vi translation is broken` means that JSON file doesn't parse.

## 2. Release

1. Pick the version (semver; while below 1.0, a new feature bumps the minor, a fix the patch).
2. Set it in both places, which must agree (the Nix source build reads it from `Cargo.toml`):
   - `Cargo.toml`: `version`
   - `Cargo.lock`: the `version` line of the `ski` entry. Edit it by hand or let `cargo build --release` update it; `git diff Cargo.lock` must show only that one line.
3. Check it builds the way CI does: `nix-shell --run "cargo build --release --locked"`.
4. Commit with `chore: release vX.Y.Z` and push `master`.
5. Optional dry run, which builds and packages but publishes nothing: `gh workflow run release.yml --ref master`, then `gh run watch`.
6. Tag and push:
   ```sh
   git tag -a vX.Y.Z -m "Ski X.Y.Z"
   git push origin vX.Y.Z
   ```
7. Wait for the run (about 4–8 minutes):
   ```sh
   gh run list --workflow=release.yml --limit 1
   gh run watch <id> --exit-status
   gh run view <id> --json conclusion   # must be "success"
   ```
   The warning that Node.js 20 is deprecated is harmless.
8. Confirm the release has four assets: `gh release view vX.Y.Z --json assets -q '.assets[].name'`. The notes are generated from the commits since the last tag; edit them with `gh release edit` if they need tidying.
9. The run's last step commits `nix/release.json` (the new version and the tarball's hash) to `master`, which makes the Nix package (`default.nix`, `nix/bin.nix`) install the new binary instead of compiling. `git pull` before the next commit.

Never edit or re-upload a release's tarball after that commit: its hash would no longer match and the Nix install would fail. To fix one by hand, `nix-prefetch-url <tarball url>` and put `sha256-` plus the base64 of that hash (`nix-hash --type sha256 --to-sri <hash>`) into `nix/release.json`.

## 3. Update the nixpkgs pull request

While Ski is not yet in nixpkgs, the package is added by an open pull request, [NixOS/nixpkgs#581531](https://github.com/NixOS/nixpkgs/pull/581531), from the `ski-init` branch of `ilyamiro/nixpkgs`. Each release updates **that same pull request**. Never close it or open a new one: a broken force-push already got an earlier one (#581489) closed by GitHub. Once it is merged, this section is replaced by ordinary `ski: X -> Y` update pull requests, which `passthru.updateScript` (nix-update) can prepare.

The package is `pkgs/by-name/sk/ski/package.nix`. A release changes three lines: `version`, the `src` `hash` and `cargoHash`.

1. Go to `~/Projects/nixpkgs`. Check that the local `ski-init` is the pull request's current head:
   ```sh
   gh pr view 581531 --repo NixOS/nixpkgs --json headRefOid -q .headRefOid
   git rev-parse HEAD
   ```
   The `fork/ski-init` tracking ref may be stale; the pull request's `headRefOid` is the truth. If HEAD differs, fetch the branch first: `git fetch git@github.com:ilyamiro/nixpkgs.git ski-init`.
2. The checkout is shallow, so the commits may show no parent. Before rewriting anything, fetch the real parents, or an amend would produce an unrelated root commit and wreck the pull request:
   ```sh
   git fetch --deepen=2 git@github.com:ilyamiro/nixpkgs.git ski-init
   git log --format='%h %p %s' -3   # every commit must list a parent
   ```
3. Source hash:
   ```sh
   h=$(nix-prefetch-url --unpack --name source https://github.com/ilyamiro/Ski/archive/refs/tags/vX.Y.Z.tar.gz)
   nix-hash --type sha256 --to-sri $h
   ```
   Put the result in `hash`, set `version`, and set `cargoHash = "";`.
4. Cargo hash: `timeout 400 nix-build -A ski.cargoDeps --no-out-link`. It fails with a hash mismatch; copy the `got:` value into `cargoHash`. To find that line, filter on `got:`, not on `error`, because crate names like `thiserror` match too. The first run downloads every crate (a few minutes); reruns are fast.
5. Check that it evaluates: `nix-instantiate -A ski`. A full build (`nix-build -A ski`, 5–10 minutes) is worth running when the dependencies or build steps changed.
6. Commit and push to the same branch. Either way works and keeps the pull request:
   - **Squash (preferred by nixpkgs reviewers).** Amend the package commit and rename it, then force-push with a lease pinned to the head from step 1, so the push is refused if the branch moved:
     ```sh
     git commit -a --amend -m "ski: init at X.Y.Z"
     git push --force-with-lease=ski-init:<head from step 1> git@github.com:ilyamiro/nixpkgs.git HEAD:ski-init
     ```
     Only do this after step 2.
   - **Plain commit.** Commit `ski: <old> -> X.Y.Z` on top and push normally (`git push git@github.com:ilyamiro/nixpkgs.git HEAD:ski-init`). Nothing is rewritten. Squash it later as above if reviewers ask.
7. Retitle the pull request and check it:
   ```sh
   gh pr edit 581531 --repo NixOS/nixpkgs --title "ski: init at X.Y.Z"
   gh pr view 581531 --repo NixOS/nixpkgs --json state,title,headRefOid,commits -q '.state, .title, .headRefOid, [.commits[].messageHeadline]'
   ```
   It must be `OPEN`, the head must be your new commit, and `maintainers: add ilyamiro` must still be the first commit.

## Things that go wrong

- The workflow fails at "Check the tag matches Cargo.toml": the tag and `version` differ. Fix the version, commit, then move the tag (`git tag -d`, `git push origin :refs/tags/vX.Y.Z`, tag again), as long as the release was not published yet.
- Never reuse a tag for a release that has already been published; cut a new patch version.
- New files that the packages must contain go into `packaging/stage.sh` and then into both `[package.metadata.deb]` and `[package.metadata.generate-rpm]` in `Cargo.toml`. A new run-time library goes into the `depends` of the deb and the `requires` of the rpm, since neither is detected automatically. The nixpkgs package needs it too: a library Ski loads at run time goes into `runtimeLibs`, a program it runs into `runtimeTools`.
- Translation files are built into the binary (`include_str!`), so a new language needs no packaging change. It needs its `src/i18n/<code>.json` and its arms in the `Language` matches in `src/i18n/mod.rs`.
- Builds are x86_64 only, on `ubuntu-22.04` so that the binary runs on older glibc. Do not move the runner to a newer image without a reason.
- `git push` asks for a password: the remote is HTTPS. Push to the `git@github.com:` URL instead.
- Do not add a `Co-Authored-By: Claude` line, or any other AI attribution, to commits or pull requests, in this repository or in nixpkgs (see `AGENTS.md`).
