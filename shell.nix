{ pkgs ? import <nixpkgs> {} }:

let
  guiLibraries = with pkgs; [
    libGL
    wayland
    libxkbcommon
    libx11
    libxcursor
    libxi
    libxrandr
    vulkan-loader
  ];
in
pkgs.mkShell {
  buildInputs = with pkgs; [
    rustc
    cargo
    rustfmt
    clippy
    rust-analyzer
    pkg-config
    openssl
  ] ++ guiLibraries;

  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
  RUST_BACKTRACE = 1;

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibraries;
}
