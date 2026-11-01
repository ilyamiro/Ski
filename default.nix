# Usage in configuration.nix (or import nix/module.nix and set programs.ski.enable = true):
#   environment.systemPackages = [ (pkgs.callPackage /path/to/Ski { }) ];
#
# By default this is the binary of the latest release (nix/release.json), which installs in seconds.
# `{ fromSource = true; }` compiles this checkout instead (nix/package.nix), which takes minutes.
{ pkgs ? import <nixpkgs> { }, fromSource ? false }:

if fromSource || pkgs.stdenv.hostPlatform.system != "x86_64-linux"
then import ./nix/package.nix { inherit pkgs; }
else import ./nix/bin.nix { inherit pkgs; }
