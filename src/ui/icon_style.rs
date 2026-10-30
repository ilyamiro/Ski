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
            let block = (size * painter.ctx().pixels_per_point() / 16.0).round().max(2.0);
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
    let earth = darken(color, 0.8);
    let grass = darken(color, 0.42);
    let mut stone = Rect::from_min_max(center + Vec2::new(-0.36, -0.4) * size, center + Vec2::new(0.36, 0.4) * size);
    let face_top;
    match seed % 3 {
        0 => {
            let top = radius(stone.width() / 2.0);
            painter.rect_filled(stone, CornerRadius { nw: top, ne: top, sw: 1, se: 1 }, stone_c);
            let face = stone.shrink(size * 0.05);
            let top = radius(face.width() / 2.0);
            painter.rect_filled(face, CornerRadius { nw: top, ne: top, sw: 1, se: 1 }, face_c);
            face_top = face.top() + size * 0.1;
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
            poly(painter, outline, stone_c);
            poly(painter, face, face_c);
            face_top = stone.top() + size * 0.12;
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
        painter.text(Pos2::new(stone.center().x, face_top + size * 0.04), Align2::CENTER_CENTER, "R.I.P.", FontId::proportional(size * 0.11), color.gamma_multiply(0.85));
    }
    let (glyph_size, glyph_at) = if rip {
        (size * 0.42, center + Vec2::new(0.0, size * 0.1))
    } else {
        (size * if size < 28.0 { 0.62 } else { 0.52 }, center + Vec2::new(0.0, size * 0.04 + (face_top - stone.top() - size * 0.05).max(0.0) * 0.3))
    };
    plain(painter, glyph_at, glyph, glyph_size, color);

    let ground = stone.bottom() + size * 0.02;
    painter.add(Shape::ellipse_filled(Pos2::new(center.x, ground), Vec2::new(size * 0.5, size * 0.08), earth));
    if size >= 24.0 {
        for (i, x) in [-0.4f32, -0.3, 0.27, 0.4].into_iter().enumerate() {
            let x = center.x + x * size;
            for blade in [-1.0, 0.0, 1.0] {
                let lean = (blade * 0.03 + noise(seed, i as u32) * 0.02) * size;
                let bx = x + blade * size * 0.02;
                poly(painter, vec![Pos2::new(bx - size * 0.015, ground), Pos2::new(bx + lean, ground - size * 0.09), Pos2::new(bx + size * 0.015, ground)], grass);
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

/// The glyph rasterized onto a `grid`×`grid` canvas with hard edges: true where it is inked.
fn pixel_cells(glyph: &str, grid: usize) -> Option<Vec<bool>> {
    use ab_glyph::Font;
    let font = nerd_font()?;
    let outline = font.outline_glyph(font.glyph_id(glyph.chars().next()?).with_scale(grid as f32))?;
    let bounds = outline.px_bounds();
    let dx = ((grid as f32 - bounds.width()) / 2.0).round() as i32;
    let dy = ((grid as f32 - bounds.height()) / 2.0).round() as i32;
    let mut cells = vec![false; grid * grid];
    outline.draw(|x, y, coverage| {
        let (x, y) = (x as i32 + dx, y as i32 + dy);
        if coverage >= 0.5 && (0..grid as i32).contains(&x) && (0..grid as i32).contains(&y) {
            cells[y as usize * grid + x as usize] = true;
        }
    });
    Some(cells)
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

/// Returns false when the glyph is not in the font, so the caller can fall back to text.
fn paint_pixel_glyph(painter: &egui::Painter, center: Pos2, glyph: &str, size: f32, block: f32, color: Color32, shadow: bool) -> bool {
    let grid = (size * painter.ctx().pixels_per_point() / block).round().clamp(6.0, 64.0) as usize;
    let Some(tex) = mask_texture(painter.ctx(), format!("pixel-glyph-{glyph}-{grid}"), grid, || pixel_cells(glyph, grid)) else { return false };
    let rect = pixel_rect(painter, center, Vec2::splat(grid as f32), block);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if shadow {
        // A hard drop shadow one block down and right, like a sprite
        let make = || pixel_cells(glyph, grid).map(|cells| pixel_shadow(&cells, grid));
        if let Some(tex) = mask_texture(painter.ctx(), format!("pixel-shadow-{glyph}-{grid}"), grid + 1, make) {
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

    #[test]
    fn noise_is_steady_and_bounded() {
        for i in 0..100 {
            let v = noise(7, i);
            assert!((-1.0..=1.0).contains(&v));
            assert_eq!(v, noise(7, i));
        }
    }
}
