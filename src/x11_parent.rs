//! The X11 counterpart of `wayland_parent.rs`: the portal names the requester's window as
//! `x11:<hex window id>`, and the dialog is marked as transient for it (`WM_TRANSIENT_FOR`) so the
//! window manager keeps it above that window and centers it there.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

/// Marks `window` as a child of the window `handle` names; false when this is not an X11 window
/// or the handle is of another kind.
pub fn attach<W: HasWindowHandle + HasDisplayHandle>(window: &W, handle: &str) -> bool {
    let Some(parent) = handle.strip_prefix("x11:").and_then(|id| u64::from_str_radix(id.trim_start_matches("0x"), 16).ok()) else { return false };
    let (Ok(display), Ok(own)) = (window.display_handle(), window.window_handle()) else { return false };
    let (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(own)) = (display.as_raw(), own.as_raw()) else { return false };
    let Some(display) = display.display else { return false };
    let Ok(xlib) = x11_dl::xlib::Xlib::open() else { return false };
    // SAFETY: the display and window come from the live window winit created, which outlives this
    // call; the calls only read them and write one property.
    unsafe {
        (xlib.XSetTransientForHint)(display.as_ptr().cast(), own.window, parent);
        (xlib.XFlush)(display.as_ptr().cast());
    }
    true
}
