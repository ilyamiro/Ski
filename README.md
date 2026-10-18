# Ski

A small, fast, keyboard-friendly file manager for Linux, written in Rust with [egui](https://github.com/emilk/egui).
Flat Material look, no borders, and colors that follow your [Serpantinum](https://github.com/ilyamiro) theme.

## Features

- **Browse** in list or grid view, with image thumbnails, custom folder icons, pinned folders and global search
- **Select** like any file manager: Ctrl+click, Shift+click, Shift+arrows, Ctrl+A
- **Copy, cut, paste**, plus **Copy to…** / **Move to…** with a folder chooser; nothing is ever overwritten
- **Compress** to `.zip`, `.tar.gz`, `.tar.xz`, `.tar.bz2`, `.tar.zst` or `.7z`; **extract** here or to a folder you pick
- **Open with…**: default, recommended and all installed apps, with icons and search
- **Properties** with type-specific details (image size, text statistics, folder totals)
- **Settings**: themes (System, Midnight, Nord, Gruvbox, Light), accent color, font, interface size, default view and sorting, and default apps per file kind
- Vim-style keys (`j` `k` `h` `l`) alongside the arrow keys

## Build and run

Needs a recent stable Rust toolchain and the usual GUI libraries (Wayland/X11, Vulkan or OpenGL).
On NixOS or with Nix:

```sh
nix-shell
cargo run --release -- [folder]
```

Elsewhere, `cargo run --release -- [folder]` works once the GUI libraries are installed.

## Archives

`.zip` and `.tar.gz` are handled in-process. The other formats use the system's `tar` with `xz`, `bzip2` or `zstd`, and `7z`, and only appear when those tools are installed. Extracting `.rar`, `.iso` and similar needs `7z`.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `j` / `k`, arrows | Move selection |
| `Shift` + arrows, `Shift`/`Ctrl` + click | Select several |
| `Ctrl+A` | Select all |
| `Enter` / `l` | Open |
| `Backspace` / `h` | Parent folder |
| `Alt+Left` / `Alt+Right` | History |
| `/` | Filter by name |
| `Ctrl+L` | Edit path |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Ctrl+H` | Toggle hidden files |
| `Ctrl+1` / `Ctrl+2` | List / grid |
| `Ctrl+D` | Pin folder |
| `Alt+Enter` | Properties |
| `y` | Copy path |
| `F5` | Refresh |
| `?` | Settings |

Preferences are stored in `~/.config/ski/`.

## License

[MIT](LICENSE). The bundled fonts keep their own licenses: Adwaita Sans (SIL OFL) and Symbols Nerd Font Mono (MIT).
