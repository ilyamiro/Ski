//! Rebindable keyboard shortcuts. A binding is a string like `Ctrl+Shift+N` or `j`.
//! Only user overrides are stored in settings; `Action::default_bindings` fills the rest.

use eframe::egui;

use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    MoveDown,
    MoveUp,
    Open,
    ParentFolder,
    Back,
    Forward,
    First,
    Last,
    Filter,
    EditPath,
    ToggleHidden,
    ZoomIn,
    ZoomOut,
    ListView,
    GridView,
    PinFolder,
    Properties,
    Copy,
    Cut,
    Paste,
    SelectAll,
    CopyPath,
    Refresh,
    NewFolder,
    NewWindow,
    Settings,
    ShowShortcuts,
    Rename,
    Trash,
    DeleteForever,
    NewFile,
    GoHome,
    Quit,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    Undo,
    Redo,
    Duplicate,
    MakeLink,
    OpenTerminal,
    OpenAsAdmin,
    InvertSelection,
    SelectPattern,
    SelectNone,
    ContextMenu,
    ShowJobs,
}

impl Action {
    pub const ALL: [Action; 48] = [
        Action::MoveDown, Action::MoveUp, Action::Open, Action::ParentFolder, Action::Back, Action::Forward,
        Action::First, Action::Last, Action::Filter, Action::EditPath, Action::ToggleHidden, Action::ZoomIn, Action::ZoomOut, Action::ListView,
        Action::GridView, Action::PinFolder, Action::Properties, Action::Copy, Action::Cut, Action::Paste,
        Action::SelectAll, Action::CopyPath, Action::Refresh, Action::NewFolder, Action::NewWindow,
        Action::Settings, Action::ShowShortcuts, Action::Rename, Action::Trash, Action::DeleteForever,
        Action::NewFile, Action::GoHome, Action::Quit,
        Action::NewTab, Action::CloseTab, Action::NextTab, Action::PrevTab,
        Action::Undo, Action::Redo, Action::Duplicate, Action::MakeLink, Action::OpenTerminal, Action::OpenAsAdmin,
        Action::InvertSelection, Action::SelectPattern, Action::SelectNone, Action::ContextMenu,
        Action::ShowJobs,
    ];

    /// Stable id used as the key in settings.json.
    pub fn id(self) -> &'static str {
        match self {
            Action::MoveDown => "move_down", Action::MoveUp => "move_up", Action::Open => "open",
            Action::ParentFolder => "parent_folder", Action::Back => "back", Action::Forward => "forward",
            Action::First => "first", Action::Last => "last", Action::Filter => "filter",
            Action::EditPath => "edit_path", Action::ToggleHidden => "toggle_hidden", Action::ZoomIn => "zoom_in", Action::ZoomOut => "zoom_out",
            Action::ListView => "list_view", Action::GridView => "grid_view", Action::PinFolder => "pin_folder",
            Action::Properties => "properties", Action::Copy => "copy", Action::Cut => "cut",
            Action::Paste => "paste", Action::SelectAll => "select_all", Action::CopyPath => "copy_path",
            Action::Refresh => "refresh", Action::NewFolder => "new_folder", Action::NewWindow => "new_window",
            Action::Settings => "settings", Action::ShowShortcuts => "show_shortcuts",
            Action::Rename => "rename", Action::Trash => "trash", Action::DeleteForever => "delete_forever",
            Action::NewFile => "new_file", Action::GoHome => "go_home", Action::Quit => "quit",
            Action::NewTab => "new_tab", Action::CloseTab => "close_tab", Action::NextTab => "next_tab", Action::PrevTab => "prev_tab",
            Action::Undo => "undo", Action::Redo => "redo",
            Action::Duplicate => "duplicate", Action::MakeLink => "make_link", Action::OpenTerminal => "open_terminal", Action::OpenAsAdmin => "open_as_admin",
            Action::InvertSelection => "invert_selection", Action::SelectPattern => "select_pattern",
            Action::SelectNone => "select_none", Action::ContextMenu => "context_menu",
            Action::ShowJobs => "show_jobs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::MoveDown => tr("Move selection down"), Action::MoveUp => tr("Move selection up"),
            Action::Open => tr("Open"), Action::ParentFolder => tr("Parent folder"), Action::Back => tr("Back"),
            Action::Forward => tr("Forward"), Action::First => tr("Jump to first"), Action::Last => tr("Jump to last"),
            Action::Filter => tr("Filter by name"), Action::EditPath => tr("Edit path"),
            Action::ToggleHidden => tr("Toggle hidden files"), Action::ZoomIn => tr("Larger icons"), Action::ZoomOut => tr("Smaller icons"), Action::ListView => tr("List view"),
            Action::GridView => tr("Grid view"), Action::PinFolder => tr("Pin folder"), Action::Properties => tr("Properties"),
            Action::Copy => tr("Copy"), Action::Cut => tr("Cut"), Action::Paste => tr("Paste"),
            Action::SelectAll => tr("Select all"), Action::CopyPath => tr("Copy path"), Action::Refresh => tr("Refresh"),
            Action::NewFolder => tr("New folder"), Action::NewWindow => tr("New window"), Action::Settings => tr("Settings"),
            Action::ShowShortcuts => tr("Show shortcuts"), Action::Rename => tr("Rename"),
            Action::Trash => tr("Move to trash"), Action::DeleteForever => tr("Delete permanently"),
            Action::NewFile => tr("New file"), Action::GoHome => tr("Home folder"), Action::Quit => tr("Close window"),
            Action::NewTab => tr("New tab"), Action::CloseTab => tr("Close tab"), Action::NextTab => tr("Next tab"), Action::PrevTab => tr("Previous tab"),
            Action::Undo => tr("Undo"), Action::Redo => tr("Redo"),
            Action::Duplicate => tr("Duplicate"), Action::MakeLink => tr("Create link"), Action::OpenTerminal => tr("Open in terminal"), Action::OpenAsAdmin => tr("Open as administrator"),
            Action::InvertSelection => tr("Invert selection"), Action::SelectPattern => tr("Select matching…"),
            Action::SelectNone => tr("Deselect all"), Action::ContextMenu => tr("Open context menu"),
            Action::ShowJobs => tr("Show file operations"),
        }
    }

    /// Section the action is listed under in the editor.
    pub fn group(self) -> &'static str {
        match self {
            Action::MoveDown | Action::MoveUp | Action::Open | Action::ParentFolder | Action::Back
            | Action::Forward | Action::First | Action::Last | Action::EditPath | Action::GoHome
            | Action::NewTab | Action::CloseTab | Action::NextTab | Action::PrevTab => tr("Navigation"),
            Action::Filter | Action::ToggleHidden | Action::ZoomIn | Action::ZoomOut | Action::ListView | Action::GridView | Action::Refresh
            | Action::Settings | Action::ShowShortcuts | Action::Quit => tr("View"),
            _ => tr("Files"),
        }
    }

    pub fn default_bindings(self) -> &'static [&'static str] {
        match self {
            Action::MoveDown => &["ArrowDown", "j"], Action::MoveUp => &["ArrowUp", "k"],
            Action::Open => &["Enter", "l"], Action::ParentFolder => &["Backspace", "h", "Alt+ArrowUp"],
            Action::Back => &["Alt+ArrowLeft"], Action::Forward => &["Alt+ArrowRight"],
            Action::First => &["Home", "g"], Action::Last => &["End", "Shift+G"],
            Action::Filter => &["/", "Ctrl+F"], Action::EditPath => &["Ctrl+L"],
            Action::ToggleHidden => &["Ctrl+H"], Action::ZoomIn => &["Ctrl+="], Action::ZoomOut => &["Ctrl+-"], Action::ListView => &["Ctrl+1"], Action::GridView => &["Ctrl+2"],
            Action::PinFolder => &["Ctrl+D"], Action::Properties => &["Alt+Enter"],
            Action::Copy => &["Ctrl+C"], Action::Cut => &["Ctrl+X"], Action::Paste => &["Ctrl+V"],
            Action::SelectAll => &["Ctrl+A"], Action::CopyPath => &["y"], Action::Refresh => &["F5", "Ctrl+R"],
            Action::NewFolder => &["Ctrl+Shift+N"], Action::NewWindow => &["Ctrl+N"],
            Action::Settings => &["Ctrl+,"], Action::ShowShortcuts => &["?"],
            Action::Rename => &["F2"], Action::Trash => &["Delete"], Action::DeleteForever => &["Shift+Delete"],
            Action::NewFile => &["Ctrl+Alt+N"], Action::GoHome => &["Alt+Home"], Action::Quit => &["Ctrl+Q"],
            Action::NewTab => &["Ctrl+T"], Action::CloseTab => &["Ctrl+W"],
            Action::NextTab => &["Ctrl+Tab", "Ctrl+PageDown"], Action::PrevTab => &["Ctrl+Shift+Tab", "Ctrl+PageUp"],
            Action::Undo => &["Ctrl+Z"], Action::Redo => &["Ctrl+Shift+Z", "Ctrl+Y"],
            Action::Duplicate => &["Ctrl+Shift+D"], Action::MakeLink => &["Ctrl+Shift+M"], Action::OpenTerminal => &["F4"], Action::OpenAsAdmin => &[],
            Action::InvertSelection => &["Ctrl+Shift+I"], Action::SelectPattern => &["Ctrl+S"],
            Action::SelectNone => &["Ctrl+Shift+A"], Action::ContextMenu => &["Menu", "Shift+F10"],
            Action::ShowJobs => &["Ctrl+J"],
        }
    }
}

/// Effective bindings for `action`: the user's override, or the defaults.
pub fn bindings(overrides: &std::collections::BTreeMap<String, Vec<String>>, action: Action) -> Vec<String> {
    overrides
        .get(action.id())
        .cloned()
        .unwrap_or_else(|| action.default_bindings().iter().map(|s| s.to_string()).collect())
}

/// Format a key press as a binding string, e.g. `Ctrl+Shift+N`. `None` for bare modifier presses.
pub fn format_binding(key: egui::Key, mods: egui::Modifiers) -> String {
    let mut out = String::new();
    if mods.command { out.push_str("Ctrl+"); }
    if mods.alt { out.push_str("Alt+"); }
    if mods.shift { out.push_str("Shift+"); }
    out.push_str(key.name());
    out
}

/// A binding as it is shown to people: arrows instead of key names, symbols kept as typed.
pub fn display(binding: &str) -> String {
    binding
        .split('+')
        .map(|part| match part {
            "ArrowUp" => "↑",
            "ArrowDown" => "↓",
            "ArrowLeft" => "←",
            "ArrowRight" => "→",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The first binding of `action`, formatted for a menu hint.
pub fn hint(overrides: &std::collections::BTreeMap<String, Vec<String>>, action: Action, vim_keys: bool) -> Option<String> {
    bindings(overrides, action)
        .into_iter()
        .find(|b| vim_keys || !is_bare_letter(b))
        .map(|b| display(&b))
}

/// Parse a binding string into the key and the modifiers it requires.
pub fn parse_binding(s: &str) -> Option<(egui::Key, egui::Modifiers)> {
    let mut mods = egui::Modifiers::NONE;
    let mut key = None;
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" => mods.command = true,
            "alt" => mods.alt = true,
            "shift" => mods.shift = true,
            _ => key = egui::Key::from_name(part),
        }
    }
    key.map(|k| (k, mods))
}

/// True when `binding` was pressed this frame. Modifiers must match exactly (Shift is ignored
/// for symbol keys that already imply it, like `?`).
pub fn pressed(ctx: &egui::Context, binding: &str) -> bool {
    let Some((key, mods)) = parse_binding(binding) else { return false };
    ctx.input(|i| {
        i.key_pressed(key)
            && i.modifiers.command == mods.command
            && i.modifiers.alt == mods.alt
            && (i.modifiers.shift == mods.shift || matches!(key, egui::Key::Questionmark))
    })
}

/// Same as `pressed`, but allows Shift to be held even if not explicitly required by the binding.
pub fn pressed_allow_shift(ctx: &egui::Context, binding: &str) -> bool {
    let Some((key, mods)) = parse_binding(binding) else { return false };
    ctx.input(|i| {
        i.key_pressed(key)
            && i.modifiers.command == mods.command
            && i.modifiers.alt == mods.alt
            && (i.modifiers.shift == mods.shift || (i.modifiers.shift && !mods.shift) || matches!(key, egui::Key::Questionmark))
    })
}

/// Returns true if the binding consists of a single letter without Ctrl or Alt (e.g. `j`, `k`, `g`, `Shift+G`, `y`).
pub fn is_bare_letter(binding: &str) -> bool {
    let Some((key, mods)) = parse_binding(binding) else { return false };
    if !mods.command && !mods.alt {
        let name = key.name();
        name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic())
    } else {
        false
    }
}

/// True when any binding for `action` was pressed this frame, respecting `vim_keys`.
pub fn is_action_pressed(
    ctx: &egui::Context,
    overrides: &std::collections::BTreeMap<String, Vec<String>>,
    action: Action,
    vim_keys: bool,
) -> bool {
    let list = bindings(overrides, action);
    let allow_shift = matches!(action, Action::MoveDown | Action::MoveUp);
    list.iter().any(|b| {
        if !vim_keys && is_bare_letter(b) {
            return false;
        }
        if allow_shift && !is_bare_letter(b) {
            pressed_allow_shift(ctx, b)
        } else {
            pressed(ctx, b)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_bare_letter() {
        assert!(is_bare_letter("j"));
        assert!(is_bare_letter("k"));
        assert!(is_bare_letter("g"));
        assert!(is_bare_letter("Shift+G"));
        assert!(is_bare_letter("y"));
        assert!(!is_bare_letter("ArrowDown"));
        assert!(!is_bare_letter("Ctrl+C"));
        assert!(!is_bare_letter("Alt+Enter"));
        assert!(!is_bare_letter("Enter"));
        assert!(!is_bare_letter("Backspace"));
        assert!(!is_bare_letter("F5"));
        assert!(!is_bare_letter("?"));
        assert!(!is_bare_letter("/"));
    }

    #[test]
    fn test_bindings_default_and_override() {
        let mut overrides = std::collections::BTreeMap::new();
        assert_eq!(bindings(&overrides, Action::MoveDown), vec!["ArrowDown", "j"]);

        overrides.insert("move_down".to_string(), vec!["Down".to_string()]);
        assert_eq!(bindings(&overrides, Action::MoveDown), vec!["Down"]);
    }

    #[test]
    fn test_display_and_hint() {
        assert_eq!(display("Alt+ArrowLeft"), "Alt+←");
        assert_eq!(display("ArrowDown"), "↓");
        assert_eq!(display("Ctrl+Shift+N"), "Ctrl+Shift+N");
        let none = std::collections::BTreeMap::new();
        assert_eq!(hint(&none, Action::MoveDown, true).as_deref(), Some("↓"));
        // Without vim keys the bare-letter binding is skipped
        assert_eq!(hint(&none, Action::CopyPath, false), None);
        assert_eq!(hint(&none, Action::CopyPath, true).as_deref(), Some("y"));
        assert_eq!(hint(&none, Action::Rename, true).as_deref(), Some("F2"));
    }

    #[test]
    fn test_every_action_has_a_unique_binding_id() {
        let mut ids: Vec<&str> = Action::ALL.iter().map(|a| a.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Action::ALL.len());
        // No two actions share a default binding, or the first one would shadow the other
        let mut bindings: Vec<&str> = Action::ALL.iter().flat_map(|a| a.default_bindings().iter().copied()).collect();
        let total = bindings.len();
        bindings.sort_unstable();
        bindings.dedup();
        assert_eq!(bindings.len(), total, "duplicate default binding");
    }

    #[test]
    fn test_format_and_parse_binding() {
        let (k, m) = parse_binding("Ctrl+Shift+N").expect("parse Ctrl+Shift+N");
        assert_eq!(k, egui::Key::N);
        assert!(m.command);
        assert!(m.shift);
        assert!(!m.alt);
        assert_eq!(format_binding(k, m), "Ctrl+Shift+N");
    }
}
