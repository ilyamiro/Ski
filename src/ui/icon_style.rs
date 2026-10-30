//! Icon styles: how folders, file-type icons and interface glyphs are drawn. A style never
//! changes the palette (every shape is tinted with the color it is given); it changes the shapes
//! and the way glyphs are rendered. Previews of pictures are not icons and keep their look.
use std::f32::consts::PI;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Shape, Stroke, Vec2};
use serde::{Deserialize, Serialize};

use super::icons;
use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum IconStyle {
    /// Flat folders and plain glyphs; office documents are drawn pages.
    #[default]
    Classic,
    /// Jack-o'-lantern folders with a stem, cobwebs and a spider; files on tombstones; links as ghosts.
    Halloween,
    /// Folders wrapped as presents under snow; files capped with snow.
    Winter,
    /// Chunky 8-bit pixel art, glyphs included.
    Pixel,
    /// Wobbly pencil outlines with hatching, files on dog-eared pages. The one style left hollow.
    Sketch,
    /// Glassy gradients with a shine across the top.
    Glossy,
    /// Soft, chunky shapes with depth, as if modelled in clay.
    Clay,
    /// Folded paper: flat facets and creases.
    Origami,
}

impl IconStyle {
    pub const ALL: [IconStyle; 8] = [
        IconStyle::Classic,
        IconStyle::Halloween,
        IconStyle::Winter,
        IconStyle::Pixel,
        IconStyle::Sketch,
        IconStyle::Glossy,
        IconStyle::Clay,
        IconStyle::Origami,
    ];

    pub fn label(self) -> &'static str {
        match self {
            IconStyle::Classic => tr("Classic"),
            IconStyle::Halloween => tr("Halloween"),
            IconStyle::Winter => tr("Winter"),
            IconStyle::Pixel => tr("Pixel"),
            IconStyle::Sketch => tr("Sketch"),
            IconStyle::Glossy => tr("Glossy"),
            IconStyle::Clay => tr("Clay"),
            IconStyle::Origami => tr("Origami"),
        }
    }
}

// A style that no longer exists falls back to the default instead of failing the whole settings file
impl<'de> Deserialize<'de> for IconStyle {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Self::ALL.into_iter().find(|s| format!("{s:?}") == name).unwrap_or_default())
    }
}

static STYLE: AtomicU8 = AtomicU8::new(0);

pub fn set(style: IconStyle) {
    STYLE.store(style as u8, Ordering::Relaxed);
}

pub fn current() -> IconStyle {
    IconStyle::ALL.get(STYLE.load(Ordering::Relaxed) as usize).copied().unwrap_or_default()
}

/// A glyph of the interface (buttons, menus, the sidebar). Only Pixel restyles the glyph itself;
/// the shapes of the other styles belong to file icons.
pub fn paint_glyph(style: IconStyle, painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    if style == IconStyle::Pixel {
        let block = (size * painter.ctx().pixels_per_point() / 16.0).round().max(1.0);
        if paint_pixel_glyph(painter, center, glyph, size, block, color, false) {
            return;
        }
    }
    plain(painter, center, glyph, size, color);
}

/// The type icon of a file, from its glyph.
pub fn paint_file(style: IconStyle, painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    match style {
        IconStyle::Classic => plain(painter, center, glyph, size, color),
        IconStyle::Halloween if glyph == icons::SYMLINK => ghost(painter, center, size, color),
        IconStyle::Halloween => tombstone(painter, center, glyph, size, color),
        IconStyle::Winter => winter_file(painter, center, glyph, size, color),
        IconStyle::Pixel => {
            // About 24 cells across, enough for a logo, in blocks of whole screen pixels: one on small
            // icons, where a single pixel is all the detail there is, and from two up once a block of
            // two still leaves the logo its cells
            let px = size * painter.ctx().pixels_per_point();
            let block = if px < 28.0 { 1.0 } else { (px / 24.0).round().max(2.0) };
            if !paint_pixel_glyph(painter, center, glyph, size, block, color, true) {
                plain(painter, center, glyph, size, color);
            }
        }
        IconStyle::Sketch => sketch_page(painter, center, glyph, size, color),
        IconStyle::Glossy => glossy_file(painter, center, glyph, size, color),
        IconStyle::Clay => clay_file(painter, center, glyph, size, color),
        IconStyle::Origami => origami_file(painter, center, glyph, size, color),
    }
}

pub fn paint_folder(style: IconStyle, painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    match style {
        IconStyle::Classic => classic_folder(painter, center, size, color),
        IconStyle::Halloween => halloween_folder(painter, center, size, color),
        IconStyle::Winter => winter_folder(painter, center, size, color),
        IconStyle::Pixel => pixel_folder(painter, center, size, color),
        IconStyle::Sketch => sketch_folder(painter, center, size, color),
        IconStyle::Glossy => glossy_folder(painter, center, size, color),
        IconStyle::Clay => clay_folder(painter, center, size, color),
        IconStyle::Origami => origami_folder(painter, center, size, color),
    }
}

fn plain(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    painter.text(center, Align2::CENTER_CENTER, glyph, icons::font(size), color);
}

// ---- Shared drawing helpers ----

// Colors are premultiplied, so shading moves the color channels and leaves alpha alone; a faded
// icon stays faded however it is shaded.
fn darken(c: Color32, t: f32) -> Color32 {
    let k = 1.0 - t;
    Color32::from_rgba_premultiplied((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8, c.a())
}

fn lighten(c: Color32, t: f32) -> Color32 {
    let a = c.a() as f32;
    let up = |v: u8| (v as f32 + (a - v as f32).max(0.0) * t) as u8;
    Color32::from_rgba_premultiplied(up(c.r()), up(c.g()), up(c.b()), c.a())
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// White at `alpha`, faded along with `c`.
fn white(c: Color32, alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha * c.a() as f32) as u8)
}

/// A glyph color that reads on a shape filled with `c`: dark on light tints, pale on dark ones.
fn ink(c: Color32) -> Color32 {
    let luma = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if luma > c.a() as f32 * 0.55 { darken(c, 0.72) } else { lighten(c, 0.85) }
}

fn radius(r: f32) -> u8 {
    r.round().clamp(0.0, 255.0) as u8
}

fn poly(painter: &egui::Painter, pts: Vec<Pos2>, fill: Color32) {
    painter.add(Shape::convex_polygon(pts, fill, Stroke::NONE));
}

/// A convex shape shaded top to bottom.
fn gradient(painter: &egui::Painter, pts: &[Pos2], top: Color32, bottom: Color32) {
    let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let mut mesh = egui::Mesh::default();
    for p in pts {
        mesh.colored_vertex(*p, mix(top, bottom, (p.y - y0) / (y1 - y0).max(0.001)));
    }
    for i in 1..pts.len().saturating_sub(1) as u32 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(Shape::mesh(mesh));
}

/// A soft round glow: `fill` in the middle, fading out to nothing at `r`.
fn glow(painter: &egui::Painter, at: Pos2, r: f32, fill: Color32) {
    let n = 32;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(at, fill);
    for ring in [(0.5, fill.gamma_multiply(0.55)), (1.0, Color32::TRANSPARENT)] {
        for i in 0..n {
            mesh.colored_vertex(at + Vec2::angled(i as f32 / n as f32 * 2.0 * PI) * r * ring.0, ring.1);
        }
    }
    for i in 0..n {
        let (a, b) = (1 + i, 1 + (i + 1) % n);
        mesh.add_triangle(0, a, b);
        mesh.add_triangle(a, b, b + n);
        mesh.add_triangle(a, b + n, a + n);
    }
    painter.add(Shape::mesh(mesh));
}

/// Corners rounded off with true circular arcs, `r` back from each corner along both edges, so a
/// rounded rectangle matches the one egui draws for the same radius.
fn rounded(pts: &[Pos2], r: f32) -> Vec<Pos2> {
    rounded_each(pts, &vec![r; pts.len()])
}

/// Like `rounded`, with a radius for each corner; 0 keeps a corner sharp.
fn rounded_each(pts: &[Pos2], radii: &[f32]) -> Vec<Pos2> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n * 9);
    for i in 0..n {
        let (prev, p, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        if radii[i] <= 0.0 {
            out.push(p);
            continue;
        }
        let rr = radii[i].min((prev - p).length() / 2.0).min((next - p).length() / 2.0);
        let (da, db) = ((prev - p).normalized(), (next - p).normalized());
        let (a, b) = (p + da * rr, p + db * rr);
        // A rational quadratic weighted by the sine of half the corner's angle is exactly a circle
        let w = (da.dot(db).clamp(-1.0, 1.0).acos() / 2.0).sin();
        for j in 0..=8 {
            let t = j as f32 / 8.0;
            let (ka, kp, kb) = ((1.0 - t) * (1.0 - t), 2.0 * w * t * (1.0 - t), t * t);
            out.push(((a.to_vec2() * ka + p.to_vec2() * kp + b.to_vec2() * kb) / (ka + kp + kb)).to_pos2());
        }
    }
    out
}

fn corners(rect: Rect) -> [Pos2; 4] {
    [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]
}

/// Points scaled toward their middle, for an inset panel of the same shape.
fn inset(pts: &[Pos2], k: f32) -> Vec<Pos2> {
    let mid = pts.iter().fold(Vec2::ZERO, |s, p| s + p.to_vec2()) / pts.len() as f32;
    pts.iter().map(|p| (mid + (p.to_vec2() - mid) * k).to_pos2()).collect()
}

fn seed_of(s: &str) -> u32 {
    s.bytes().fold(0x811C_9DC5, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// A steady pseudo-random value in -1..1, so details look the same every frame.
fn noise(seed: u32, i: u32) -> f32 {
    let mut x = seed ^ i.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32 * 2.0 - 1.0
}

/// The box every folder style shares, so a style switch keeps the layout: the body, the corner
/// radius, where the front panel starts and where the tab ends.
struct FolderBox {
    body: Rect,
    r: f32,
    front_top: f32,
    tab_right: f32,
}

fn folder_box(center: Pos2, size: f32) -> FolderBox {
    let w = size * 1.05;
    let body = Rect::from_center_size(center, Vec2::new(w, w * 0.8));
    let r = w * 0.1;
    FolderBox { body, r, front_top: body.top() + r, tab_right: body.left() + w * 0.35 }
}

impl FolderBox {
    fn front(&self) -> Rect {
        Rect::from_min_max(Pos2::new(self.body.left(), self.front_top), self.body.max)
    }

    /// The tab as a convex shape: up the left side, along the top, sloping down to the front.
    fn tab(&self) -> Vec<Pos2> {
        let b = self.body;
        vec![
            Pos2::new(b.left(), self.front_top + self.r),
            Pos2::new(b.left(), b.top()),
            Pos2::new(self.tab_right, b.top()),
            Pos2::new(self.tab_right + self.r, self.front_top),
            Pos2::new(self.tab_right + self.r, self.front_top + self.r),
        ]
    }

    /// The outline of tab and front together, corners unrounded.
    fn silhouette(&self) -> Vec<Pos2> {
        let b = self.body;
        vec![
            Pos2::new(b.left(), b.bottom()),
            Pos2::new(b.left(), b.top()),
            Pos2::new(self.tab_right, b.top()),
            Pos2::new(self.tab_right + self.r, self.front_top),
            Pos2::new(b.right(), self.front_top),
            Pos2::new(b.right(), b.bottom()),
        ]
    }
}

/// A page with its top right corner folded over: the page, the fold size and its outline.
fn page(center: Pos2, size: f32) -> (Rect, f32, Vec<Pos2>) {
    let rect = Rect::from_center_size(center, Vec2::new(size * 0.74, size * 0.94));
    let fold = size * 0.24;
    let outline = vec![
        rect.left_top(),
        Pos2::new(rect.right() - fold, rect.top()),
        Pos2::new(rect.right(), rect.top() + fold),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    (rect, fold, outline)
}

fn fold_triangle(rect: Rect, fold: f32) -> Vec<Pos2> {
    vec![Pos2::new(rect.right() - fold, rect.top()), Pos2::new(rect.right(), rect.top() + fold), Pos2::new(rect.right() - fold, rect.top() + fold)]
}

/// How big the glyph on a page is: more of the page on small icons, so it stays legible in a list.
fn page_glyph(size: f32) -> f32 {
    size * if size < 28.0 { 0.62 } else { 0.5 }
}

// ---- Classic ----

/// A folder drawn in two flat layers: a slightly darker back panel that only shows as the tab (rounded on its
/// outer corner, sloping down on the right), and the front panel over it.
fn classic_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let f = folder_box(center, size);
    let (body, r) = (f.body, f.r);
    let mut tab = vec![Pos2::new(body.left(), f.front_top + r)];
    for i in 0..=8 {
        let a = PI * (1.0 + 0.5 * i as f32 / 8.0);
        tab.push(Pos2::new(body.left() + r + r * a.cos(), body.top() + r + r * a.sin()));
    }
    tab.push(Pos2::new(f.tab_right, body.top()));
    tab.push(Pos2::new(f.tab_right + r, f.front_top));
    tab.push(Pos2::new(f.tab_right + r, f.front_top + r));
    poly(painter, tab, darken(color, 0.2));
    painter.rect_filled(f.front(), CornerRadius::same(radius(r)), color);
}

// ---- Halloween ----

/// A jack-o'-lantern folder: a stem with a curling vine where the tab ends, ribs running from top to
/// bottom, and a carved face lit from inside.
fn halloween_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let f = folder_box(center, size);
    let (body, r) = (f.body, f.r);
    let carve = darken(color, 0.72);
    let candle = lighten(color, 0.65);
    // The front sits a little lower than the classic one, to leave the stem room above it
    let front_top = body.top() + r * 1.6;
    let tab_top = body.top() + r * 0.5;
    let tab = [
        Pos2::new(body.left(), front_top + r),
        Pos2::new(body.left(), tab_top),
        Pos2::new(f.tab_right, tab_top),
        Pos2::new(f.tab_right + r, front_top),
        Pos2::new(f.tab_right + r, front_top + r),
    ];
    poly(painter, rounded(&tab, r * 0.6), darken(color, 0.3));

    // The stem, leaning a little and narrowing to a cut top, and a vine curling off it
    if size >= 20.0 {
        let stem_x = body.left() + body.width() * 0.6;
        let sw = body.width() * 0.08;
        let stem_top = body.top() - r * 0.1;
        let stem = [
            Pos2::new(stem_x - sw * 0.5, front_top + r * 0.5),
            Pos2::new(stem_x - sw * 0.2, stem_top + r * 0.15),
            Pos2::new(stem_x + sw * 0.55, stem_top),
            Pos2::new(stem_x + sw * 0.5, front_top + r * 0.5),
        ];
        poly(painter, rounded(&stem, sw * 0.15), darken(color, 0.5));
        if size >= 32.0 {
            let hub = Pos2::new(stem_x + sw * 0.5 + r * 0.55, front_top - r * 0.75);
            let curl: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0;
                    hub + Vec2::angled(PI + t * 1.6 * PI) * r * 0.55 * (1.0 - t * 0.55)
                })
                .collect();
            painter.add(Shape::line(curl, Stroke::new((size / 45.0).max(1.0), darken(color, 0.45))));
        }
    }

    let front = Rect::from_min_max(Pos2::new(body.left(), front_top), body.max);
    painter.rect_filled(front, CornerRadius::same(radius(r * 1.4)), color);
    let (fw, fh) = (front.width(), front.height());
    // Ribs from the top edge to the bottom one, bowing out with the pumpkin, clear of the face
    if size >= 22.0 {
        let rib = Stroke::new((size / 36.0).max(1.0), darken(color, 0.2));
        for (k, bow) in [(0.2, -1.0), (0.8, 1.0)] {
            let pts = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    Pos2::new(front.left() + fw * k + bow * fw * 0.05 * (PI * t).sin(), front.top() + fh * t)
                })
                .collect();
            painter.add(Shape::line(pts, rib));
        }
    }

    // Carved face; on bigger icons a candle glows inside each cut
    let lit = size >= 26.0;
    let eye_y = front.top() + fh * 0.34;
    let (ew, eh) = (fw * 0.1, fh * 0.24);
    for k in [0.33, 0.67] {
        let x = front.left() + fw * k;
        let eye = vec![Pos2::new(x - ew, eye_y + eh / 2.0), Pos2::new(x, eye_y - eh / 2.0), Pos2::new(x + ew, eye_y + eh / 2.0)];
        if lit {
            let glow = inset(&eye, 0.5).into_iter().map(|p| p + Vec2::new(0.0, eh * 0.15)).collect();
            poly(painter, eye, carve);
            poly(painter, glow, candle);
        } else {
            poly(painter, eye, carve);
        }
    }
    if size >= 24.0 {
        let (x, y) = (front.center().x, front.top() + fh * 0.53);
        poly(painter, vec![Pos2::new(x - ew * 0.5, y + eh * 0.2), Pos2::new(x, y - eh * 0.2), Pos2::new(x + ew * 0.5, y + eh * 0.2)], carve);
    }
    if size >= 18.0 {
        let mouth = Rect::from_min_max(
            Pos2::new(front.left() + fw * 0.24, front.top() + fh * 0.62),
            Pos2::new(front.right() - fw * 0.24, front.top() + fh * 0.82),
        );
        let round = radius(mouth.height() * 0.9);
        painter.rect_filled(mouth, CornerRadius { nw: 1, ne: 1, sw: round, se: round }, carve);
        if lit {
            let glow = Rect::from_min_max(Pos2::new(mouth.left() + mouth.width() * 0.12, mouth.center().y), Pos2::new(mouth.right() - mouth.width() * 0.12, mouth.bottom() - mouth.height() * 0.18));
            let round = radius(glow.height());
            painter.rect_filled(glow, CornerRadius { nw: 0, ne: 0, sw: round, se: round }, candle);
        }
        // Teeth: one down from the top lip on each side, one up from the bottom in the middle
        let tooth = mouth.width() * 0.13;
        for k in [0.3, 0.7] {
            let x = mouth.left() + mouth.width() * k;
            poly(painter, vec![Pos2::new(x - tooth, mouth.top() - 0.5), Pos2::new(x + tooth, mouth.top() - 0.5), Pos2::new(x, mouth.center().y + mouth.height() * 0.15)], color);
        }
        let x = mouth.center().x;
        poly(painter, vec![Pos2::new(x - tooth, mouth.bottom() + 0.5), Pos2::new(x, mouth.center().y - mouth.height() * 0.05), Pos2::new(x + tooth, mouth.bottom() + 0.5)], color);
    }
}

/// A bat in flight: scalloped wings around a small body with ears.
fn bat(painter: &egui::Painter, at: Pos2, span: f32, fill: Color32) {
    let s = span;
    for side in [-1.0f32, 1.0] {
        let p = |x: f32, y: f32| at + Vec2::new(side * x * s, y * s);
        let root = p(0.06, -0.04);
        let edge = [p(0.5, -0.22), p(0.44, 0.06), p(0.32, 0.0), p(0.22, 0.09), p(0.1, 0.04)];
        for w in edge.windows(2) {
            poly(painter, vec![root, w[0], w[1]], fill);
        }
        poly(painter, vec![p(0.02, -0.08), p(0.07, -0.2), p(0.09, -0.06)], fill);
    }
    painter.add(Shape::ellipse_filled(at, Vec2::new(s * 0.08, s * 0.11), fill));
}

/// A file as a tombstone with its glyph carved in, on a grassy mound. Each kind of file gets its own
/// shape of stone (arched, cross-topped or pointed), so a folder of them looks like a graveyard.
fn tombstone(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    let seed = seed_of(glyph);
    let stone_c = darken(color, 0.6);
    let face_c = darken(color, 0.48);
    let earth = darken(color, 0.72);
    let grass = darken(color, 0.3);
    let mut stone = Rect::from_min_max(center + Vec2::new(-0.36, -0.4) * size, center + Vec2::new(0.36, 0.4) * size);
    let face_top;
    // How far the carved face reaches either side of the middle at a height, to fit the lettering in
    let face_half: Box<dyn Fn(f32) -> f32>;
    match seed % 3 {
        0 => {
            let top = radius(stone.width() / 2.0);
            painter.rect_filled(stone, CornerRadius { nw: top, ne: top, sw: 1, se: 1 }, stone_c);
            let face = stone.shrink(size * 0.05);
            let top = radius(face.width() / 2.0);
            painter.rect_filled(face, CornerRadius { nw: top, ne: top, sw: 1, se: 1 }, face_c);
            face_top = face.top() + size * 0.1;
            let (r, arch_y) = (face.width() / 2.0, face.top() + face.width() / 2.0);
            face_half = Box::new(move |y| (r * r - (arch_y - y).max(0.0).powi(2)).max(0.0).sqrt());
        }
        1 => {
            stone.min.y += size * 0.14;
            let cross = stone_c;
            let cx = stone.center().x;
            painter.rect_filled(Rect::from_min_max(Pos2::new(cx - size * 0.06, stone.top() - size * 0.2), Pos2::new(cx + size * 0.06, stone.top() + 1.0)), CornerRadius::same(1), cross);
            painter.rect_filled(Rect::from_center_size(Pos2::new(cx, stone.top() - size * 0.12), Vec2::new(size * 0.34, size * 0.1)), CornerRadius::same(1), cross);
            painter.rect_filled(stone, CornerRadius::same(radius(size * 0.06)), stone_c);
            painter.rect_filled(stone.shrink(size * 0.05), CornerRadius::same(radius(size * 0.03)), face_c);
            face_top = stone.top() + size * 0.05;
            let half = stone.width() / 2.0 - size * 0.05;
            face_half = Box::new(move |_| half);
        }
        _ => {
            let outline = vec![
                stone.left_bottom(),
                Pos2::new(stone.left(), stone.top() + size * 0.18),
                Pos2::new(stone.center().x, stone.top() - size * 0.04),
                Pos2::new(stone.right(), stone.top() + size * 0.18),
                stone.right_bottom(),
            ];
            let face = inset(&outline, 0.84);
            // Under the roof the face widens from nothing at its peak to full width at its shoulders
            let (peak, shoulder) = (face[2], face[3]);
            face_half = Box::new(move |y| (shoulder.x - peak.x) * ((y - peak.y) / (shoulder.y - peak.y)).clamp(0.0, 1.0));
            poly(painter, outline, stone_c);
            poly(painter, face, face_c);
            // The lettering sits down where the face has widened under the roof
            face_top = stone.top() + size * 0.17;
        }
    }
    if size >= 36.0 {
        let p = |x: f32, y: f32| stone.min + Vec2::new(stone.width() * x, stone.height() * y);
        let crack = Stroke::new((size / 48.0).max(1.0), darken(color, 0.75));
        painter.add(Shape::line(vec![p(0.86, 0.22), p(0.79, 0.31), p(0.84, 0.38), p(0.78, 0.48)], crack));
    }

    // Stones over 48 points read R.I.P. above the carved glyph
    let rip = size >= 48.0;
    if rip {
        // Shrunk to fit wherever it is carved, whatever the interface font: inside the face where the
        // top of the letters is, and clear of the crack
        let at = Pos2::new(stone.center().x, face_top + size * 0.04);
        let ink = color.gamma_multiply(0.85);
        let font = size * 0.11;
        let cap_top = at.y - font * 0.4;
        let crack_half = stone.width() * 0.28 - size * 0.03;
        let room = 2.0 * (face_half(cap_top) - size * 0.04).min(crack_half).max(0.0);
        let width = painter.layout_no_wrap("R.I.P.".into(), FontId::proportional(font), ink).size().x;
        let font = font * (room / width).min(1.0);
        painter.text(at, Align2::CENTER_CENTER, "R.I.P.", FontId::proportional(font), ink);
    }
    let (glyph_size, glyph_at) = if rip {
        (size * 0.42, center + Vec2::new(0.0, size * 0.1))
    } else {
        (size * if size < 28.0 { 0.62 } else { 0.52 }, center + Vec2::new(0.0, size * 0.04 + (face_top - stone.top() - size * 0.05).max(0.0) * 0.3))
    };
    plain(painter, glyph_at, glyph, glyph_size, color);

    // The stone stands in a low grassy mound that covers its foot, with tufts growing on its slopes
    let (base, reach, rise) = (stone.bottom() + size * 0.04, size * 0.44, size * 0.09);
    let surface = |dx: f32| base - rise * (1.0 - (dx / reach).powi(2)).max(0.0).sqrt();
    let mound: Vec<Pos2> = (0..=24).map(|i| {
        let dx = reach * (i as f32 / 12.0 - 1.0);
        Pos2::new(center.x + dx, surface(dx))
    }).collect();
    poly(painter, mound, earth);
    if size >= 24.0 {
        for (i, dx) in [-0.32f32, -0.2, 0.22, 0.33].into_iter().enumerate() {
            let x = center.x + dx * size;
            let root = surface(dx * size) + size * 0.015;
            for blade in [-1.0, 0.0, 1.0] {
                let lean = (blade * 0.025 + noise(seed, i as u32) * 0.015) * size;
                let bx = x + blade * size * 0.018;
                let tall = size * (0.07 + 0.015 * (1.0 - blade * blade));
                poly(painter, vec![Pos2::new(bx - size * 0.013, root), Pos2::new(bx + lean, root - tall), Pos2::new(bx + size * 0.013, root)], grass);
            }
        }
    }
    // Bats roost on about half of the stones
    if size >= 56.0 && seed % 2 == 0 {
        bat(painter, Pos2::new(stone.right() - size * 0.02, stone.top() - size * 0.02), size * 0.26, darken(color, 0.62));
    }
}

/// A link is a ghost of the file it points to.
fn ghost(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let rw = size * 0.3;
    let head = center + Vec2::new(0.0, -size * 0.12);
    let hem = center.y + size * 0.3;
    glow(painter, head + Vec2::new(0.0, rw * 0.5), rw * 1.75, color.gamma_multiply(0.22));
    let sheet = color;
    for side in [-1.0, 1.0] {
        painter.add(Shape::ellipse_filled(head + Vec2::new(side * rw * 1.05, rw * 0.75), Vec2::new(rw * 0.32, rw * 0.2), sheet));
    }
    painter.circle_filled(head, rw, sheet);
    painter.rect_filled(Rect::from_min_max(Pos2::new(head.x - rw, head.y), Pos2::new(head.x + rw, hem)), CornerRadius::ZERO, sheet);
    let wave = rw / 3.0;
    for k in [-2.0, 0.0, 2.0] {
        painter.circle_filled(Pos2::new(head.x + k * wave, hem), wave, sheet);
    }
    let eye = darken(color, 0.75);
    for k in [-1.0, 1.0] {
        let at = head + Vec2::new(k * rw * 0.38, rw * 0.05);
        painter.add(Shape::ellipse_filled(at, Vec2::new(rw * 0.14, rw * 0.22), eye));
        if size >= 32.0 {
            painter.circle_filled(at + Vec2::new(-rw * 0.04, -rw * 0.08), rw * 0.05, lighten(color, 0.9));
        }
    }
    painter.add(Shape::ellipse_filled(head + Vec2::new(0.0, rw * 0.5), Vec2::new(rw * 0.1, rw * 0.15), eye));
}

// ---- Winter ----

/// Snow lying on an edge at `top`, from `left` to `right`: a band rounded like the corners it covers
/// (`round`), soft drifts on it and drops hanging below, over a cool shadow that gives it volume and
/// keeps it apart from a pale background.
#[allow(clippy::too_many_arguments)]
fn snow_cap(painter: &egui::Painter, left: f32, right: f32, top: f32, depth: f32, round: f32, color: Color32, seed: u32) {
    let y = top + depth * 0.45;
    let snow = lighten(color, 0.92);
    let shade = mix(lighten(color, 0.5), snow, 0.4);
    let lumps = ((right - left) / (depth * 2.4)).round().max(2.0) as u32;
    let step = (right - left) / lumps as f32;
    // A few drifts of uneven height, overlapping so they read as one blanket, and kept within the
    // ends so the snow does not hang out past what it lies on
    let drifts: Vec<(Pos2, Vec2)> = (0..lumps)
        .map(|i| {
            let rr = step * (0.58 + 0.08 * noise(seed, i));
            let x = (left + step * (i as f32 + 0.5) + step * 0.12 * noise(seed, i + 20)).clamp(left + rr, right - rr);
            (Pos2::new(x, y - depth * 0.3), Vec2::new(rr, depth * (0.68 + 0.14 * noise(seed, i + 40))))
        })
        .collect();
    let drops: Vec<Pos2> = (0..lumps.saturating_sub(1))
        .filter(|i| noise(seed, i + 50) > 0.0)
        .map(|i| Pos2::new(left + step * (i as f32 + 1.0) + step * 0.2 * noise(seed, i + 60), y + depth * 0.45))
        .collect();
    let band = Rect::from_min_max(Pos2::new(left, y - depth * 0.45), Pos2::new(right, y + depth * 0.35));
    for (fill, dy) in [(shade, depth * 0.2), (snow, 0.0)] {
        let dy = Vec2::new(0.0, dy);
        painter.rect_filled(band.translate(dy), CornerRadius::same(radius(round)), fill);
        for d in &drops {
            painter.add(Shape::ellipse_filled(*d + dy, Vec2::new(depth * 0.26, depth * 0.4), fill));
        }
        for (at, r) in &drifts {
            painter.add(Shape::ellipse_filled(*at + dy, *r, fill));
        }
    }
}

fn snowflake(painter: &egui::Painter, at: Pos2, r: f32, color: Color32) {
    let stroke = Stroke::new((r * 0.22).max(0.8), lighten(color, 0.9));
    for i in 0..3 {
        let d = Vec2::angled(i as f32 * PI / 3.0 + PI / 2.0) * r;
        painter.line_segment([at - d, at + d], stroke);
    }
}

/// A folder wrapped as a present: a ribbon across the front, a bow on the snow on top.
fn winter_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let f = folder_box(center, size);
    let (body, r) = (f.body, f.r);
    let seed = seed_of("winter");
    poly(painter, rounded(&f.tab(), r * 0.6), darken(color, 0.22));
    let front = f.front();
    painter.rect_filled(front, CornerRadius::same(radius(r)), color);
    let ribbon = darken(color, 0.38);
    let band = front.width() * 0.13;
    let rx = front.left() + front.width() * 0.66;
    painter.rect_filled(Rect::from_min_max(Pos2::new(rx - band / 2.0, front.top()), Pos2::new(rx + band / 2.0, front.bottom())), CornerRadius::ZERO, ribbon);
    let ry = front.top() + front.height() * 0.58;
    painter.rect_filled(Rect::from_min_max(Pos2::new(front.left(), ry - band / 2.0), Pos2::new(front.right(), ry + band / 2.0)), CornerRadius::ZERO, ribbon);
    snow_cap(painter, front.left(), front.right(), front.top(), r * 0.9, r, color, seed);
    snow_cap(painter, body.left(), f.tab_right, body.top(), r * 0.5, r * 0.6, color, seed ^ 7);
    // The bow, sitting on the snow above the ribbon
    let bow = Pos2::new(rx, front.top() - r * 0.1);
    let wing = band * 1.6;
    for side in [-1.0, 1.0] {
        if size >= 28.0 {
            // Tails hanging down over the snow
            poly(painter, vec![bow, bow + Vec2::new(side * wing * 0.25, wing * 0.9), bow + Vec2::new(side * wing * 0.6, wing * 0.75)], darken(color, 0.48));
        }
        // A bow tie: a triangle out to each side, fattened by a rounded end
        let tip_top = bow + Vec2::new(side * wing, -wing * 0.55);
        let tip_bottom = bow + Vec2::new(side * wing, wing * 0.4);
        poly(painter, vec![bow, tip_top, tip_bottom], ribbon);
        painter.add(Shape::ellipse_filled(tip_top.lerp(tip_bottom, 0.5), Vec2::new(wing * 0.16, wing * 0.48), ribbon));
    }
    painter.circle_filled(bow, wing * 0.24, darken(color, 0.55));
    if size >= 40.0 {
        snowflake(painter, Pos2::new(front.left() + front.width() * 0.2, front.top() + front.height() * 0.32), size * 0.05, color);
        snowflake(painter, Pos2::new(front.left() + front.width() * 0.4, front.top() + front.height() * 0.8), size * 0.035, color);
        snowflake(painter, Pos2::new(front.right() - front.width() * 0.12, front.top() + front.height() * 0.84), size * 0.04, color);
    }
}

/// A page with snow piled on top.
fn winter_file(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    let (rect, fold, outline) = page(center, size);
    poly(painter, outline, color);
    poly(painter, fold_triangle(rect, fold), darken(color, 0.25));
    snow_cap(painter, rect.left(), rect.right() - fold, rect.top() - size * 0.015, size * 0.1, size * 0.04, color, seed_of(glyph));
    plain(painter, center + Vec2::new(0.0, size * 0.07), glyph, page_glyph(size), ink(color));
}

// ---- Pixel ----

fn nerd_font() -> Option<&'static ab_glyph::FontRef<'static>> {
    static FONT: OnceLock<Option<ab_glyph::FontRef<'static>>> = OnceLock::new();
    FONT.get_or_init(|| ab_glyph::FontRef::try_from_slice(icons::FONT_DATA).ok()).as_ref()
}

/// Every glyph of the Material Design set in the icon font, for choosing one.
pub fn all_glyphs() -> &'static [char] {
    use ab_glyph::Font;
    static ALL: OnceLock<Vec<char>> = OnceLock::new();
    ALL.get_or_init(|| {
        let Some(font) = nerd_font() else { return Vec::new() };
        let mut all: Vec<char> = font.codepoint_ids().map(|(_, c)| c).filter(|c| ('\u{f0001}'..='\u{f1af0}').contains(c)).collect();
        all.sort_unstable();
        all
    })
}

/// How much of each cell of a `side`×`side` canvas the glyph covers, drawn `scale` times the canvas
/// (the font's glyphs fill their em) with its middle `shift` cells off the canvas's middle.
fn glyph_coverage(font: &ab_glyph::FontRef<'static>, id: ab_glyph::GlyphId, side: usize, scale: f32, shift: Vec2) -> Option<Vec<f32>> {
    use ab_glyph::{Font, ScaleFont};
    let px = side as f32 * scale;
    let bounds = font.outline(id)?.bounds;
    let scaled = font.as_scaled(px);
    let (sx, sy) = (scaled.h_scale_factor(), scaled.v_scale_factor());
    let middle = Vec2::splat(side as f32 / 2.0) + shift;
    let at = ab_glyph::point(middle.x - (bounds.min.x + bounds.max.x) / 2.0 * sx, middle.y + (bounds.min.y + bounds.max.y) / 2.0 * sy);
    let outline = font.outline_glyph(id.with_scale_and_position(px, at))?;
    let min = outline.px_bounds().min;
    let mut cover = vec![0.0; side * side];
    outline.draw(|x, y, c| {
        let (x, y) = (x as i32 + min.x as i32, y as i32 + min.y as i32);
        if (0..side as i32).contains(&x) && (0..side as i32).contains(&y) {
            cover[y as usize * side + x as usize] = c;
        }
    });
    Some(cover)
}

/// A connected run of ink or of blank space, as `regions` finds it.
#[derive(Clone, Copy)]
struct Region {
    inked: bool,
    /// Blank space reaching the edge is outside the shape; any other blank space is a hole in it.
    border: bool,
    area: usize,
}

/// The shape split into its parts and the holes in them, the things that make a logo what it is: a
/// region index for every pixel, and the regions. Ink joins on diagonals, the way a sprite reads;
/// blank space only edge to edge, so a diagonal of ink still closes a hole.
fn regions(ink: &[bool], side: usize) -> (Vec<u32>, Vec<Region>) {
    // One pass joining each pixel to those above and to its left, then one numbering the joined sets
    fn root(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            parent[i as usize] = parent[parent[i as usize] as usize];
            i = parent[i as usize];
        }
        i
    }
    let mut parent: Vec<u32> = (0..(side * side) as u32).collect();
    for y in 0..side {
        for x in 0..side {
            let i = y * side + x;
            let inked = ink[i];
            let mut join = |j: usize| {
                if ink[j] == inked {
                    let (a, b) = (root(&mut parent, i as u32), root(&mut parent, j as u32));
                    parent[a.max(b) as usize] = a.min(b);
                }
            };
            if x > 0 {
                join(i - 1);
            }
            if y > 0 {
                join(i - side);
                if inked && x > 0 {
                    join(i - side - 1);
                }
                if inked && x + 1 < side {
                    join(i - side + 1);
                }
            }
        }
    }
    let mut label = vec![u32::MAX; side * side];
    let mut found: Vec<Region> = Vec::new();
    for i in 0..side * side {
        let r = root(&mut parent, i as u32) as usize;
        if label[r] == u32::MAX {
            label[r] = found.len() as u32;
            found.push(Region { inked: ink[i], border: false, area: 0 });
        }
        let id = label[r];
        label[i] = id;
        let (x, y) = (i % side, i / side);
        let region = &mut found[id as usize];
        region.area += 1;
        region.border |= x == 0 || y == 0 || x == side - 1 || y == side - 1;
    }
    (label, found)
}

/// The finely drawn glyph's regions as a sprite can show them: parts with no more than a quarter of a
/// cell between them are one part, the hairline between them too thin to keep, while holes are taken
/// as drawn.
fn shape_regions(ink: &[bool], side: usize) -> (Vec<u32>, Vec<Region>) {
    // Ink spread a quarter of a cell right and down closes exactly the gaps that narrow
    const NEAR: usize = FINE / 4;
    let near: Vec<bool> = (0..side * side)
        .map(|i| {
            let (x, y) = (i % side, i / side);
            (0..=NEAR.min(y)).any(|dy| (0..=NEAR.min(x)).any(|dx| ink[(y - dy) * side + x - dx]))
        })
        .collect();
    let (parts, part_regions) = regions(&near, side);
    let (blanks, blank_regions) = regions(ink, side);
    let offset = part_regions.len() as u32;
    let mut found: Vec<Region> = part_regions.into_iter().chain(blank_regions).map(|r| Region { area: 0, ..r }).collect();
    let label: Vec<u32> = (0..side * side).map(|i| if ink[i] { parts[i] } else { offset + blanks[i] }).collect();
    for &l in &label {
        found[l as usize].area += 1;
    }
    (label, found)
}

/// Each finely drawn pixel of a cell is this many of the cell.
const FINE: usize = 8;

/// The glyph drawn into `grid`×`grid` cells by its regions rather than its coverage. A cell goes to
/// the part, hole or outside that has most of it; where two parts would then touch, or a hole would
/// run into another or out of the shape, the cell held more weakly goes the other way, so thin gaps
/// survive; and a part or hole of half a cell or more that no cell went to is given its strongest
/// cell; and a part that came out in pieces is joined up again. `fine` is the glyph drawn `FINE` times
/// finer than the cells, and `label` its regions as `shape_regions` finds them. Returns the cells, how
/// much of each the glyph covers, and how badly the cells keep its features.
fn sprite_cells(fine: &[bool], label: &[u32], found: &[Region], grid: usize) -> (Vec<bool>, Vec<f32>, f32) {
    let side = grid * FINE;
    // Parts smaller than a quarter of a cell and holes smaller than half of one are only edges,
    // specks and slivers; they count as plain ink or blank space
    let minor = |r: u32| {
        let region = found[r as usize];
        region.area * if region.inked { 4 } else { 2 } < FINE * FINE
    };
    const INK: u32 = u32::MAX - 1;
    const BLANK: u32 = u32::MAX;
    let n = grid * grid;
    let mut owner = vec![BLANK; n];
    let mut strength = vec![0.0f32; n];
    let mut cover = vec![0.0f32; n];
    let mut tallies: Vec<Vec<(u32, usize)>> = vec![Vec::new(); n];
    for (i, &l) in label.iter().enumerate() {
        let cell = (i / side / FINE) * grid + (i % side) / FINE;
        let key = if minor(l) { if fine[i] { INK } else { BLANK } } else { l };
        let tally = &mut tallies[cell];
        match tally.iter_mut().find(|(k, _)| *k == key) {
            Some(t) => t.1 += 1,
            None => tally.push((key, 1)),
        }
        if fine[i] {
            cover[cell] += 1.0 / (FINE * FINE) as f32;
        }
    }
    let inked = |k: u32| k == INK || (k != BLANK && found[k as usize].inked);
    for (cell, tally) in tallies.iter().enumerate() {
        if let Some(&(k, c)) = tally.iter().max_by_key(|(_, c)| *c) {
            owner[cell] = k;
            strength[cell] = c as f32 / (FINE * FINE) as f32;
        }
    }

    // Regions that must not meet: two parts touching even at a corner, two blank regions sharing an edge
    for _ in 0..3 {
        let mut changed = false;
        for cell in 0..n {
            let (x, y) = ((cell % grid) as i32, (cell / grid) as i32);
            for (dx, dy) in [(1, 0), (0, 1), (1, 1), (-1, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if !(0..grid as i32).contains(&nx) || !(0..grid as i32).contains(&ny) {
                    continue;
                }
                let other = ny as usize * grid + nx as usize;
                let (a, b) = (owner[cell], owner[other]);
                let distinct = a != b && a < INK && b < INK && inked(a) == inked(b);
                if !distinct || (!inked(a) && dx != 0 && dy != 0) {
                    continue;
                }
                let weak = if strength[cell] <= strength[other] { cell } else { other };
                owner[weak] = if inked(owner[weak]) { BLANK } else { INK };
                strength[weak] = 0.0;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // A feature of half a cell or more that lost every cell gets back the one it fills most
    for (r, region) in found.iter().enumerate() {
        let r = r as u32;
        if region.area * 2 < FINE * FINE || (!region.inked && region.border) || owner.contains(&r) {
            continue;
        }
        let best = (0..n).filter_map(|cell| tallies[cell].iter().find(|(k, _)| *k == r).map(|(_, c)| (cell, *c))).max_by_key(|(_, c)| *c);
        if let Some((cell, _)) = best {
            let held = owner[cell];
            // Unless that would take the last cell of another feature
            if held >= INK || owner.iter().filter(|&&o| o == held).count() > 1 {
                owner[cell] = r;
            }
        }
    }

    // A stroke thinner than a cell holds no cell by a majority and would vanish. Every cell a part
    // reaches into is offered to it, clear of other parts and of holes, and the offer is thinned to a
    // line a cell wide, leaving the cells the part holds outright as they are
    for (r, region) in found.iter().enumerate() {
        let r = r as u32;
        if !region.inked || region.area * 2 < FINE * FINE {
            continue;
        }
        let near_other = |cell: usize| {
            let (x, y) = ((cell % grid) as i32, (cell / grid) as i32);
            (-1..=1).any(|dy| {
                (-1..=1).any(|dx| {
                    let (nx, ny) = (x + dx, y + dy);
                    (0..grid as i32).contains(&nx) && (0..grid as i32).contains(&ny) && {
                        let o = owner[ny as usize * grid + nx as usize];
                        o < INK && o != r && found[o as usize].inked
                    }
                })
            })
        };
        let free = |o: u32| o >= INK || (!found[o as usize].inked && found[o as usize].border);
        let held: Vec<bool> = owner.iter().map(|&o| o == r).collect();
        let offered: Vec<bool> = (0..n)
            .map(|cell| {
                held[cell] || (free(owner[cell]) && tallies[cell].iter().any(|&(k, c)| k == r && c * 5 >= FINE * FINE) && !near_other(cell))
            })
            .collect();
        // Strongest first, and none that would close off a hole that is not in the glyph
        let share = |cell: usize| tallies[cell].iter().find(|(k, _)| *k == r).map_or(0, |(_, c)| *c);
        let mut taken: Vec<usize> = thin(offered, &held, grid).into_iter().enumerate().filter(|&(cell, on)| on && !held[cell]).map(|(cell, _)| cell).collect();
        taken.sort_by_key(|&cell| std::cmp::Reverse(share(cell)));
        for cell in taken {
            let before = owner[cell];
            let blanks = blank_count(&owner, grid, inked);
            owner[cell] = r;
            if blank_count(&owner, grid, inked) > blanks {
                owner[cell] = before;
            } else {
                strength[cell] = 0.0;
            }
        }
    }

    // A stroke about a cell wide falls half in one cell and half in the next, and either way breaks up.
    // A part that came out in pieces is joined up again along the cells it runs through, where they
    // are cheapest to take and keep clear of every other part
    for (r, region) in found.iter().enumerate() {
        let r = r as u32;
        if !region.inked || region.area < FINE * FINE {
            continue;
        }
        let share = |cell: usize| tallies[cell].iter().find(|(k, _)| *k == r).map_or(0.0, |(_, c)| *c as f32 / (FINE * FINE) as f32);
        let neighbours = |cell: usize| {
            let (x, y) = ((cell % grid) as i32, (cell / grid) as i32);
            [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)].into_iter().filter_map(move |(dx, dy)| {
                let (nx, ny) = (x + dx, y + dy);
                ((0..grid as i32).contains(&nx) && (0..grid as i32).contains(&ny)).then_some(ny as usize * grid + nx as usize)
            })
        };
        let other_part = |o: u32| o < INK && o != r && found[o as usize].inked;
        for _ in 0..grid {
            let mine: Vec<bool> = owner.iter().map(|&o| o == r).collect();
            let (piece, pieces) = regions(&mine, grid);
            let mut sizes = vec![0; pieces.len()];
            for (cell, &p) in piece.iter().enumerate() {
                if mine[cell] {
                    sizes[p as usize] += 1;
                }
            }
            let Some(main) = (0..pieces.len()).filter(|&p| pieces[p].inked).max_by_key(|&p| sizes[p]) else { break };
            if pieces.iter().filter(|p| p.inked).count() < 2 {
                break;
            }
            // Cheapest path out of the biggest piece to any other, through cells the part touches
            let mut cost = vec![u32::MAX; n];
            let mut from = vec![usize::MAX; n];
            let mut queue = std::collections::BinaryHeap::new();
            for cell in 0..n {
                if mine[cell] && piece[cell] as usize == main {
                    cost[cell] = 0;
                    queue.push(std::cmp::Reverse((0, cell)));
                }
            }
            let mut reached = None;
            while let Some(std::cmp::Reverse((c, cell))) = queue.pop() {
                if c > cost[cell] {
                    continue;
                }
                if mine[cell] && piece[cell] as usize != main {
                    reached = Some(cell);
                    break;
                }
                for next in neighbours(cell) {
                    let open = mine[next] || (share(next) > 0.0 && !other_part(owner[next]) && !neighbours(next).any(|m| other_part(owner[m])));
                    if !open {
                        continue;
                    }
                    let step = if mine[next] { 0 } else { ((1.0 - share(next)) * 100.0) as u32 + 1 };
                    if c + step < cost[next] {
                        cost[next] = c + step;
                        from[next] = cell;
                        queue.push(std::cmp::Reverse((c + step, next)));
                    }
                }
            }
            let Some(mut cell) = reached else { break };
            let (undo, blanks) = (owner.clone(), blank_count(&owner, grid, inked));
            while cost[cell] > 0 {
                owner[cell] = r;
                cell = from[cell];
            }
            // Joining a stroke must not close off a hole the glyph does not have
            if blank_count(&owner, grid, inked) > blanks {
                owner = undo;
                break;
            }
        }
    }

    // What the sprite gets wrong, feature by feature: a part or hole of the glyph that is gone, or
    // broken in pieces, or two run together (any hole into the outside)
    let cells: Vec<bool> = owner.iter().map(|&o| inked(o)).collect();
    let (piece, pieces) = regions(&cells, grid);
    const OUTSIDE: u32 = u32::MAX - 2;
    let key = |o: u32| match o {
        INK | BLANK => None,
        o if !found[o as usize].inked && found[o as usize].border => Some(OUTSIDE),
        o => Some(o),
    };
    let weight = |o: u32| if o == OUTSIDE || found[o as usize].area >= FINE * FINE { 1.0 } else { 0.4 };
    let mut held: Vec<Vec<u32>> = vec![Vec::new(); pieces.len()];
    let mut spread_over: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for cell in 0..n {
        if let Some(k) = key(owner[cell]) {
            let p = piece[cell];
            if !held[p as usize].contains(&k) {
                held[p as usize].push(k);
            }
            let over = spread_over.entry(k).or_default();
            if !over.contains(&p) {
                over.push(p);
            }
        }
    }
    let mut faults = 0.0;
    for keys in &held {
        if keys.len() > 1 {
            faults += keys.iter().map(|&k| weight(k)).sum::<f32>() - keys.iter().map(|&k| weight(k)).fold(0.0, f32::max);
        }
    }
    for (r, region) in found.iter().enumerate() {
        let r = r as u32;
        if region.area * 2 < FINE * FINE || key(r) != Some(r) {
            continue;
        }
        faults += weight(r) * spread_over.get(&r).map_or(1.0, |over| over.len() as f32 - 1.0);
    }
    (cells, cover, faults)
}

/// How many separate runs of blank space the cells, inked as `inked` says of their owners, leave.
fn blank_count(owner: &[u32], side: usize, inked: impl Fn(u32) -> bool) -> usize {
    let cells: Vec<bool> = owner.iter().map(|&o| inked(o)).collect();
    regions(&cells, side).1.iter().filter(|r| !r.inked).count()
}

/// `on` thinned to lines a cell wide, keeping its connections and the cells `keep` marks (Zhang and
/// Suen's thinning); the loose ends the thinning leaves, one cell long, are cut off.
fn thin(mut on: Vec<bool>, keep: &[bool], side: usize) -> Vec<bool> {
    let at = |on: &[bool], x: i32, y: i32| (0..side as i32).contains(&x) && (0..side as i32).contains(&y) && on[y as usize * side + x as usize];
    // The eight neighbours clockwise from above
    let ring = |on: &[bool], cell: usize| {
        let (x, y) = ((cell % side) as i32, (cell / side) as i32);
        [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)].map(|(dx, dy)| at(on, x + dx, y + dy))
    };
    loop {
        let mut changed = false;
        for pass in 0..2 {
            let gone: Vec<usize> = (0..side * side)
                .filter(|&cell| on[cell] && !keep[cell])
                .filter(|&cell| {
                    let p = ring(&on, cell);
                    let count = p.iter().filter(|&&b| b).count();
                    let turns = (0..8).filter(|&i| !p[i] && p[(i + 1) % 8]).count();
                    let (n, e, s, w) = (p[0], p[2], p[4], p[6]);
                    let clear = if pass == 0 { !(n && e && s) && !(e && s && w) } else { !(n && e && w) && !(n && s && w) };
                    (2..=6).contains(&count) && turns == 1 && clear
                })
                .collect();
            changed |= !gone.is_empty();
            for cell in gone {
                on[cell] = false;
            }
        }
        if !changed {
            break;
        }
    }
    let spurs: Vec<usize> = (0..side * side).filter(|&cell| on[cell] && !keep[cell] && ring(&on, cell).iter().filter(|&&b| b).count() == 1).collect();
    for cell in spurs {
        on[cell] = false;
    }
    on
}

/// The glyph as a `grid`×`grid` sprite: true where it is inked. A glyph lands on so few cells that
/// where its strokes and holes fall across them decides whether they survive, so it is tried at every
/// eighth of a cell and a few slightly smaller sizes, and the fit kept that keeps the parts and holes
/// of the finely drawn glyph, follows its outline most closely and leaves no stray specks.
fn pixel_cells(glyph: &str, grid: usize) -> Option<Vec<bool>> {
    use ab_glyph::Font;
    let font = nerd_font()?;
    let id = font.glyph_id(glyph.chars().next()?);
    if id.0 == 0 {
        return None;
    }
    // The glyph drawn finely once at each size, on a canvas a cell bigger than the sprite, so each
    // eighth of a cell it can move under the cells is a whole fine pixel
    let wide = (grid + 1) * FINE;
    let mut fits = Vec::new();
    let mut canvases = Vec::new();
    for (k, scale) in [1.0, 0.96, 0.92, 0.88].into_iter().enumerate() {
        let canvas: Vec<bool> = glyph_coverage(font, id, wide, scale * grid as f32 / (grid + 1) as f32, Vec2::ZERO)?.into_iter().map(|c| c >= 0.5).collect();
        // Ink summed over every rectangle from the corner, so any cell's coverage is four lookups
        let mut sum = vec![0u32; (wide + 1) * (wide + 1)];
        for y in 0..wide {
            for x in 0..wide {
                sum[(y + 1) * (wide + 1) + x + 1] = canvas[y * wide + x] as u32 + sum[y * (wide + 1) + x + 1] + sum[(y + 1) * (wide + 1) + x] - sum[y * (wide + 1) + x];
            }
        }
        let ink = |x0: usize, y0: usize, x1: usize, y1: usize| sum[y1 * (wide + 1) + x1] + sum[y0 * (wide + 1) + x0] - sum[y0 * (wide + 1) + x1] - sum[y1 * (wide + 1) + x0];
        let total = ink(0, 0, wide, wide);
        for oy in 0..=FINE {
            for ox in 0..=FINE {
                // Only where the whole glyph is on the sprite
                if ink(ox, oy, ox + grid * FINE, oy + grid * FINE) < total {
                    continue;
                }
                // Sharp fits first: the fewest cells left half covered
                let blur: f32 = (0..grid * grid)
                    .map(|cell| {
                        let (x, y) = (ox + cell % grid * FINE, oy + cell / grid * FINE);
                        let c = ink(x, y, x + FINE, y + FINE) as f32 / (FINE * FINE) as f32;
                        c.min(1.0 - c)
                    })
                    .sum();
                let shift = Vec2::new(FINE as f32 / 2.0 - ox as f32, FINE as f32 / 2.0 - oy as f32) / FINE as f32;
                fits.push((blur + (1.0 - scale) * grid as f32 * 2.0 + shift.length() * 0.5, k, ox, oy));
            }
        }
        // Its parts and holes are the same wherever it sits on the sprite, so they are found once
        let (label, found) = shape_regions(&canvas, wide);
        canvases.push((scale, canvas, label, found));
    }
    fits.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Then the sharpest two at each size drawn out properly, and judged as sprites
    let mut best: Option<(f32, Vec<bool>)> = None;
    let finalists = (0..canvases.len()).flat_map(|k| fits.iter().filter(move |f| f.1 == k).take(2));
    for &(_, k, ox, oy) in finalists {
        let (scale, canvas, label, found) = &canvases[k];
        let side = grid * FINE;
        let window = |i: usize| (oy + i / side) * wide + ox + i % side;
        let fine: Vec<bool> = (0..side * side).map(|i| canvas[window(i)]).collect();
        let label: Vec<u32> = (0..side * side).map(|i| label[window(i)]).collect();
        let (cells, cover, faults) = sprite_cells(&fine, &label, found, grid);
        let (_, got) = regions(&cells, grid);
        let specks = got.iter().filter(|r| r.area == 1 && (r.inked || !r.border)).count() as f32;
        let off: f32 = cells.iter().zip(&cover).map(|(&on, &c)| if on { 1.0 - c } else { c }).sum();
        let shift = Vec2::new(FINE as f32 / 2.0 - ox as f32, FINE as f32 / 2.0 - oy as f32) / FINE as f32;
        let score = faults * grid as f32 + specks * 1.5 + off + (1.0 - scale) * grid as f32 * 2.0 + shift.length() * 0.5;
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, cells));
        }
    }
    best.map(|(_, cells)| cells)
}

/// The sprite's drop shadow on a canvas one block bigger: the glyph moved one block down and right,
/// left out wherever the glyph itself or a hole enclosed by it is, so it only falls outside it.
fn pixel_shadow(cells: &[bool], grid: usize) -> Vec<bool> {
    let side = grid + 1;
    let ink = |x: usize, y: usize| x < grid && y < grid && cells[y * grid + x];
    // Everything blank that can be reached from the border is outside; the rest is the glyph or its holes
    let mut outside = vec![false; side * side];
    let mut stack: Vec<(usize, usize)> = (0..side).flat_map(|i| [(i, 0), (i, side - 1), (0, i), (side - 1, i)]).collect();
    while let Some((x, y)) = stack.pop() {
        if outside[y * side + x] || ink(x, y) {
            continue;
        }
        outside[y * side + x] = true;
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx < side && ny < side {
                stack.push((nx, ny));
            }
        }
    }
    (0..side * side).map(|i| (i % side, i / side)).map(|(x, y)| x > 0 && y > 0 && ink(x - 1, y - 1) && outside[y * side + x]).collect()
}

/// A mask as a texture: white where it is set, so the painter's tint gives it any color. Kept for the
/// life of the window.
fn mask_texture(ctx: &egui::Context, name: String, side: usize, make: impl FnOnce() -> Option<Vec<bool>>) -> Option<egui::TextureHandle> {
    let id = egui::Id::NULL.with(&name);
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(tex);
    }
    let rgba: Vec<u8> = make()?.into_iter().flat_map(|on| [if on { 255 } else { 0 }; 4]).collect();
    let image = egui::ColorImage::from_rgba_premultiplied([side; 2], &rgba);
    let tex = ctx.load_texture(name, image, egui::TextureOptions::NEAREST);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// Pixels per block, in physical pixels, sized so blocks land on whole screen pixels.
fn pixel_rect(painter: &egui::Painter, center: Pos2, cells: Vec2, block: f32) -> Rect {
    let ppp = painter.ctx().pixels_per_point();
    let size = cells * block / ppp;
    let min = center - size / 2.0;
    Rect::from_min_size(Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp), size)
}

/// The glyph thresholded straight onto the cells, centred: a stand-in for the few frames until its
/// fitted sprite is ready.
fn quick_cells(glyph: &str, grid: usize) -> Option<Vec<bool>> {
    use ab_glyph::Font;
    let font = nerd_font()?;
    let id = font.glyph_id(glyph.chars().next()?);
    Some(glyph_coverage(font, id, grid, 1.0, Vec2::ZERO)?.into_iter().map(|c| c >= 0.5).collect())
}

type Fitted = Option<std::sync::Arc<Vec<bool>>>;

/// The fitted sprite of a glyph on `grid` cells, once it is ready. Fitting takes a few milliseconds a
/// sprite, too long for the frame that first shows a folder of many kinds of files, so it is done on
/// worker threads, and the window repainted as each sprite comes in.
fn fitted_cells(ctx: &egui::Context, glyph: char, grid: usize) -> Option<std::sync::Arc<Vec<bool>>> {
    use std::collections::HashMap;
    use std::sync::{mpsc, Arc, Mutex};
    type Job = (char, usize, egui::Context);
    static DONE: OnceLock<Mutex<HashMap<(char, usize), Fitted>>> = OnceLock::new();
    static QUEUE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let done = DONE.get_or_init(Default::default);
    if let Some(fitted) = done.lock().ok()?.get(&(glyph, grid)) {
        return fitted.clone();
    }
    let queue = QUEUE.get_or_init(|| {
        let (send, receive) = mpsc::channel::<Job>();
        let receive = Arc::new(Mutex::new(receive));
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).clamp(1, 4));
        for _ in 0..workers {
            let receive = receive.clone();
            std::thread::spawn(move || loop {
                let Ok((glyph, grid, ctx)) = receive.lock().map_err(drop).and_then(|r| r.recv().map_err(drop)) else { return };
                let cells = pixel_cells(&glyph.to_string(), grid).map(Arc::new);
                if let Ok(mut done) = DONE.get_or_init(Default::default).lock() {
                    done.insert((glyph, grid), cells);
                }
                ctx.request_repaint();
            });
        }
        Mutex::new(send)
    });
    // Asked for once: a pending sprite is marked so the next frames do not ask again
    done.lock().ok()?.insert((glyph, grid), None);
    queue.lock().ok()?.send((glyph, grid, ctx.clone())).ok()?;
    None
}

/// Returns false when the glyph is not in the font, so the caller can fall back to text.
fn paint_pixel_glyph(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, block: f32, color: Color32, shadow: bool) -> bool {
    use ab_glyph::Font;
    let Some(c) = glyph.chars().next() else { return false };
    if nerd_font().is_none_or(|font| font.glyph_id(c).0 == 0) {
        return false;
    }
    let ctx = painter.ctx();
    let grid = (size * ctx.pixels_per_point() / block).round().clamp(6.0, 64.0) as usize;
    let fitted = fitted_cells(ctx, c, grid);
    // The stand-in and the fitted sprite are kept apart, so the fitted one replaces it when it comes
    let name = if fitted.is_some() { format!("{glyph}-{grid}") } else { format!("{glyph}-{grid}-quick") };
    let cells = || fitted.as_ref().map(|cells| cells.to_vec()).or_else(|| quick_cells(glyph, grid));
    let Some(tex) = mask_texture(ctx, format!("pixel-glyph-{name}"), grid, cells) else { return false };
    let rect = pixel_rect(painter, center, Vec2::splat(grid as f32), block);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if shadow {
        // A hard drop shadow one block down and right, like a sprite
        let make = || cells().map(|cells| pixel_shadow(&cells, grid));
        if let Some(tex) = mask_texture(ctx, format!("pixel-shadow-{name}"), grid + 1, make) {
            let step = rect.width() / grid as f32;
            let area = Rect::from_min_size(rect.min, rect.size() + Vec2::splat(step));
            painter.image(tex.id(), area, uv, darken(color, 0.7).gamma_multiply(0.55));
        }
    }
    painter.image(tex.id(), rect, uv, color);
    true
}

/// A 16×13 sprite: `o` outline, `b` back panel, `h` highlight, `f` front, `s` shade.
const PIXEL_FOLDER: [&str; 13] = [
    ".oooooo.........",
    "obbbbbbo........",
    "obbbbbbboooooooo",
    "ohhhhhhhhhhhhhho",
    "ohffffffffffffso",
    "ohffffffffffffso",
    "ohffffffffffffso",
    "ohffffffffffffso",
    "ohffffffffffffso",
    "ohffffffffffffso",
    "ohffffffffffffso",
    "osssssssssssssso",
    ".oooooooooooooo.",
];

fn pixel_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let block = (size * 1.05 * painter.ctx().pixels_per_point() / 16.0).round().max(1.0);
    let rect = pixel_rect(painter, center, Vec2::new(16.0, 13.0), block);
    let step = rect.width() / 16.0;
    let shade = |c: u8| match c {
        b'o' => Some(darken(color, 0.55)),
        b'b' => Some(darken(color, 0.22)),
        b'h' => Some(lighten(color, 0.4)),
        b'f' => Some(color),
        b's' => Some(darken(color, 0.12)),
        _ => None,
    };
    for (y, row) in PIXEL_FOLDER.iter().enumerate() {
        let row = row.as_bytes();
        let mut x = 0;
        // One rectangle per run of equal pixels keeps a sprite to a few dozen shapes
        while x < row.len() {
            let mut end = x + 1;
            while end < row.len() && row[end] == row[x] {
                end += 1;
            }
            if let Some(c) = shade(row[x]) {
                let min = rect.min + Vec2::new(x as f32, y as f32) * step;
                painter.rect_filled(Rect::from_min_size(min, Vec2::new((end - x) as f32 * step, step)), CornerRadius::ZERO, c);
            }
            x = end;
        }
    }
}

// ---- Sketch ----

/// A path traced twice by an unsteady hand: a firm line, then a lighter one that strays a little.
fn sketch_path(painter: &egui::Painter, pts: &[Pos2], closed: bool, stroke: Stroke, amp: f32, seed: u32) {
    let n = pts.len();
    let segments = if closed { n } else { n - 1 };
    for pass in 0..2u32 {
        let seed = seed.wrapping_add(pass * 7919);
        let shift = Vec2::new(noise(seed, 1000), noise(seed, 1001)) * amp * pass as f32;
        let mut out = Vec::new();
        let mut k = 0;
        for s in 0..segments {
            let (a, b) = (pts[s], pts[(s + 1) % n]);
            let d = b - a;
            let normal = d.rot90().normalized();
            let steps = ((d.length() / (amp * 8.0).max(3.0)).ceil() as usize).clamp(2, 8);
            for j in 0..steps {
                out.push(a + d * (j as f32 / steps as f32) + normal * noise(seed, k) * amp + shift);
                k += 1;
            }
        }
        // A closed outline comes back round its first corner and ends a touch past it, the way a
        // pencil overshoots
        if closed {
            out.push(out[0]);
            out.push(out[0] + (out[1] - out[0]) * 0.6);
        } else {
            out.push(pts[n - 1] + shift);
        }
        let stroke = if pass == 0 { stroke } else { Stroke::new(stroke.width * 0.55, stroke.color.gamma_multiply(0.55)) };
        painter.add(Shape::line(out, stroke));
    }
}

/// Diagonal hatching over `rect`.
fn hatch(painter: &egui::Painter, rect: Rect, gap: f32, stroke: Stroke) {
    let h = rect.height();
    let mut t = -h;
    while t < rect.width() {
        let mut a = Pos2::new(rect.left() + t, rect.bottom());
        let mut b = Pos2::new(rect.left() + t + h, rect.top());
        if a.x < rect.left() {
            a = Pos2::new(rect.left(), rect.bottom() - (rect.left() - a.x));
        }
        if b.x > rect.right() {
            b = Pos2::new(rect.right(), rect.top() + (b.x - rect.right()));
        }
        if a.x < b.x {
            painter.line_segment([a, b], stroke);
        }
        t += gap;
    }
}

fn sketch_width(size: f32) -> f32 {
    (size / 14.0).clamp(1.1, 3.0)
}

fn sketch_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let f = folder_box(center, size);
    let stroke = Stroke::new(sketch_width(size), color);
    let amp = size * 0.018;
    painter.rect_filled(f.front(), CornerRadius::ZERO, color.gamma_multiply(0.2));
    let tab = Rect::from_min_max(f.body.min, Pos2::new(f.tab_right, f.front_top));
    painter.rect_filled(tab, CornerRadius::ZERO, color.gamma_multiply(0.1));
    if size >= 24.0 {
        hatch(painter, tab.shrink(stroke.width), (size * 0.06).max(2.5), Stroke::new(stroke.width * 0.45, color.gamma_multiply(0.6)));
    }
    let seed = seed_of("folder");
    sketch_path(painter, &f.silhouette(), true, stroke, amp, seed);
    let lip = [Pos2::new(f.body.left(), f.front_top), Pos2::new(f.tab_right + f.r, f.front_top)];
    sketch_path(painter, &lip, false, stroke, amp, seed ^ 0x55);
}

/// A dog-eared page drawn in pencil, with the file's glyph on it.
fn sketch_page(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    let (rect, fold, outline) = page(center, size);
    let stroke = Stroke::new(sketch_width(size), color);
    let amp = size * 0.016;
    let corner = Pos2::new(rect.right() - fold, rect.top() + fold);
    poly(painter, fold_triangle(rect, fold), color.gamma_multiply(0.3));
    let seed = seed_of(glyph);
    sketch_path(painter, &outline, true, stroke, amp, seed);
    sketch_path(painter, &[Pos2::new(rect.right() - fold, rect.top()), corner, Pos2::new(rect.right(), rect.top() + fold)], false, stroke, amp, seed ^ 0xAA);
    plain(painter, center + Vec2::new(0.0, size * 0.06), glyph, size * 0.56, color);
}

// ---- Glossy ----

fn outline_stroke(size: f32, color: Color32) -> Stroke {
    Stroke::new((size / 48.0).max(0.8), darken(color, 0.4))
}

/// Shine over the top part of a shape: white fading out downwards.
fn shine(painter: &egui::Painter, pts: &[Pos2], r: f32, color: Color32) {
    gradient(painter, &rounded(pts, r), white(color, 0.55), white(color, 0.0));
}

fn glossy_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let f = folder_box(center, size);
    let r = f.r;
    let tab = rounded(&f.tab(), r * 0.6);
    gradient(painter, &tab, darken(color, 0.05), darken(color, 0.4));
    painter.add(Shape::closed_line(tab, outline_stroke(size, color)));
    let front = rounded(&corners(f.front()), r);
    gradient(painter, &front, lighten(color, 0.35), darken(color, 0.18));
    let gloss = Rect::from_min_max(f.front().min + Vec2::splat(r * 0.45), Pos2::new(f.front().right() - r * 0.45, f.front().top() + f.front().height() * 0.45));
    shine(painter, &corners(gloss), r * 0.7, color);
    painter.add(Shape::closed_line(front, outline_stroke(size, color)));
}

fn glossy_file(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    let (rect, fold, outline) = page(center, size);
    // The corners either side of the fold stay sharp, so the folded triangle meets them exactly
    let m = size * 0.05;
    let shape = rounded_each(&outline, &[m, 0.0, 0.0, m, m]);
    gradient(painter, &shape, lighten(color, 0.4), darken(color, 0.12));
    let gloss = [
        rect.min + Vec2::splat(m),
        Pos2::new(rect.right() - fold - m * 0.4, rect.top() + m),
        Pos2::new(rect.right() - m, rect.top() + fold + m * 0.4),
        Pos2::new(rect.right() - m, rect.top() + rect.height() * 0.45),
        Pos2::new(rect.left() + m, rect.top() + rect.height() * 0.45),
    ];
    shine(painter, &gloss, m, color);
    gradient(painter, &fold_triangle(rect, fold), lighten(color, 0.7), lighten(color, 0.2));
    painter.add(Shape::closed_line(shape, outline_stroke(size, color)));
    plain(painter, center + Vec2::new(0.0, size * 0.07), glyph, page_glyph(size), ink(color));
}

// ---- Clay ----

/// How far the clay's underside shows below it.
fn clay_depth(size: f32) -> Vec2 {
    Vec2::new(0.0, (size * 0.06).max(1.5))
}

fn clay_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let f = folder_box(center, size);
    let r = f.r * 1.6;
    let depth = clay_depth(size);
    let under = darken(color, 0.42);
    let tab = rounded(&f.tab(), r * 0.6);
    let front = f.front();
    // The underside first, peeking out below the whole lump
    poly(painter, tab.iter().map(|p| *p + depth).collect(), under);
    painter.rect_filled(front.translate(depth), CornerRadius::same(radius(r)), under);
    poly(painter, tab, darken(color, 0.18));
    painter.rect_filled(front, CornerRadius::same(radius(r)), color);
    soft_light(painter, front, r, color);
    // A dull highlight where the lid meets the front
    let lip = Rect::from_min_max(front.min + Vec2::new(r * 0.6, r * 0.35), Pos2::new(front.right() - r * 0.6, front.top() + r * 0.7));
    painter.rect_filled(lip, CornerRadius::same(radius(lip.height() / 2.0)), lighten(color, 0.4));
}

/// Clay catching the light: a little brighter on top, shading off towards the bottom.
fn soft_light(painter: &egui::Painter, rect: Rect, r: f32, color: Color32) {
    gradient(painter, &rounded(&corners(rect), radius(r) as f32), white(color, 0.0), Color32::from_black_alpha((0.1 * color.a() as f32) as u8));
}

fn clay_file(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    let tile = Rect::from_center_size(center - clay_depth(size) * 0.5, Vec2::new(size * 0.8, size * 0.84));
    let r = CornerRadius::same(radius(size * 0.24));
    painter.rect_filled(tile.translate(clay_depth(size)), r, darken(color, 0.42));
    painter.rect_filled(tile, r, color);
    soft_light(painter, tile, size * 0.24, color);
    let hi = Rect::from_min_size(tile.min + Vec2::new(size * 0.14, size * 0.08), Vec2::new(size * 0.3, size * 0.07));
    painter.rect_filled(hi, CornerRadius::same(radius(hi.height() / 2.0)), lighten(color, 0.45));
    plain(painter, tile.center() + Vec2::new(0.0, size * 0.03), glyph, page_glyph(size) * 1.05, ink(color));
}

// ---- Origami ----

/// A folder folded from paper like an envelope: a flap folded down over the front, and the front
/// creased in two facets of different light.
fn origami_folder(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let f = folder_box(center, size);
    let b = f.body;
    // The tab, its top left corner folded back onto it: the corner cut off along the diagonal, and
    // the flap it makes mirrored across that cut
    let dog = (f.front_top - b.top()) * 0.75;
    let mut tab = f.tab();
    tab.splice(1..2, [Pos2::new(b.left(), b.top() + dog), Pos2::new(b.left() + dog, b.top())]);
    poly(painter, tab, darken(color, 0.3));
    poly(painter, vec![Pos2::new(b.left(), b.top() + dog), Pos2::new(b.left() + dog, b.top()), Pos2::new(b.left() + dog, b.top() + dog)], darken(color, 0.12));

    // The front in its darker facet, the lighter one laid over it, so the facets meet without a seam
    let front = f.front();
    let (top_x, bottom_x) = (front.left() + front.width() * 0.56, front.left() + front.width() * 0.44);
    painter.rect_filled(front, CornerRadius::ZERO, darken(color, 0.06));
    poly(painter, vec![front.left_top(), Pos2::new(top_x, front.top()), Pos2::new(bottom_x, front.bottom()), front.left_bottom()], lighten(color, 0.14));
    // The crease between the facets, at height `t` of the front
    let crease = |t: f32| Pos2::new(top_x + (bottom_x - top_x) * t, front.top() + front.height() * t);
    // The flap's tip lands on the crease, and its shadow falls straight down onto the front
    let (depth, drop) = (0.42, 0.06);
    let tip = crease(depth);
    let shadow_tip = crease(depth + drop);
    let dy = Vec2::new(0.0, shadow_tip.y - tip.y);
    poly(painter, vec![front.left_top(), front.right_top(), front.right_top() + dy, shadow_tip, front.left_top() + dy], darken(color, 0.3));
    poly(painter, vec![front.left_top(), front.right_top(), tip], darken(color, 0.2));
    if size >= 28.0 {
        painter.line_segment([shadow_tip, crease(1.0)], Stroke::new((size / 70.0).max(0.7), lighten(color, 0.4)));
    }
}

fn origami_file(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, color: Color32) {
    let (rect, fold, outline) = page(center, size);
    let (top_x, bottom_x) = (rect.left() + rect.width() * 0.42, rect.left() + rect.width() * 0.58);
    // The page in its darker facet, the lighter one laid over it, so the facets meet without a seam
    poly(painter, outline, darken(color, 0.06));
    poly(painter, vec![rect.left_top(), Pos2::new(top_x, rect.top()), Pos2::new(bottom_x, rect.bottom()), rect.left_bottom()], lighten(color, 0.14));
    // The corner folded over onto the page, and the shadow it throws
    let corner = Pos2::new(rect.right() - fold, rect.top() + fold);
    let (a, b) = (Pos2::new(rect.right() - fold, rect.top()), Pos2::new(rect.right(), rect.top() + fold));
    poly(painter, vec![a, b, corner + Vec2::new(-fold * 0.12, fold * 0.12)], darken(color, 0.3));
    poly(painter, vec![a, b, corner], lighten(color, 0.3));
    plain(painter, center + Vec2::new(0.0, size * 0.07), glyph, page_glyph(size), ink(color));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shading_keeps_alpha() {
        let c = Color32::from_rgba_premultiplied(100, 50, 20, 128);
        assert_eq!(darken(c, 0.5).a(), 128);
        assert_eq!(lighten(c, 1.0), Color32::from_rgba_premultiplied(128, 128, 128, 128));
    }

    #[test]
    fn ink_contrasts_with_the_fill() {
        assert!(ink(Color32::from_rgb(240, 240, 240)).r() < 100);
        assert!(ink(Color32::from_rgb(30, 30, 60)).r() > 150);
    }

    #[test]
    fn style_round_trips_through_settings() {
        for style in IconStyle::ALL {
            let json = serde_json::to_string(&style).unwrap();
            assert_eq!(serde_json::from_str::<IconStyle>(&json).unwrap(), style);
        }
        // A style that was removed reads as the default rather than failing
        assert_eq!(serde_json::from_str::<IconStyle>("\"Neon\"").unwrap(), IconStyle::Classic);
    }

    #[test]
    fn pixel_folder_sprite_is_rectangular() {
        assert!(PIXEL_FOLDER.iter().all(|row| row.len() == 16));
    }

    /// The parts and holes of a glyph's pixel sprite on `grid` cells.
    fn sprite_features(glyph: &str, grid: usize) -> (usize, usize) {
        let cells = pixel_cells(glyph, grid).unwrap();
        let (_, found) = regions(&cells, grid);
        (found.iter().filter(|r| r.inked).count(), found.iter().filter(|r| !r.inked && !r.border).count())
    }

    #[test]
    fn pixel_sprites_keep_what_makes_a_logo() {
        for grid in [16, 20, 24] {
            // Python's two snakes, each with its eye
            assert_eq!(sprite_features(icons::PYTHON, grid), (2, 2), "python on {grid}");
            // The gear of Rust in one piece, the R and the gear's middle open
            let (parts, holes) = sprite_features(icons::RUST, grid);
            assert!(parts == 1 && holes >= 2, "rust on {grid}: {parts} parts, {holes} holes");
            // The facets of Elixir's drop meet in hairlines too fine to show, so it is one drop
            assert_eq!(sprite_features(icons::ELIXIR, grid).0, 1, "elixir on {grid}");
        }
        // With room enough, Docker's containers stand apart from each other and from the whale
        assert!(sprite_features(icons::DOCKER, 24).0 >= 8);
        // Java's strokes, thinner than a cell, are drawn rather than lost
        assert!(sprite_features(icons::JAVA, 24).0 >= 6);
    }

    #[test]
    fn thinning_keeps_lines_joined_and_held_cells() {
        // A bar two cells thick, its left end held
        let side = 8;
        let on: Vec<bool> = (0..side * side).map(|i| (3..5).contains(&(i / side)) && (1..7).contains(&(i % side))).collect();
        let keep: Vec<bool> = (0..side * side).map(|i| i == 3 * side + 1 || i == 4 * side + 1).collect();
        let thin = thin(on.clone(), &keep, side);
        assert!(keep.iter().zip(&thin).all(|(&k, &t)| !k || t));
        let (_, found) = regions(&thin, side);
        assert_eq!(found.iter().filter(|r| r.inked).count(), 1);
        assert!(thin.iter().filter(|&&t| t).count() < on.iter().filter(|&&o| o).count());
    }

    #[test]
    fn noise_is_steady_and_bounded() {
        for i in 0..100 {
            let v = noise(7, i);
            assert!((-1.0..=1.0).contains(&v));
            assert_eq!(v, noise(7, i));
        }
    }
}
