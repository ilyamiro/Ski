# Releasing

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`, which builds Ski and publishes a GitHub release with the `.tar.gz`, `.deb`, `.rpm` and `SHA256SUMS`. The same tag also makes `docker.yml` push the container image. Nothing is built or uploaded by hand.

A release has four parts: test, tag and publish, send the update to nixpkgs, then bump the Ski version Serpantinum bundles. A patch release takes about 15 minutes in total, most of it waiting for builds.

## Before you start

- **Push over SSH, never HTTPS.** An HTTPS remote makes git ask for a GitHub password, which doesn't work. Ski's `origin` is `git@github.com:ilyamiro/Ski.git`. In the nixpkgs checkout (`~/Projects/nixpkgs`), push to `git@github.com:ilyamiro/nixpkgs.git`. If you see `Permission denied (publickey)`, run `gitavail` to load the key and retry.
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

## 3. Bring the release into nixpkgs

Ski is in nixpkgs (`pkgs/by-name/sk/ski/package.nix`, maintainer `ilyamiro`), and `passthru.updateScript` lets the `r-ryantm` bot open a `ski: X -> Y` pull request on its own, but only after days. To get the version into `nixos-unstable` sooner, open that pull request yourself right after the release. Each release is a **new, separate** pull request from a new branch off the current nixpkgs `master`; never reuse or rewrite an old branch.

1. Check nobody has done it yet: `gh pr list --repo NixOS/nixpkgs --search "ski: in:title" --state open`. If the bot's pull request is already there, skip to step 6 and merge that one.
2. In `~/Projects/nixpkgs`, start from upstream, not from an old branch:
   ```sh
   git fetch --depth=1 https://github.com/NixOS/nixpkgs.git master
   git switch -c ski-X.Y.Z FETCH_HEAD
   ```
3. Update the package: `nix-shell maintainers/scripts/update.nix --argstr package ski`. It changes `version`, `hash` and `cargoHash` in `pkgs/by-name/sk/ski/package.nix` and nothing else; check with `git diff`. If it fails, do it by hand: `nix-prefetch-url --unpack --name source https://github.com/ilyamiro/Ski/archive/refs/tags/vX.Y.Z.tar.gz`, convert with `nix-hash --type sha256 --to-sri`, set `cargoHash = "";`, run `timeout 400 nix-build -A ski.cargoDeps --no-out-link` and copy the `got:` value.
4. Build it: `timeout 900 nix-build -A ski --no-out-link`. It must succeed. If the release added a run-time library or program, `runtimeLibs` / `runtimeTools` in `package.nix` need it too.
5. Commit `ski: A -> X.Y.Z` (no AI attribution), push over SSH and open the pull request:
   ```sh
   git push git@github.com:ilyamiro/nixpkgs.git HEAD:ski-X.Y.Z
   gh pr create --repo NixOS/nixpkgs --base master --head ilyamiro:ski-X.Y.Z --title "ski: A -> X.Y.Z" --body "https://github.com/ilyamiro/Ski/releases/tag/vX.Y.Z"
   ```
6. When CI is green, the maintainer merges it by commenting `@NixOS/nixpkgs-merge-bot merge` on the pull request. After the merge the version reaches `nixos-unstable` when the channel next advances, usually within a few days.

## 4. Update the Ski version in Serpantinum

Serpantinum's AUR packages bundle a pinned Ski release binary, so each release bumps it there too. In `~/Projects/serpantinum` (`git pull` first), both `packaging/aur/serpantinum/PKGBUILD` and `packaging/aur/serpantinum-git/PKGBUILD` have:

- `_skiver=X.Y.Z`: the new version.
- The Ski tarball's line in `sha256sums` (the second entry): the new tarball's hash, from the release's checksums:
  ```sh
  gh release download vX.Y.Z --repo ilyamiro/Ski -p SHA256SUMS -O - | grep x86_64-linux.tar.gz
  ```

Find any other pinned version with `grep -rn "<old version>" --exclude-dir=.git .`. Commit with `chore(aur): bundle Ski X.Y.Z` (Serpantinum's own `AGENTS.md` asks for one-line conventional commits) and push `master`.

## Things that go wrong

- The workflow fails at "Check the tag matches Cargo.toml": the tag and `version` differ. Fix the version, commit, then move the tag (`git tag -d`, `git push origin :refs/tags/vX.Y.Z`, tag again), as long as the release was not published yet.
- Never reuse a tag for a release that has already been published; cut a new patch version.
- New files that the packages must contain go into `packaging/stage.sh` and then into both `[package.metadata.deb]` and `[package.metadata.generate-rpm]` in `Cargo.toml`. A new run-time library goes into the `depends` of the deb and the `requires` of the rpm, since neither is detected automatically. The nixpkgs package needs it too: a library Ski loads at run time goes into `runtimeLibs`, a program it runs into `runtimeTools`.
- Translation files are built into the binary (`include_str!`), so a new language needs no packaging change. It needs its `src/i18n/<code>.json` and its arms in the `Language` matches in `src/i18n/mod.rs`.
- Builds are x86_64 only, on `ubuntu-22.04` so that the binary runs on older glibc. Do not move the runner to a newer image without a reason.
- `git push` asks for a password: the remote is HTTPS. Push to the `git@github.com:` URL instead.
- Do not add a `Co-Authored-By: Claude` line, or any other AI attribution, to commits or pull requests (see `AGENTS.md`).
