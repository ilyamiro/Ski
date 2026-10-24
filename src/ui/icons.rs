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
pub const CHEVRON_LEFT: &str = "\u{f0141}";
pub const CHEVRON_RIGHT: &str = "\u{f0142}";
pub const CHEVRON_DOWN: &str = "\u{f0140}";
pub const ADD: &str = "\u{f0415}";
pub const REMOVE: &str = "\u{f0374}";
pub const MORE: &str = "\u{f01d8}";

pub const VIEW_LIST: &str = "\u{f0572}";
pub const GRID_VIEW: &str = "\u{f0570}";
pub const SORT_VARIANT: &str = "\u{f04bf}";
pub const ROUNDED_CORNER: &str = "\u{f0607}";
pub const OPACITY: &str = "\u{f05cc}";
pub const SIDEBAR: &str = "\u{f06fd}";
pub const STATUS_BAR: &str = "\u{f06fb}";
pub const ANIMATION: &str = "\u{f093a}";
pub const RUN: &str = "\u{f070e}";
pub const RULER: &str = "\u{f046d}";
pub const CURSOR: &str = "\u{f01c0}";
pub const VIEW_COLUMN: &str = "\u{f056d}";
pub const RESIZE: &str = "\u{f0a68}";
pub const VISIBILITY: &str = "\u{f06d0}";
pub const VISIBILITY_OFF: &str = "\u{f06d1}";
pub const SORT_ASC: &str = "\u{f04bc}";
pub const SORT_DESC: &str = "\u{f04bd}";

pub const HEART: &str = "\u{f02d1}";
pub const HEART_OUTLINE: &str = "\u{f02d5}";
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

pub const RECENT: &str = "\u{f02da}";
pub const TRASH: &str = "\u{f09e7}";
pub const DELETE_FOREVER: &str = "\u{f05e8}";
pub const RESTORE: &str = "\u{f099b}";
pub const RENAME: &str = "\u{f03eb}";
pub const USB: &str = "\u{f0553}";
pub const UNDO: &str = "\u{f054d}";
pub const SEARCH_OFF: &str = "\u{f0968}";

pub const MENU: &str = "\u{f035c}";
pub const TUNE: &str = "\u{f062e}";
pub const KEYBOARD: &str = "\u{f030c}";
pub const NEW_WINDOW: &str = "\u{f05b2}";
pub const TRANSLATE: &str = "\u{f05ca}";
pub const COG: &str = "\u{f0493}";

// Tabs, links, drives, bookmarks, selection and transfers
pub const TAB: &str = "\u{f04e9}";
pub const NEW_TAB: &str = "\u{f075c}";
pub const TERMINAL: &str = "\u{f07b7}";
pub const LINK: &str = "\u{f0337}";
pub const LOCK: &str = "\u{f033e}";
pub const LOCK_OPEN: &str = "\u{f033f}";
pub const EJECT: &str = "\u{f01ea}";
pub const UNMOUNT: &str = "\u{f104c}";
pub const REDO: &str = "\u{f044e}";
pub const BOOKMARK: &str = "\u{f00c0}";
pub const BOOKMARK_PLUS: &str = "\u{f00c5}";
pub const BOOKMARK_OUTLINE: &str = "\u{f00c3}";
pub const DUPLICATE: &str = "\u{f0191}";
pub const CANCEL: &str = "\u{f073a}";
pub const PAUSE: &str = "\u{f03e4}";
pub const PLAY: &str = "\u{f040a}";
pub const SELECT_INVERSE: &str = "\u{f0487}";
pub const SELECT_OFF: &str = "\u{f0488}";
pub const SELECT_PATTERN: &str = "\u{f0451}";
pub const FILTER: &str = "\u{f0236}";
pub const ALERT: &str = "\u{f0026}";
pub const FOLDER_OPEN: &str = "\u{f0770}";
pub const SHIELD: &str = "\u{f099d}";
pub const CASE_SENSITIVE: &str = "\u{f0b34}";
pub const TEXT_SEARCH: &str = "\u{f13b8}";
pub const DRAG: &str = "\u{f0b90}";
pub const TEMPLATE: &str = "\u{f1a9e}";
pub const SYNC: &str = "\u{f04e6}";
pub const HISTORY: &str = "\u{f02da}";

// File types
pub const PDF: &str = "\u{f0226}";
pub const HTML: &str = "\u{f031d}";
pub const DISC: &str = "\u{f05ee}";
pub const WORD: &str = "\u{f022d}";
pub const EXCEL: &str = "\u{f021c}";
pub const POWERPOINT: &str = "\u{f0228}";
pub const MARKDOWN: &str = "\u{f0354}";
pub const PYTHON: &str = "\u{f0320}";
pub const RUST: &str = "\u{f1617}";
pub const JAVASCRIPT: &str = "\u{f031e}";
pub const TYPESCRIPT: &str = "\u{f06e6}";
pub const CSS: &str = "\u{f031c}";
pub const JSON: &str = "\u{f0626}";
pub const LANG_C: &str = "\u{f0671}";
pub const LANG_CPP: &str = "\u{f0672}";
pub const GO: &str = "\u{f07d3}";
pub const JAVA: &str = "\u{f0b37}";
pub const LUA: &str = "\u{f08b1}";
pub const SHELL: &str = "\u{f1183}";
pub const NIX: &str = "\u{f1105}";
pub const PHP: &str = "\u{f031f}";
pub const RUBY: &str = "\u{f0d2d}";
pub const KOTLIN: &str = "\u{f1219}";
pub const SWIFT: &str = "\u{f06e5}";
pub const GIT: &str = "\u{f02a2}";
pub const VECTOR: &str = "\u{f0721}";
pub const FONT: &str = "\u{f06d6}";
pub const EBOOK: &str = "\u{f14f7}";
pub const KEY: &str = "\u{f1184}";
pub const CERTIFICATE: &str = "\u{f1186}";
pub const CONFIG: &str = "\u{f107b}";
pub const TABLE: &str = "\u{f0c7e}";
pub const DATABASE: &str = "\u{f01bc}";
pub const LOG: &str = "\u{f0bc2}";
pub const PACKAGE: &str = "\u{f03d6}";
pub const APPLICATION: &str = "\u{f08c6}";
pub const TORRENT: &str = "\u{f1464}";
pub const DOCKER: &str = "\u{f0868}";
pub const ZIP: &str = "\u{f05c4}";
pub const TEXT: &str = "\u{f0219}";
pub const CAD: &str = "\u{f0eeb}";

pub const TAG: &str = "\u{f04f9}";
pub const SOURCE_BRANCH: &str = "\u{f062c}";
