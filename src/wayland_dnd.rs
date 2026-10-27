//! Drag and drop between windows on Wayland. winit offers neither side of it there, so this talks
//! to the compositor itself, on its own event queue of the window's display: it starts a drag
//! (`wl_data_device.start_drag`) when files are carried out of the window, and takes part as a
//! target for drags from any window, Ski's own or another program's. Files travel as
//! `text/uri-list`, like in Nautilus. The queue is dispatched from the frame loop (`poll`); a
//! small thread only wakes the loop when the compositor has something to say.

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
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child};

const URI_LIST: &str = "text/uri-list";
const PLAIN_TEXT: &str = "text/plain;charset=utf-8";
/// Offered only by Ski, so a drag that comes back to it is known without reading the files.
const OWN_MIME: &str = "application/x-ski-drag";
const ICON_SIZE: i32 = 40;

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
    _pointer: WlPointer,
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
        let pointer = seat.get_pointer(&qh, ());
        connection.flush().ok()?;

        let state = State { surface: Some(surface_id), ..State::default() };
        let this = Self { connection, queue, state, surface, compositor, shm, manager, device, _seat: seat, _pointer: pointer };
        this.spawn_waker(ctx);
        Some(this)
    }

    /// Wakes the frame loop when the compositor sends something, since a drag over the window
    /// produces no events for egui.
    fn spawn_waker(&self, ctx: &eframe::egui::Context) {
        let fd: RawFd = self.connection.backend().poll_fd().as_raw_fd();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            loop {
                let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                // SAFETY: one valid pollfd; the display outlives the process's windows
                let n = unsafe { libc::poll(&mut pfd, 1, -1) };
                if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    return;
                }
                ctx.request_repaint();
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        });
    }

    /// Takes in what the compositor has sent and reports the state of a drag over the window.
    pub fn poll(&mut self) -> Poll {
        if let Some(guard) = self.queue.prepare_read() {
            let _ = guard.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.state);

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

        let icon = self.drag_icon(&qh, paths.len(), style);
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

    /// A rounded square carrying the number of files, drawn into a shared-memory buffer.
    fn drag_icon(&self, qh: &QueueHandle<State>, count: usize, style: IconStyle) -> Option<(WlSurface, WlBuffer, WlShmPool)> {
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

/// Colors (`0xAARRGGBB`) of the drag icon, and the buffer scale of the output it appears on.
#[derive(Clone, Copy)]
pub struct IconStyle {
    pub background: u32,
    pub text: u32,
    pub scale: i32,
}

/// Premultiplied Argb8888 pixels: a rounded square with the file count centered in it.
fn render_icon(size: usize, scale: f32, count: usize, style: IconStyle) -> Vec<u8> {
    use ab_glyph::{Font, FontRef, ScaleFont};
    let channels = |argb: u32| ((argb >> 24) as f32, (argb >> 16 & 0xff) as f32, (argb >> 8 & 0xff) as f32, (argb & 0xff) as f32);
    let (ba, br, bg, bb) = channels(style.background);
    let (_, tr, tg, tb) = channels(style.text);
    let radius = 10.0 * scale;

    // Straight-alpha color per pixel: the square first, the text over it
    let mut rgba = vec![[br, bg, bb, 0.0f32]; size * size];
    for y in 0..size {
        for x in 0..size {
            let dx = (radius - x.min(size - 1 - x) as f32 - 0.5).max(0.0);
            let dy = (radius - y.min(size - 1 - y) as f32 - 0.5).max(0.0);
            rgba[y * size + x][3] = ba * (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
        }
    }

    let label = if count > 99 { "99+".to_owned() } else { count.to_string() };
    if let Ok(font) = FontRef::try_from_slice(include_bytes!("../assets/fonts/AdwaitaSans-Regular.ttf")) {
        // Long numbers get smaller so that they stay inside the square
        let px = (if label.len() > 2 { 15.0 } else if label.len() == 2 { 20.0 } else { 24.0 }) * scale;
        let scaled = font.as_scaled(ab_glyph::PxScale::from(px));
        let width: f32 = label.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum();
        // Digits are as tall as the capital height, which is what is centered vertically
        let cap = font.glyph_id('0');
        let cap_height = font.outline_glyph(cap.with_scale(scaled.scale())).map_or(px * 0.7, |g| g.px_bounds().height());
        let baseline = size as f32 / 2.0 + cap_height / 2.0;
        let mut pen = (size as f32 - width) / 2.0;
        for c in label.chars() {
            let id = scaled.glyph_id(c);
            let glyph = id.with_scale_and_position(scaled.scale(), ab_glyph::point(pen, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, cover| {
                    let (x, y) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
                    if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
                        return;
                    }
                    let px = &mut rgba[y as usize * size + x as usize];
                    // Text over the square: coverage mixes the colors, and the square's alpha rises with it
                    let mix = cover.clamp(0.0, 1.0);
                    px[0] += (tr - px[0]) * mix;
                    px[1] += (tg - px[1]) * mix;
                    px[2] += (tb - px[2]) * mix;
                    px[3] += (255.0 - px[3]) * mix * (px[3] / ba.max(1.0));
                });
            }
            pen += scaled.h_advance(id);
        }
    }

    let mut out = Vec::with_capacity(size * size * 4);
    for [r, g, b, a] in rgba {
        // Premultiplied, as Argb8888 wants it
        for c in [b, g, r] {
            out.push((c * a / 255.0 + 0.5) as u8);
        }
        out.push((a + 0.5) as u8);
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
ignore_events!(WlSeat, WlDataDeviceManager, WlCompositor, WlShm, WlShmPool, WlBuffer, WlSurface);

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
                let own = info.mimes.iter().any(|m| m == OWN_MIME);
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
    fn icon_shows_the_count() {
        let style = IconStyle { background: 0xE6_C0_A0_FF, text: 0xFF_1C_1B_22, scale: 2 };
        let plain = render_icon(80, 2.0, 0, style);
        let three = render_icon(80, 2.0, 3, style);
        assert_eq!(three.len(), 80 * 80 * 4);
        assert_ne!(plain, three);
        // The corner stays transparent, the middle is covered
        assert_eq!(three[3], 0);
        assert!(three[(40 * 80 + 4) * 4 + 3] > 200);
    }
}
