//! Icon glyphs from the Nerd Fonts "Symbols Nerd Font Mono" (Material Design set, `md-*`).
//!
//! This is the same font Serpantinum pins for its icons. Icons are only ever drawn with the
//! dedicated `icons` font family, so no glyph can fall back to another font and every glyph
//! shares the same monospaced metrics.
#![allow(dead_code)]

use eframe::egui::{FontFamily, FontId};

pub const FAMILY: &str = "icons";

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FAMILY.into()))
}

pub const FOLDER: &str = "\u{f024b}";
pub const FILE: &str = "\u{f0224}";
pub const DOCUMENT: &str = "\u{f09ee}";
pub const CODE: &str = "\u{f0174}";
pub const IMAGE: &str = "\u{f0976}";
pub const AUDIO: &str = "\u{f0f74}";
pub const VIDEO: &str = "\u{f0bdc}";
pub const ARCHIVE: &str = "\u{f07b9}";
pub const EXECUTABLE: &str = "\u{f018d}";
pub const SYMLINK: &str = "\u{f0339}";

pub const HOME: &str = "\u{f06a1}";
pub const DESKTOP: &str = "\u{f07c0}";
pub const DOWNLOAD: &str = "\u{f0b8f}";
pub const DOCUMENTS: &str = "\u{f0c83}";
pub const STORAGE: &str = "\u{f02ca}";

pub const ARROW_BACK: &str = "\u{f004d}";
pub const ARROW_FORWARD: &str = "\u{f0054}";
pub const ARROW_UP: &str = "\u{f005d}";
pub const SEARCH: &str = "\u{f0349}";
pub const SETTINGS: &str = "\u{f08bb}";
pub const CLOSE: &str = "\u{f0156}";
pub const CHECK: &str = "\u{f012c}";
pub const CHEVRON_RIGHT: &str = "\u{f0142}";
pub const MORE: &str = "\u{f01d8}";

pub const VIEW_LIST: &str = "\u{f0572}";
pub const GRID_VIEW: &str = "\u{f0570}";
pub const VISIBILITY: &str = "\u{f06d0}";
pub const VISIBILITY_OFF: &str = "\u{f06d1}";
pub const SORT_ASC: &str = "\u{f04bc}";
pub const SORT_DESC: &str = "\u{f04bd}";

pub const PIN: &str = "\u{f0403}";
pub const PIN_OFF: &str = "\u{f0406}";
pub const FOLDER_PLUS: &str = "\u{f0257}";
pub const FILE_PLUS: &str = "\u{f0752}";
pub const INFO: &str = "\u{f02fd}";
pub const COPY: &str = "\u{f018f}";
pub const OPEN: &str = "\u{f03cc}";
pub const REFRESH: &str = "\u{f0450}";
pub const APPS: &str = "\u{f003b}";
pub const CUT: &str = "\u{f0190}";
pub const PASTE: &str = "\u{f0192}";
pub const FOLDER_MOVE: &str = "\u{f0252}";
pub const FOLDER_COPY: &str = "\u{f0253}";
pub const SELECT_ALL: &str = "\u{f0486}";
pub const COMPRESS: &str = "\u{f06eb}";
pub const EXTRACT: &str = "\u{f1259}";
