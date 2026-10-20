//! Drives in the sidebar: keep the list fresh, mount, unmount and eject without blocking the window.
use super::*;
use crate::fs::drives;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(4);

pub(super) enum DriveMsg {
    Listed(Vec<Place>),
    Mounted { dev: String, name: String, open: bool, result: Result<PathBuf, String> },
    Gone { dev: String, name: String, eject: bool, mount: PathBuf, result: Result<(), String> },
}

pub(super) struct DriveState {
    tx: mpsc::Sender<DriveMsg>,
    rx: mpsc::Receiver<DriveMsg>,
    last: Instant,
    listing: bool,
    /// Devices a mount or eject is working on.
    pub(super) busy: HashSet<String>,
}

impl DriveState {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, last: Instant::now(), listing: false, busy: HashSet::new() }
    }
}

impl SkiApp {
    /// Apply finished work and ask for a new listing every few seconds.
    pub(super) fn poll_drives(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.drive_state.rx.try_recv() {
            match msg {
                DriveMsg::Listed(found) => {
                    self.drive_state.listing = false;
                    self.set_drive_places(found);
                }
                DriveMsg::Mounted { dev, name, open, result } => {
                    self.drive_state.busy.remove(&dev);
                    match result {
                        Ok(path) => {
                            if open {
                                self.close_search();
                                self.navigate_to(path.canonicalize().unwrap_or(path));
                            }
                            self.request_drive_listing(ctx, true);
                        }
                        Err(err) => self.set_toast(trf("Could not mount “{}”: {}", &[&name, &err])),
                    }
                }
                DriveMsg::Gone { dev, name, eject, mount, result } => {
                    self.drive_state.busy.remove(&dev);
                    match result {
                        Ok(()) => {
                            if self.current_path.starts_with(&mount) {
                                self.navigate_to(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
                            }
                            self.set_toast(if eject { trf("“{}” can be removed safely", &[&name]) } else { trf("Unmounted “{}”", &[&name]) });
                            self.request_drive_listing(ctx, true);
                        }
                        Err(err) => self.set_toast(trf("Could not unmount “{}”: {}", &[&name, &err])),
                    }
                }
            }
        }
        self.request_drive_listing(ctx, false);
        ctx.request_repaint_after(REFRESH);
    }

    fn request_drive_listing(&mut self, ctx: &egui::Context, now: bool) {
        let state = &mut self.drive_state;
        if state.listing || !now && state.last.elapsed() < REFRESH {
            return;
        }
        state.listing = true;
        state.last = Instant::now();
        let (tx, ctx) = (state.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(DriveMsg::Listed(drives::list()));
            ctx.request_repaint();
        });
    }

    fn set_drive_places(&mut self, mut found: Vec<Place>) {
        for place in &mut found {
            if place.drive.as_ref().is_some_and(|d| d.mounted) {
                place.path = place.path.canonicalize().unwrap_or_else(|_| place.path.clone());
            }
        }
        let same = |a: &Place, b: &Place| a.name == b.name && a.path == b.path && a.drive == b.drive;
        let old: Vec<&Place> = self.places.iter().filter(|p| p.drive.is_some()).collect();
        if old.len() == found.len() && old.iter().zip(&found).all(|(a, b)| same(a, b)) {
            return;
        }
        self.places.retain(|p| p.drive.is_none());
        self.places.extend(found);
    }

    /// Mount a device and, when `open`, show it once it is there.
    pub(super) fn mount_drive(&mut self, ctx: &egui::Context, dev: String, name: String, open: bool) {
        if !self.drive_state.busy.insert(dev.clone()) {
            return;
        }
        let (tx, ctx) = (self.drive_state.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = drives::mount(&dev);
            let _ = tx.send(DriveMsg::Mounted { dev, name, open, result });
            ctx.request_repaint();
        });
    }

    /// Unmount a device; `eject` also powers a removable drive off.
    pub(super) fn unmount_drive(&mut self, ctx: &egui::Context, dev: String, name: String, mount: PathBuf, eject: bool) {
        if !self.drive_state.busy.insert(dev.clone()) {
            return;
        }
        let (tx, ctx) = (self.drive_state.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = if eject { drives::eject(&dev) } else { drives::unmount(&dev) };
            let _ = tx.send(DriveMsg::Gone { dev, name, eject, mount, result });
            ctx.request_repaint();
        });
    }
}

/// Small round eject button at the right end of a sidebar row; true when clicked.
pub(super) fn row_eject_button(ui: &mut egui::Ui, row: egui::Rect, row_hovered: bool, id: impl std::hash::Hash + std::fmt::Debug, theme: &SerpantinumTheme) -> bool {
    let size = 28.0;
    let rect = egui::Rect::from_center_size(egui::pos2(row.right() - 8.0 - size / 2.0, row.center().y), egui::vec2(size, size));
    let resp = ui.interact(rect, ui.make_persistent_id(id), egui::Sense::click());
    let show = material::animate(ui.ctx(), resp.id.with("show"), row_hovered || resp.hovered());
    if show > 0.0 {
        let hover = material::animate(ui.ctx(), resp.id.with("hov"), resp.hovered());
        if hover > 0.0 {
            ui.painter().circle_filled(rect.center(), size / 2.0, theme.with_alpha(theme.text, (hover * material::ALPHA_HOVER as f32) as u8));
        }
        let color = material::lerp_color(theme.subtext0, theme.text, hover);
        material::paint_icon(ui.painter(), rect.center(), icons::EJECT, material::ICON_SM, color.gamma_multiply(show));
    }
    resp.on_hover_text(tr("Eject")).clicked()
}
