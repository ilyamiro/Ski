# Usage in configuration.nix (or import nix/module.nix and set programs.ski.enable = true):
#   environment.systemPackages = [ (pkgs.callPackage /path/to/Ski { }) ];
{ pkgs ? import <nixpkgs> { } }:

let
  inherit (pkgs) lib;

  runtimeLibs = with pkgs; [
    libGL
    wayland
    libxkbcommon
    libx11
    libxcursor
    libxi
    libxrandr
    vulkan-loader
  ];

  runtimeTools = with pkgs; [
    xdg-utils
    fontconfig
    gnutar
    p7zip
  ];
in
pkgs.rustPlatform.buildRustPackage {
  pname = "ski";
  version = "0.1.0";

  # Only what the build reads, so editing the README, docs or nix files doesn't rebuild
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
      ./assets
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];

  postInstall = ''
    install -Dm644 assets/ski.desktop $out/share/applications/ski.desktop
    install -Dm644 assets/logo.svg $out/share/icons/hicolor/scalable/apps/ski.svg

    install -Dm644 assets/ski.portal $out/share/xdg-desktop-portal/portals/ski.portal
    for name in org.freedesktop.impl.portal.desktop.ski org.freedesktop.FileManager1; do
      install -Dm644 assets/$name.service $out/share/dbus-1/services/$name.service
      substituteInPlace $out/share/dbus-1/services/$name.service --replace-fail @out@ $out
    done

    wrapProgram $out/bin/ski \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs} \
      --prefix PATH : ${lib.makeBinPath runtimeTools} \
      --suffix XDG_DATA_DIRS : ${pkgs.shared-mime-info}/share
  '';

  meta = {
    description = "A small, fast, keyboard-friendly file manager";
    homepage = "https://github.com/ilyamiro/Ski";
    license = lib.licenses.mit;
    mainProgram = "ski";
    platforms = lib.platforms.linux;
  };
}
