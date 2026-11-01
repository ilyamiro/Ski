# Ski built from this checkout. default.nix uses it with `fromSource = true`, and on systems
# other than x86_64-linux, which have no release binary.
{ pkgs, lib ? pkgs.lib }:

let
  runtime = import ./runtime.nix { inherit lib pkgs; };

  # Crane builds the dependencies as a derivation of their own, so editing src/ recompiles only Ski
  # and not egui, resvg and the rest
  crane = fetchTarball {
    url = "https://github.com/ipetkov/crane/archive/refs/tags/v0.24.0.tar.gz";
    sha256 = "06y3dvs9ms4c70r2ia50k77650yrihc09jibqbqrzly23dn42s4m";
  };
  craneLib = import crane { inherit pkgs; };

  version = (lib.importTOML ../Cargo.toml).package.version;

  # Only what the build reads, so editing the README, docs or nix files doesn't rebuild
  src = lib.fileset.toSource {
    root = ./..;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../assets
    ];
  };

  # The dependencies are built from a Cargo.toml and Cargo.lock in which Ski's own version is 0.0.0,
  # so bumping the version for a release doesn't rebuild all of them; only a change to the
  # dependencies (or to nixpkgs' Rust) does
  lockText = builtins.readFile ../Cargo.lock;
  lockWithoutVersion = builtins.replaceStrings
    [ "name = \"ski\"\nversion = \"${version}\"" ]
    [ "name = \"ski\"\nversion = \"0.0.0\"" ]
    lockText;
  depsLock =
    assert lib.assertMsg (lockWithoutVersion != lockText) "Cargo.lock has no ski ${version} entry";
    builtins.toFile "Cargo.lock" lockWithoutVersion;
  depsManifest = craneLib.writeTOML "Cargo.toml" (lib.recursiveUpdate
    (craneLib.cleanCargoToml { cargoToml = ../Cargo.toml; })
    { package.version = "0.0.0"; });
  # Named like `src` so both builds unpack to the same path and cargo's fingerprints carry over
  depsSrc = pkgs.runCommand "source" { } ''
    mkdir -p $out/src
    cp ${depsManifest} $out/Cargo.toml
    cp ${depsLock} $out/Cargo.lock
    echo 'fn main() {}' > $out/src/main.rs
  '';

  common = {
    pname = "ski";
    strictDeps = true;
    cargoVendorDir = craneLib.vendorCargoDeps { cargoLock = depsLock; };

    # The tests are run during development; a user's build doesn't need to compile Ski a second time for them
    doCheck = false;

    # rustc already uses several threads per crate, so one job per core can exhaust the RAM.
    # Eight jobs is a middle ground for a 16-core, 16 GB machine; lower it if the build runs out of memory.
    CARGO_BUILD_JOBS = "8";
    nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
  };

  cargoArtifacts = craneLib.buildDepsOnly (common // {
    version = "0.0.0";
    dummySrc = depsSrc;
    # Crane runs `cargo check` over the dependencies first so that clippy can reuse it; nothing here
    # does, and it would compile every dependency's metadata a second time
    cargoCheckCommand = "true";
  });
in
craneLib.buildPackage (common // {
  inherit src version cargoArtifacts;

  postInstall = ''
    install -Dm644 assets/ski.desktop $out/share/applications/ski.desktop
    install -Dm644 assets/logo.svg $out/share/icons/hicolor/scalable/apps/ski.svg

    install -Dm644 assets/ski.portal $out/share/xdg-desktop-portal/portals/ski.portal
    for name in org.freedesktop.impl.portal.desktop.ski org.freedesktop.FileManager1; do
      install -Dm644 assets/$name.service $out/share/dbus-1/services/$name.service
      substituteInPlace $out/share/dbus-1/services/$name.service --replace-fail @out@ $out
    done

    ${runtime.wrapProgram}
  '';

  meta = {
    description = "A small, fast, keyboard-friendly file manager";
    homepage = "https://github.com/ilyamiro/Ski";
    license = lib.licenses.mit;
    mainProgram = "ski";
    platforms = lib.platforms.linux;
  };
})
