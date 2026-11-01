# Ski from the binary of the latest GitHub release, which installs in seconds instead of compiling.
# release.json names that release; the release workflow updates it on master after each one.
{ pkgs, lib ? pkgs.lib }:

let
  runtime = import ./runtime.nix { inherit lib pkgs; };
  release = lib.importJSON ./release.json;
in
pkgs.stdenv.mkDerivation {
  pname = "ski";
  inherit (release) version;

  src = pkgs.fetchurl {
    url = "https://github.com/ilyamiro/Ski/releases/download/v${release.version}/ski-${release.version}-x86_64-linux.tar.gz";
    inherit (release) hash;
  };

  # The tarball holds usr/ with the binary and the desktop, icon, portal and D-Bus files
  sourceRoot = "usr";

  # The binary links only glibc and libgcc_s; autoPatchelf points it at Nix's
  nativeBuildInputs = with pkgs; [ autoPatchelfHook makeWrapper ];
  buildInputs = [ pkgs.stdenv.cc.cc.lib ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r bin share $out/
    for service in $out/share/dbus-1/services/*.service; do
      substituteInPlace $service --replace-fail /usr/bin/ski $out/bin/ski
    done
    ${runtime.wrapProgram}
    runHook postInstall
  '';

  meta = {
    description = "A small, fast, keyboard-friendly file manager";
    homepage = "https://github.com/ilyamiro/Ski";
    license = lib.licenses.mit;
    mainProgram = "ski";
    platforms = [ "x86_64-linux" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
