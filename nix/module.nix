# NixOS module: Ski as the default file manager and as the system's open / save dialog.
#
#   imports = [ /path/to/Ski/nix/module.nix ];
#   programs.ski.enable = true;
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.ski;
in
{
  options.programs.ski = {
    enable = lib.mkEnableOption "Ski, a small keyboard-friendly file manager";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ../default.nix { };
      defaultText = lib.literalExpression "pkgs.callPackage ./default.nix { }";
      description = "The Ski package to use.";
    };

    defaultFileManager = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Open folders with Ski (the `inode/directory` handler) and answer
        `org.freedesktop.FileManager1`, which browsers use for "Show in folder".
      '';
    };

    servers = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Turn on GVfs, which Ski uses to open folders on servers (SMB, SFTP, FTP, WebDAV)
        from Connect to server.
      '';
    };

    filePicker = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Use Ski as the xdg-desktop-portal FileChooser, so the "Open" and "Save as" dialogs of
        browsers, Telegram and other portal-aware programs are Ski.
      '';
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    { environment.systemPackages = [ cfg.package ]; }

    (lib.mkIf cfg.defaultFileManager {
      xdg.mime.defaultApplications."inode/directory" = "ski.desktop";
      services.dbus.packages = [ cfg.package ];
    })

    (lib.mkIf cfg.servers {
      services.gvfs.enable = lib.mkDefault true;
    })

    (lib.mkIf cfg.filePicker {
      xdg.portal.enable = true;
      xdg.portal.extraPortals = [ cfg.package ];
      xdg.portal.config.common."org.freedesktop.impl.portal.FileChooser" = "ski";
      # GTK 3 programs only ask the portal when told to
      environment.sessionVariables.GTK_USE_PORTAL = "1";
    })
  ]);
}
