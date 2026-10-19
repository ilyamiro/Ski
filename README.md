# Ski

A small, fast, keyboard-friendly file manager for Linux, written in Rust with [egui](https://github.com/emilk/egui).
Flat Material look, no borders, and colors that follow your [Serpantinum](https://github.com/ilyamiro) theme.

## Features

- **Browse** in list or grid view, with image thumbnails, custom folder icons, pinned folders and global search
- **Navigate** with breadcrumbs, a sidebar with Recent, Trash, your folders, pinned folders and mounted drives, and the back / forward mouse buttons
- **Sort from the list header**: click Name, Size or Modified; column titles line up with their values
- **Select** like any file manager: Ctrl+click, Shift+click, Shift+arrows, Ctrl+A; click empty space or press Esc to deselect
- **Copy, cut, paste**, plus **Copy to…** / **Move to…** with a folder chooser; nothing is ever overwritten. Files waiting to be moved are shown faded, and files dropped on the window are copied into the folder
- **Rename**, **move to trash** with an Undo in the toast, **restore** from the trash, and permanent deletion that asks first (the freedesktop trash, shared with other file managers)
- **Compress** to `.zip`, `.tar.gz`, `.tar.xz`, `.tar.bz2`, `.tar.zst` or `.7z`; **extract** here or to a folder you pick
- **Open with…**: default, recommended and all installed apps, with icons and search
- **Properties** with type-specific details (image size, text statistics, folder totals)
- **Settings**: themes (System, Midnight, Nord, Gruvbox, Light), accent color, font, interface size, animations, default view and sorting, status bar, deletion confirmation, rebindable shortcuts, and default apps per file kind
- Context menus show the keyboard shortcut of every action, and a status bar counts items and selected size
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
| `Alt+Left` / `Alt+Right` | History (also the mouse back / forward buttons) |
| `Alt+Up` / `Alt+Home` | Parent folder / Home |
| `/` | Filter by name |
| `Ctrl+L` | Edit path |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `F2` | Rename |
| `Delete` / `Shift+Delete` | Move to trash / delete permanently |
| `Ctrl+Shift+N` / `Ctrl+Alt+N` | New folder / new file |
| `Esc` | Close a dialog or menu, clear the filter, then deselect |
| `Ctrl+H` | Toggle hidden files |
| `Ctrl+1` / `Ctrl+2` | List / grid |
| `Ctrl+D` | Pin folder |
| `Alt+Enter` | Properties |
| `y` | Copy path |
| `F5` | Refresh |
| `Ctrl+,` | Settings |
| `?` | Keyboard shortcuts |
| `Ctrl+Q` / `Ctrl+W` | Close the window |

Preferences are stored in `~/.config/ski/`.

## License

[MIT](LICENSE). The bundled fonts keep their own licenses: Adwaita Sans (SIL OFL) and Symbols Nerd Font Mono (MIT).
