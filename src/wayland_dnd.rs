//! Drag and drop between windows on Wayland. winit offers neither side of it there, so this talks
//! to the compositor itself, on its own event queue of the window's display: it starts a drag
//! (`wl_data_device.start_drag`) when files are carried out of the window, and takes part as a
//! target for drags from any window, Ski's own or another program's. Files travel as
//! `text/uri-list`, like in Nautilus. The queue is dispatched from the frame loop (`poll`), which
//! never touches the socket; a small thread reads it and wakes the loop when an event is for this queue.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, ButtonState, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child};

const URI_LIST: &str = "text/uri-list";
const PLAIN_TEXT: &str = "text/plain;charset=utf-8";
/// Offered only by Ski, so a drag that comes back to it is known without reading the files.
const OWN_MIME: &str = "application/x-ski-drag";
const ICON_SIZE: i32 = 64;

/// A drag that is over the window.
pub struct Hover {
    /// Window coordinates, in the compositor's logical pixels.
    pub pos: (f64, f64),
    /// The dragged files; empty until they have been read.
    pub paths: Vec<PathBuf>,
}

/// Files let go over the window.
pub struct Dropped {
    pub pos: (f64, f64),
    pub paths: Vec<PathBuf>,
    pub copy: bool,
}

#[derive(Default)]
pub struct Poll {
    pub hover: Option<Hover>,
    pub dropped: Option<Dropped>,
    /// Something is still on its way; call again soon.
    pub busy: bool,
}

#[derive(Default)]
struct Reader {
    fd: Option<OwnedFd>,
    data: Vec<u8>,
}

impl Reader {
    fn done(&self) -> bool {
        self.fd.is_none()
    }

    fn pump(&mut self) {
        let Some(fd) = &self.fd else { return };
        let mut buf = [0u8; 8192];
        loop {
            // SAFETY: reads into a buffer of the given length from a descriptor we own
            let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                self.data.extend_from_slice(&buf[..n as usize]);
                if self.data.len() > 16 << 20 {
                    self.fd = None;
                    return;
                }
            } else if n == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock {
                self.fd = None;
                return;
            } else {
                return;
            }
        }
    }
}

struct Incoming {
    offer: WlDataOffer,
    serial: u32,
    pos: (f64, f64),
    own: bool,
    has_files: bool,
    accepting: bool,
    action: DndAction,
    reader: Reader,
}

struct OfferInfo {
    mimes: Vec<String>,
}

struct Outgoing {
    source: WlDataSource,
    paths: Vec<PathBuf>,
    uris: String,
    text: String,
    icon: Option<(WlSurface, WlBuffer, WlShmPool)>,
}

#[derive(Default)]
struct State {
    surface: Option<ObjectId>,
    /// Only while the seat has a pointer; its button presses give the serial a drag starts with.
    pointer: Option<WlPointer>,
    press_serial: Option<u32>,
    offers: HashMap<ObjectId, OfferInfo>,
    selection: Option<WlDataOffer>,
    hover: Option<Incoming>,
    dropping: Option<Incoming>,
    out: Option<Outgoing>,
}

pub struct WaylandDnd {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    surface: WlSurface,
    compositor: WlCompositor,
    shm: Option<WlShm>,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    _seat: WlSeat,
    /// Asks for a frame when an event lands in this queue.
    waker: std::task::Waker,
}

struct Repaint(eframe::egui::Context);

impl std::task::Wake for Repaint {
    fn wake(self: std::sync::Arc<Self>) {
        self.0.request_repaint();
    }
}

impl WaylandDnd {
    /// `None` when the window is not a Wayland one or the compositor lacks what is needed.
    pub fn new<W: HasWindowHandle + HasDisplayHandle>(window: &W, ctx: &eframe::egui::Context) -> Option<Self> {
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else { return None };
        let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else { return None };

        // SAFETY: both pointers come from the live window winit created, which outlives this
        // object; the display stays owned by winit (a foreign display is never disconnected by us).
        let connection = Connection::from_backend(unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) });
        let surface_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&connection, surface_id.clone()).ok()?;

        let (globals, queue) = registry_queue_init::<State>(&connection).ok()?;
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=5, ()).ok()?;
        let manager: WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).ok()?;
        let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).ok()?;
        let shm: Option<WlShm> = globals.bind(&qh, 1..=1, ()).ok();
        let device = manager.get_data_device(&seat, &qh, ());
        connection.flush().ok()?;

        let state = State { surface: Some(surface_id), ..State::default() };
        let waker = std::sync::Arc::new(Repaint(ctx.clone())).into();
        Self::spawn_reader(connection.clone());
        Some(Self { connection, queue, state, surface, compositor, shm, manager, device, _seat: seat, waker })
    }

    /// Reads the socket for this queue in the way libwayland expects of each reader (prepare, wait
    /// until it is readable, read), so it never holds up winit or the clipboard, which read the same
    /// display. Events read for this queue wake the frame loop through `waker`, since a drag over the
    /// window produces no events for egui; nothing else that arrives asks for a frame.
    fn spawn_reader(connection: Connection) {
        std::thread::spawn(move || {
            loop {
                let Some(guard) = connection.prepare_read() else {
                    // Another reader already took in events for this queue: hand them on
                    if connection.backend().dispatch_inner_queue().is_err() {
                        return;
                    }
                    continue;
                };
                let mut pfd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
                // SAFETY: one valid pollfd; the display outlives the process's windows
                let n = unsafe { libc::poll(&mut pfd, 1, -1) };
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return;
                }
                // Only read once there is something: reading an empty socket would wait for the other readers
                if pfd.revents & libc::POLLIN != 0 {
                    match guard.read() {
                        Ok(_) => {}
                        Err(wayland_backend::client::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => return,
                    }
                } else if pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                    return;
                }
            }
        });
    }

    /// Takes in what the compositor has sent and reports the state of a drag over the window.
    /// Never waits on the socket: the reader thread takes care of that.
    pub fn poll(&mut self) -> Poll {
        let mut cx = std::task::Context::from_waker(&self.waker);
        let _ = self.queue.poll_dispatch_pending(&mut cx, &mut self.state);

        let mut poll = Poll::default();
        if let Some(h) = &mut self.state.hover {
            h.reader.pump();
            poll.busy |= !h.reader.done();
            poll.hover = Some(Hover { pos: h.pos, paths: Self::paths_of(h, &self.state.out) });
        }
        if let Some(d) = &mut self.state.dropping {
            d.reader.pump();
            if d.reader.done() || d.own {
                let d = self.state.dropping.take().unwrap();
                let paths = Self::paths_of(&d, &self.state.out);
                let copy = d.action == DndAction::Copy;
                if d.action == DndAction::Copy || d.action == DndAction::Move {
                    if d.offer.version() >= 3 {
                        d.offer.finish();
                    }
                    poll.dropped = Some(Dropped { pos: d.pos, paths, copy });
                }
                self.state.offers.remove(&d.offer.id());
                d.offer.destroy();
            } else {
                poll.busy = true;
            }
        }
        let _ = self.connection.flush();
        poll
    }

    fn paths_of(incoming: &Incoming, out: &Option<Outgoing>) -> Vec<PathBuf> {
        if incoming.own && let Some(out) = out {
            return out.paths.clone();
        }
        let text = String::from_utf8_lossy(&incoming.reader.data);
        text.lines().filter_map(|l| crate::clipboard::percent_decode(l.trim())).collect()
    }

    /// Tells the compositor whether the spot the drag is over takes a drop (it shows the pointer to match).
    pub fn set_accepting(&mut self, ok: bool) {
        let Some(h) = &mut self.state.hover else { return };
        if h.accepting == ok || !h.has_files {
            return;
        }
        h.accepting = ok;
        h.offer.accept(h.serial, ok.then(|| URI_LIST.to_owned()));
        if h.offer.version() >= 3 {
            let (all, preferred) = if ok { (DndAction::Copy | DndAction::Move, DndAction::Move) } else { (DndAction::empty(), DndAction::empty()) };
            h.offer.set_actions(all, preferred);
        }
        let _ = self.connection.flush();
    }

    /// Starts a drag of `paths` out of the window; `false` when it cannot be started (no button
    /// press to hang it on, or a drag is going on already).
    pub fn start_drag(&mut self, paths: &[PathBuf], style: IconStyle) -> bool {
        let Some(serial) = self.state.press_serial else { return false };
        if self.state.out.is_some() || paths.is_empty() {
            return false;
        }
        let qh = self.queue.handle();
        let source = self.manager.create_data_source(&qh, ());
        for mime in [OWN_MIME, URI_LIST, PLAIN_TEXT] {
            source.offer(mime.to_owned());
        }
        if source.version() >= 3 {
            source.set_actions(DndAction::Copy | DndAction::Move);
        }
        let uris: String = paths.iter().map(|p| format!("{}\r\n", crate::clipboard::percent_encode(p))).collect();
        let text: String = paths.iter().map(|p| format!("{}\n", p.display())).collect();

        let icon = self.drag_icon(&qh, paths.len(), &style);
        self.device.start_drag(Some(&source), &self.surface, icon.as_ref().map(|i| &i.0), serial);
        if let Some((surface, buffer, _)) = &icon {
            surface.set_buffer_scale(style.scale);
            surface.attach(Some(buffer), 0, 0);
            surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
            surface.commit();
        }
        self.state.out = Some(Outgoing { source, paths: paths.to_vec(), uris, text, icon });
        self.state.press_serial = None;
        let _ = self.connection.flush();
        true
    }

    /// The drag image, drawn into a shared-memory buffer.
    fn drag_icon(&self, qh: &QueueHandle<State>, count: usize, style: &IconStyle) -> Option<(WlSurface, WlBuffer, WlShmPool)> {
        let shm = self.shm.as_ref()?;
        let size = (ICON_SIZE * style.scale) as usize;
        let pixels = render_icon(size, style.scale as f32, count, style);
        // SAFETY: memfd_create takes a name and flags and returns a new descriptor we own
        let raw = unsafe { libc::memfd_create(c"ski-drag-icon".as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return None;
        }
        let mut file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(raw) });
        file.write_all(&pixels).ok()?;
        let pool = shm.create_pool(file.as_fd(), pixels.len() as i32, qh, ());
        let buffer = pool.create_buffer(0, size as i32, size as i32, size as i32 * 4, wl_shm::Format::Argb8888, qh, ());
        let surface = self.compositor.create_surface(qh, ());
        Some((surface, buffer, pool))
    }
}

/// How the drag image looks: colors (`0xAARRGGBB`), the type icons of the first few files (front
/// first), and the buffer scale of the output it appears on.
#[derive(Clone)]
pub struct IconStyle {
    pub card: u32,
    pub badge: u32,
    pub badge_text: u32,
    pub icons: Vec<(&'static str, u32)>,
    pub scale: i32,
}

/// Size of a file's card in the drag image, and how far each one behind it is shifted.
const CARD: f32 = 44.0;
const CARD_STEP: f32 = 6.0;

/// Premultiplied pixels, from 0 to 1.
struct Canvas {
    size: usize,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    /// Lays `argb` over the pixel at `coverage` (0 to 1).
    fn blend(&mut self, x: i32, y: i32, argb: u32, coverage: f32) {
        if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
            return;
        }
        let a = (argb >> 24) as f32 / 255.0 * coverage.clamp(0.0, 1.0);
        let src = [(argb >> 16 & 0xff) as f32 / 255.0 * a, (argb >> 8 & 0xff) as f32 / 255.0 * a, (argb & 0xff) as f32 / 255.0 * a, a];
        let dst = &mut self.px[y as usize * self.size + x as usize];
        for i in 0..4 {
            dst[i] = src[i] + dst[i] * (1.0 - a);
        }
    }

    /// A rounded rectangle, with its edge smoothed.
    fn rounded(&mut self, min: (f32, f32), size: (f32, f32), radius: f32, argb: u32) {
        let (x0, y0) = (min.0.floor() as i32, min.1.floor() as i32);
        let (x1, y1) = ((min.0 + size.0).ceil() as i32, (min.1 + size.1).ceil() as i32);
        let (cx, cy) = (min.0 + size.0 / 2.0, min.1 + size.1 / 2.0);
        let (hx, hy) = (size.0 / 2.0 - radius, size.1 / 2.0 - radius);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = ((x as f32 + 0.5 - cx).abs() - hx).max(0.0);
                let dy = ((y as f32 + 0.5 - cy).abs() - hy).max(0.0);
                let inside_x = (x as f32 + 0.5 - min.0).min(min.0 + size.0 - x as f32 - 0.5) + 0.5;
                let inside_y = (y as f32 + 0.5 - min.1).min(min.1 + size.1 - y as f32 - 0.5) + 0.5;
                let edge = (radius - (dx * dx + dy * dy).sqrt() + 0.5).min(inside_x).min(inside_y);
                self.blend(x, y, argb, edge);
            }
        }
    }

    /// `text` in `font`, centered on `center` by its inked outline.
    fn text(&mut self, font: &ab_glyph::FontRef, text: &str, px: f32, center: (f32, f32), argb: u32) {
        use ab_glyph::{Font, ScaleFont};
        let scaled = font.as_scaled(ab_glyph::PxScale::from(px));
        let mut pen = 0.0;
        let mut outlines = Vec::new();
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            outlines.extend(font.outline_glyph(id.with_scale_and_position(scaled.scale(), ab_glyph::point(pen, 0.0))));
            pen += scaled.h_advance(id);
        }
        let Some(first) = outlines.first() else { return };
        let mut bounds = first.px_bounds();
        for o in &outlines {
            let b = o.px_bounds();
            bounds.min.x = bounds.min.x.min(b.min.x);
            bounds.min.y = bounds.min.y.min(b.min.y);
            bounds.max.x = bounds.max.x.max(b.max.x);
            bounds.max.y = bounds.max.y.max(b.max.y);
        }
        let shift = (center.0 - (bounds.min.x + bounds.max.x) / 2.0, center.1 - (bounds.min.y + bounds.max.y) / 2.0);
        for outline in outlines {
            let b = outline.px_bounds();
            let (ox, oy) = ((b.min.x + shift.0).round() as i32, (b.min.y + shift.1).round() as i32);
            outline.draw(|gx, gy, cover| self.blend(ox + gx as i32, oy + gy as i32, argb, cover));
        }
    }
}

/// Premultiplied Argb8888 pixels: a card per file with its type icon, up to three stacked like a
/// pile, and a badge with the number of files when there is more than one.
fn render_icon(size: usize, scale: f32, count: usize, style: &IconStyle) -> Vec<u8> {
    let mut canvas = Canvas { size, px: vec![[0.0; 4]; size * size] };
    let symbols = ab_glyph::FontRef::try_from_slice(include_bytes!("../assets/fonts/SymbolsNerdFontMono-Regular.ttf")).ok();
    let sans = ab_glyph::FontRef::try_from_slice(include_bytes!("../assets/fonts/AdwaitaSans-Regular.ttf")).ok();
    let full = size as f32;
    let card = CARD * scale;
    let layers = count.clamp(1, 3);
    // The front card sits at the bottom left, those behind it up and to the right
    for layer in (0..layers).rev() {
        let step = layer as f32 * CARD_STEP * scale;
        let min = (4.0 * scale + step, full - 4.0 * scale - card - step);
        canvas.rounded((min.0, min.1 + 1.5 * scale), (card, card), 10.0 * scale, 0x40_00_00_00);
        // A dark edge keeps the cards of the pile apart
        canvas.rounded(min, (card, card), 10.0 * scale, style.badge_text & 0x00_FF_FF_FF | 0xC0 << 24);
        canvas.rounded((min.0 + scale, min.1 + scale), (card - 2.0 * scale, card - 2.0 * scale), 9.0 * scale, style.card);
        if let (Some(font), Some((glyph, color))) = (&symbols, style.icons.get(layer)) {
            canvas.text(font, glyph, 28.0 * scale, (min.0 + card / 2.0, min.1 + card / 2.0), *color);
        }
    }
    if count > 1 {
        let label = if count > 99 { "99+".to_owned() } else { count.to_string() };
        let height = 22.0 * scale;
        let width = height.max((label.len() as f32 * 7.5 + 10.0) * scale);
        let min = (full - width - 1.0 * scale, 1.0 * scale);
        canvas.rounded(min, (width, height), height / 2.0, style.badge);
        if let Some(font) = &sans {
            canvas.text(font, &label, 13.0 * scale, (min.0 + width / 2.0, min.1 + height / 2.0), style.badge_text);
        }
    }

    let mut out = Vec::with_capacity(size * size * 4);
    for [r, g, b, a] in canvas.px {
        // Already premultiplied, as Argb8888 wants it
        for c in [b, g, r, a] {
            out.push((c * 255.0 + 0.5) as u8);
        }
    }
    out
}

impl State {
    fn end_drag(&mut self) {
        if let Some(out) = self.out.take() {
            out.source.destroy();
            if let Some((surface, buffer, pool)) = out.icon {
                surface.destroy();
                buffer.destroy();
                pool.destroy();
            }
        }
    }
}

fn write_async(fd: OwnedFd, data: String) {
    std::thread::spawn(move || {
        let _ = std::fs::File::from(fd).write_all(data.as_bytes());
    });
}

fn dnd_action(action: WEnum<DndAction>) -> DndAction {
    match action {
        WEnum::Value(a) => a,
        WEnum::Unknown(_) => DndAction::empty(),
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

macro_rules! ignore_events {
    ($($proxy:ty),*) => {$(
        impl Dispatch<$proxy, ()> for State {
            fn event(_: &mut Self, _: &$proxy, _: <$proxy as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore_events!(WlDataDeviceManager, WlCompositor, WlShm, WlShmPool, WlBuffer, WlSurface);

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        // Asking a seat without a pointer for one is a protocol error, which would end the window's connection too
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event {
            let has = caps.contains(wl_seat::Capability::Pointer);
            if has && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            } else if !has && let Some(pointer) = state.pointer.take() {
                if pointer.version() >= 3 {
                    pointer.release();
                }
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(state: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_pointer::Event::Button { serial, state: WEnum::Value(ButtonState::Pressed), .. } = event {
            state.press_serial = Some(serial);
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(state: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), OfferInfo { mimes: Vec::new() });
            }
            wl_data_device::Event::Enter { serial, surface, x, y, id } => {
                if let Some(old) = state.hover.take() {
                    state.offers.remove(&old.offer.id());
                    old.offer.destroy();
                }
                let Some(offer) = id else { return };
                let Some(info) = state.offers.get(&offer.id()) else { return };
                // Every Ski offers its own type; only a drag this window started has its files at hand.
                // One from another Ski window (a process of its own) is read like any other program's
                let own = info.mimes.iter().any(|m| m == OWN_MIME) && state.out.is_some();
                let has_files = info.mimes.iter().any(|m| m == URI_LIST);
                let mut reader = Reader::default();
                if has_files && !own && Some(surface.id()) == state.surface {
                    let mut fds = [0 as RawFd; 2];
                    // SAFETY: pipe2 fills two descriptors we then own
                    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } == 0 {
                        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
                        offer.receive(URI_LIST.to_owned(), write.as_fd());
                        reader.fd = Some(read);
                    }
                }
                if Some(surface.id()) != state.surface || !has_files {
                    offer.accept(serial, None);
                }
                state.hover = Some(Incoming { offer, serial, pos: (x, y), own, has_files, accepting: false, action: DndAction::empty(), reader });
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(h) = &mut state.hover {
                    h.pos = (x, y);
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(old) = state.hover.take() {
                    state.offers.remove(&old.offer.id());
                    old.offer.destroy();
                }
            }
            wl_data_device::Event::Drop => state.dropping = state.hover.take(),
            wl_data_device::Event::Selection { id } => {
                // Clipboard offers are of no use here
                if let Some(old) = state.selection.take() {
                    state.offers.remove(&old.id());
                    old.destroy();
                }
                state.selection = id;
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(state: &mut Self, offer: &WlDataOffer, event: wl_data_offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Some(info) = state.offers.get_mut(&offer.id()) {
                    info.mimes.push(mime_type);
                }
            }
            wl_data_offer::Event::Action { dnd_action: action } => {
                let action = dnd_action(action);
                for incoming in state.hover.iter_mut().chain(state.dropping.iter_mut()) {
                    if incoming.offer.id() == offer.id() {
                        incoming.action = action;
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(state: &mut Self, _: &WlDataSource, event: wl_data_source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                if let Some(out) = &state.out {
                    match mime_type.as_str() {
                        URI_LIST => write_async(fd, out.uris.clone()),
                        PLAIN_TEXT => write_async(fd, out.text.clone()),
                        _ => {}
                    }
                }
            }
            wl_data_source::Event::Cancelled | wl_data_source::Event::DndFinished => state.end_drag(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_stacks_the_files_and_shows_the_count() {
        let style = IconStyle { card: 0xFF_30_30_40, badge: 0xFF_C0_A0_FF, badge_text: 0xFF_1C_1B_22, icons: vec![(crate::ui::icons::FILE, 0xFF_FF_FF_FF); 3], scale: 2 };
        let size = (ICON_SIZE * 2) as usize;
        let alpha = |px: &[u8], x: usize, y: usize| px[(y * size + x) * 4 + 3];
        let one = render_icon(size, 2.0, 1, &style);
        let three = render_icon(size, 2.0, 3, &style);
        assert_eq!(three.len(), size * size * 4);
        // The front card is covered and its corner rounded off
        assert!(alpha(&one, 8 + 44, size - 8 - 44) > 200);
        assert_eq!(alpha(&one, 0, size - 1), 0);
        // Only several files carry the badge in the top right corner, and a pile behind the front card
        assert_eq!(alpha(&one, size - 12, 12), 0);
        assert!(alpha(&three, size - 12, 12) > 200);
        assert_eq!(alpha(&one, 114, 60), 0);
        assert!(alpha(&three, 114, 60) > 200);
    }
}
