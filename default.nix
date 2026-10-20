# Usage in configuration.nix:
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

  src = lib.cleanSourceWith {
    src = ./.;
    filter = path: type:
      let name = baseNameOf path; in
      !(builtins.elem name [ "target" ".git" ".direnv" ".omc" ".claude" ".agents" ]);
  };

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];

  postInstall = ''
    install -Dm644 assets/ski.desktop $out/share/applications/ski.desktop
    install -Dm644 assets/logo.svg $out/share/icons/hicolor/scalable/apps/ski.svg

    wrapProgram $out/bin/ski \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs} \
      --prefix PATH : ${lib.makeBinPath runtimeTools}
  '';

  meta = {
    description = "A small, fast, keyboard-friendly file manager";
    homepage = "https://github.com/ilyamiro/Ski";
    license = lib.licenses.mit;
    mainProgram = "ski";
    platforms = lib.platforms.linux;
  };
}
