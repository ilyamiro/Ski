# What Ski needs at run time, shared by the source build (package.nix) and the release binary (bin.nix).
# The graphics libraries and fontconfig (egui's fallback for glyphs the bundled fonts lack, such as
# Chinese) are loaded with dlopen, so they go on LD_LIBRARY_PATH rather than being linked.
{ lib, pkgs }:

let
  libs = with pkgs; [
    libGL
    wayland
    libxkbcommon
    libx11
    libxcursor
    libxi
    libxrandr
    fontconfig
  ];

  tools = with pkgs; [
    xdg-utils
    fontconfig
    gnutar
    p7zip
    ffmpeg-headless
  ];
in
{
  inherit libs tools;

  # Wraps an installed $out/bin/ski with the paths above
  wrapProgram = ''
    wrapProgram $out/bin/ski \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath libs} \
      --prefix PATH : ${lib.makeBinPath tools} \
      --suffix XDG_DATA_DIRS : ${pkgs.shared-mime-info}/share
  '';
}
