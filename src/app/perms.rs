//! Changing permissions from the properties dialog.
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::fs::props::{mode_string, Properties};

/// What the permission card asks for after a frame.
#[derive(Default)]
pub(super) struct PermAction {
    pub new_mode: Option<u32>,
    pub apply_to_contents: bool,
}

/// Bits of the owner, group and others classes, for read, write and execute.
const CLASSES: [(&str, u32); 3] = [("Owner", 0o100), ("Group", 0o010), ("Others", 0o001)];
const KINDS: [(&str, u32); 3] = [("Read", 4), ("Write", 2), ("Execute", 1)];

/// The "Permissions" card: a grid of tick boxes, a switch for running the file as a program, and
/// for folders a button that hands the same permissions to everything inside.
pub(super) fn permission_card(ui: &mut egui::Ui, theme: &SerpantinumTheme, full: f32, mode: u32, is_dir: bool, can_edit: bool) -> PermAction {
    let mut action = PermAction::default();
    let mut mode = mode & 0o7777;
    material::section_label(ui, tr("Permissions"), theme);
    egui::Frame::new().fill(theme.surface1).corner_radius(egui::CornerRadius::same(material::RADIUS_CONTAINER)).show(ui, |ui| {
        ui.set_width(full);
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(4.0);
        if !is_dir
            && material::switch_row(ui, "perm_exec", icons::EXECUTABLE, tr("Allow running as a program"), theme, mode & 0o111 != 0).clicked()
            && can_edit
        {
            mode = if mode & 0o111 != 0 { mode & !0o111 } else { mode | ((mode & 0o444) >> 2) | if mode & 0o444 == 0 { 0o100 } else { 0 } };
            action.new_mode = Some(mode);
        }

        let label_w = 92.0;
        let col_w = ((full - label_w - 16.0) / 3.0).max(40.0);
        let (head, _) = ui.allocate_exact_size(egui::vec2(full, 28.0), egui::Sense::hover());
        for (i, (label, _)) in KINDS.iter().enumerate() {
            let label = match *label {
                "Read" => tr("Read"),
                "Write" => tr("Write"),
                _ => if is_dir { tr("Open") } else { tr("Execute") },
            };
            ui.painter().text(egui::pos2(head.left() + label_w + col_w * (i as f32 + 0.5), head.center().y), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(material::FONT_CAPTION), theme.subtext1);
        }
        for (name, shift) in CLASSES {
            let (row, _) = ui.allocate_exact_size(egui::vec2(full, 38.0), egui::Sense::hover());
            let label = match name {
                "Owner" => tr("Owner"),
                "Group" => tr("Group"),
                _ => tr("Others"),
            };
            ui.painter().text(egui::pos2(row.left() + 16.0, row.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(material::FONT_BODY), theme.text);
            for (i, (_, bit)) in KINDS.iter().enumerate() {
                let mask = bit * shift;
                let cell = egui::Rect::from_center_size(egui::pos2(row.left() + label_w + col_w * (i as f32 + 0.5), row.center().y), egui::vec2(30.0, 30.0));
                let resp = ui.put(cell, |ui: &mut egui::Ui| material::checkbox(ui, ("perm", shift, bit), mode & mask != 0, can_edit, theme));
                if resp.clicked() && can_edit {
                    mode ^= mask;
                    action.new_mode = Some(mode);
                }
            }
        }
        ui.add_space(4.0);
        let octal = format!("{:03o}  ·  {}", mode & 0o777, mode_string(mode, is_dir));
        let (note, _) = ui.allocate_exact_size(egui::vec2(full, 24.0), egui::Sense::hover());
        let text = if can_edit { octal } else { format!("{octal}  ·  {}", tr("Only the owner can change these")) };
        ui.painter().text(egui::pos2(note.left() + 16.0, note.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(material::FONT_CAPTION), theme.subtext1);
        if !(is_dir && can_edit) {
            ui.add_space(8.0);
        }
        if is_dir && can_edit {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                if material::button(ui, "perm_tree", Some(icons::FOLDER_COPY), tr("Apply to enclosed items"), theme, ButtonKind::Tonal, true).clicked() {
                    action.apply_to_contents = true;
                }
            });
            ui.add_space(8.0);
        }
    });
    action
}

/// Whether the person running Ski may change the permissions of this item.
pub(super) fn can_change(item: &crate::fs::file_item::FileItem) -> bool {
    // SAFETY: geteuid has no preconditions.
    let euid = unsafe { libc::geteuid() };
    !item.is_symlink && (euid == 0 || euid == item.uid)
}

impl SkiApp {
    /// Set the permission bits of one item and keep the way back.
    pub(super) fn set_mode(&mut self, props: &mut Properties, mode: u32) {
        let path = props.item.path.clone();
        let old = props.item.mode & 0o7777;
        match std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
            Ok(()) => {
                props.item.mode = (props.item.mode & !0o7777) | mode;
                props.item.file_type = if !props.item.is_dir && mode & 0o111 != 0 && props.item.file_type != crate::fs::file_item::FileType::Executable {
                    crate::fs::file_item::FileType::Executable
                } else {
                    props.item.file_type
                };
                self.history.record(UndoEntry::new(trf("Permissions of {}", &[&props.item.name]), vec![Edit::Chmod { path, mode: old }]));
                self.reload_items();
            }
            Err(err) => self.set_toast(trf("Could not change permissions: {}", &[&err])),
        }
    }

    /// Give everything inside a folder the permissions chosen for it: folders get the same bits,
    /// files the same read and write bits while keeping their own execute bits.
    pub(super) fn apply_mode_to_contents(&mut self, root: PathBuf, mode: u32) {
        let title = trf("Changing permissions in {}", &[&root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()]);
        self.engine.spawn(JobKind::Other, title, String::new(), move |w| {
            fn walk(w: &engine::Worker, dir: &Path, mode: u32, edits: &mut Vec<Edit>) -> std::io::Result<()> {
                for entry in std::fs::read_dir(dir)? {
                    w.shared.checkpoint()?;
                    let entry = entry?;
                    let path = entry.path();
                    let meta = path.symlink_metadata()?;
                    if meta.file_type().is_symlink() {
                        continue;
                    }
                    let old = meta.permissions().mode() & 0o7777;
                    let new = if meta.is_dir() { mode } else { (mode & 0o666) | (old & 0o111) };
                    if new != old {
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(new))?;
                        edits.push(Edit::Chmod { path: path.clone(), mode: old });
                    }
                    if meta.is_dir() {
                        walk(w, &path, mode, edits)?;
                    }
                }
                Ok(())
            }
            let mut edits = Vec::new();
            let result = walk(w, &root, mode, &mut edits);
            let effect = if edits.is_empty() { Effect::None } else { Effect::Record(UndoEntry::new(trn(edits.len(), "Permissions of {} item", "Permissions of {} items"), edits.clone())) };
            match result {
                Ok(()) => Outcome::done(trn(edits.len(), "Changed permissions of {} item", "Changed permissions of {} items")).with(effect),
                Err(err) => Outcome::failed(err.to_string()).with(effect),
            }
        });
    }
}
