//! Ties the chooser window to the window of the program that asked for it, the way GTK's own
//! dialogs do: the requester exports its window as an xdg-foreign handle, the portal passes the
//! handle along (`parent_window`, `wayland:<handle>`), and this imports it and makes it the
//! parent of our window. Compositors then float the dialog, center it on its parent and keep it
//! above it. winit cannot do this on Wayland, so it talks to the compositor directly, on its own
//! event queue of the window's display.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry::WlRegistry, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_imported_v2::{self, ZxdgImportedV2};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_importer_v2::ZxdgImporterV2;

/// Keeps the relation alive: destroying the imported object would undo it.
pub struct Parent {
    _connection: Connection,
    _queue: EventQueue<State>,
    _imported: ZxdgImportedV2,
}

struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZxdgImporterV2, ()> for State {
    fn event(_: &mut Self, _: &ZxdgImporterV2, _: <ZxdgImporterV2 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZxdgImportedV2, ()> for State {
    fn event(_: &mut Self, _: &ZxdgImportedV2, event: zxdg_imported_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // `Destroyed`: the parent window is gone; the compositor already dropped the relation
        let _ = event;
    }
}

/// Makes `window` a child of the exported window `handle` (`wayland:<handle>` as the portal
/// gives it). `None` when this is not a Wayland window, the handle is of another kind, or the
/// compositor lacks xdg-foreign.
pub fn attach<W: HasWindowHandle + HasDisplayHandle>(window: &W, handle: &str) -> Option<Parent> {
    let handle = handle.strip_prefix("wayland:")?;
    let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else { return None };
    let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else { return None };

    // SAFETY: both pointers come from the live window winit created, which outlives this call;
    // the display stays owned by winit (a foreign display is never disconnected by us).
    let connection = Connection::from_backend(unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) });
    let surface = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }.ok()?;
    let surface = WlSurface::from_id(&connection, surface).ok()?;

    let (globals, queue) = registry_queue_init::<State>(&connection).ok()?;
    let importer: ZxdgImporterV2 = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
    let imported = importer.import_toplevel(handle.to_owned(), &queue.handle(), ());
    imported.set_parent_of(&surface);
    connection.flush().ok()?;
    importer.destroy();
    connection.flush().ok()?;
    Some(Parent { _connection: connection, _queue: queue, _imported: imported })
}
