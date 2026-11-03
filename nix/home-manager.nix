# home-manager module: Ski for one user, with its settings and themes kept in the configuration.
#
#   imports = [ /path/to/Ski/nix/home-manager.nix ];
#   programs.ski = {
#     enable = true;
#     settings = { theme = "system"; icon_style = "System"; scroll_speed = 1.5; show_hidden = true; };
#   };
#
# The settings go to ~/.config/ski/managed.json, which Ski lays over its own settings.json on every
# read: what is set here wins, and whatever is left out stays changeable in Ski's Settings window.
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.ski;
  json = pkgs.formats.json { };
in
{
  options.programs.ski = {
    enable = lib.mkEnableOption "Ski, a small keyboard-friendly file manager";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ../default.nix { };
      defaultText = lib.literalExpression "pkgs.callPackage /path/to/Ski { }";
      description = "The Ski package to use: the latest release binary by default; `pkgs.callPackage /path/to/Ski { fromSource = true; }` compiles the checkout.";
    };

    settings = lib.mkOption {
      type = json.type;
      default = { };
      example = lib.literalExpression ''
        {
          theme = "system";          # "system", a bundled palette such as "nord", or "custom:<id>"
          accent = "#8ab4f8";
          icon_style = "System";     # System, Classic, Halloween, Winter, Pixel, Sketch, Glossy, Clay, Origami
          scroll_speed = 1.5;        # wheel notch, times 60 points; 1.0 is the default
          view_mode = "Grid";
          show_hidden = true;
          single_click_open = false;
          plugins = { git = true; duplicates = false; };
          keybinds = { toggle_hidden = [ "Ctrl+Shift+H" ]; };
        }
      '';
      description = ''
        Ski's settings, as the keys of `~/.config/ski/settings.json`. They are written to
        `~/.config/ski/managed.json`, which wins over `settings.json` key by key (maps such as
        `plugins` and `keybinds` merge), so a setting named here cannot be changed for good in
        the Settings window, and every setting left out can. A running Ski takes up a new
        generation at once. The Settings wiki page lists every key.
      '';
    };

    themes = lib.mkOption {
      type = lib.types.attrsOf json.type;
      default = { };
      example = lib.literalExpression ''
        {
          paper = {
            name = "Paper";
            base = "#fbf8f1"; mantle = "#f2ede3"; crust = "#e9e3d6";
            text = "#2b2620"; subtext0 = "#5a5248"; subtext1 = "#7d7468";
            surface0 = "#f5f1e8"; surface1 = "#e7e0d2"; surface2 = "#dbd3c3";
            accent = "#b5651d";
          };
        }
      '';
      description = ''
        Palettes of your own, by id: each becomes `~/.local/state/ski/themes/<id>.json` and is
        picked with `settings.theme = "custom:<id>"`. Colors left out keep the Dark palette's.
      '';
    };

    defaultFileManager = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Open folders with Ski: makes it the `inode/directory` handler in home-manager's
        `xdg.mimeApps`, which this turns on (home-manager then owns `~/.config/mimeapps.list`).
        "Show in folder" from browsers (`org.freedesktop.FileManager1`) needs Ski's D-Bus
        service on the session bus, which the NixOS module (`nix/module.nix`) or a distribution
        package provides.
      '';
    };

    filePicker = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Use Ski as the xdg-desktop-portal FileChooser through home-manager's `xdg.portal`, so
        the "Open" and "Save as" dialogs of portal-aware programs are Ski. On NixOS the system
        module's `filePicker` does the same for every user.
      '';
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      home.packages = [ cfg.package ];
    }

    (lib.mkIf (cfg.settings != { }) {
      xdg.configFile."ski/managed.json".source = json.generate "ski-managed.json" cfg.settings;
    })

    {
      home.file = lib.mapAttrs' (id: theme: lib.nameValuePair "${config.xdg.stateHome}/ski/themes/${id}.json" {
        source = json.generate "ski-theme-${id}.json" theme;
      }) cfg.themes;
    }

    (lib.mkIf cfg.defaultFileManager {
      xdg.mimeApps.enable = lib.mkDefault true;
      xdg.mimeApps.defaultApplications."inode/directory" = "ski.desktop";
    })

    (lib.mkIf cfg.filePicker {
      xdg.portal.enable = true;
      xdg.portal.extraPortals = [ cfg.package ];
      xdg.portal.config.common."org.freedesktop.impl.portal.FileChooser" = "ski";
      # GTK 3 programs only ask the portal when told to
      home.sessionVariables.GTK_USE_PORTAL = "1";
    })
  ]);
}
