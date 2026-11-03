//! The desktop's own light/dark preference and accent color, read from the XDG Settings portal
//! (`org.freedesktop.appearance`), which GNOME, KDE and most other desktops answer. The "System"
//! palette follows these when Serpantinum's colors are not there.
//!
//! GNOME's icon theme comes the same way, from `org.gnome.desktop.interface`, for the System icon
//! style.
//!
//! One thread asks the portal once and then listens for `SettingChanged`, so a switch in the
//! desktop's settings recolors Ski while it runs.
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use eframe::egui::Color32;
use zbus::zvariant::OwnedValue;

/// How long the first window waits for the portal's answer before it opens in the dark palette
/// and recolors once the answer comes; a running portal replies in a few milliseconds.
const FIRST_ANSWER: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Appearance {
    /// `Some(true)` for a light preference, `Some(false)` for dark, `None` when the desktop states none.
    pub light: Option<bool>,
    pub accent: Option<Color32>,
    /// The icon theme GNOME (or a desktop that answers for its settings) is set to.
    pub icon_theme: Option<String>,
}

struct State {
    current: Mutex<(bool, Appearance)>,
    ready: Condvar,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| {
        std::thread::spawn(listen);
        State { current: Mutex::new((false, Appearance::default())), ready: Condvar::new() }
    })
}

/// What the desktop asks for now. The first call waits briefly for the portal.
pub fn current() -> Appearance {
    let state = state();
    let guard = state.current.lock().unwrap_or_else(|e| e.into_inner());
    let (guard, _) = state.ready.wait_timeout_while(guard, FIRST_ANSWER, |(answered, _)| !*answered).unwrap_or_else(|e| e.into_inner());
    guard.1.clone()
}

/// Starts listening, so a later change is noticed. `wake` is called on each change for as long
/// as it returns true; each window passes its own.
pub fn watch(wake: impl Fn() -> bool + Send + 'static) {
    let _ = state();
    WAKE.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(wake));
}

type Wake = Box<dyn Fn() -> bool + Send>;
static WAKE: Mutex<Vec<Wake>> = Mutex::new(Vec::new());

fn publish(next: Appearance, first: bool) {
    let state = state();
    let mut guard = state.current.lock().unwrap_or_else(|e| e.into_inner());
    let changed = guard.1 != next;
    *guard = (true, next);
    drop(guard);
    state.ready.notify_all();
    if changed || !first {
        WAKE.lock().unwrap_or_else(|e| e.into_inner()).retain(|wake| wake());
    }
}

fn listen() {
    if run().is_err() {
        // No portal: the first window stops waiting and keeps the dark palette
        let state = state();
        state.current.lock().unwrap_or_else(|e| e.into_inner()).0 = true;
        state.ready.notify_all();
    }
}

const NAMESPACE: &str = "org.freedesktop.appearance";
const GNOME: &str = "org.gnome.desktop.interface";

fn run() -> zbus::Result<()> {
    use zbus::blocking::{Connection, Proxy};
    let connection = Connection::session()?;
    let proxy = Proxy::new(&connection, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings")?;
    // Subscribed before the first read, so nothing that changes in between is missed
    let changes = proxy.receive_signal("SettingChanged")?;
    let mut now = Appearance {
        light: read(&proxy, NAMESPACE, "color-scheme").and_then(|v| scheme(&v)).flatten(),
        accent: read(&proxy, NAMESPACE, "accent-color").and_then(|v| accent(&v)),
        icon_theme: read(&proxy, GNOME, "icon-theme").and_then(|v| text(&v)),
    };
    publish(now.clone(), true);
    for message in changes {
        let Ok((namespace, key, value)) = message.body().deserialize::<(String, String, OwnedValue)>() else { continue };
        match (namespace.as_str(), key.as_str()) {
            (NAMESPACE, "color-scheme") => now.light = scheme(&value).flatten(),
            (NAMESPACE, "accent-color") => now.accent = accent(&value),
            (GNOME, "icon-theme") => now.icon_theme = text(&value),
            _ => continue,
        }
        publish(now.clone(), false);
    }
    Ok(())
}

/// `ReadOne` where the portal has it (version 2), `Read` with its extra variant layer otherwise.
fn read(proxy: &zbus::blocking::Proxy<'_>, namespace: &str, key: &str) -> Option<OwnedValue> {
    if let Ok(value) = proxy.call::<_, _, OwnedValue>("ReadOne", &(namespace, key)) {
        return Some(value);
    }
    let value: OwnedValue = proxy.call("Read", &(namespace, key)).ok()?;
    Some(unwrap_variant(value))
}

fn unwrap_variant(value: OwnedValue) -> OwnedValue {
    match &*value {
        zbus::zvariant::Value::Value(inner) => inner.try_to_owned().map(unwrap_variant).unwrap_or(value),
        _ => value,
    }
}

/// 0 is no preference, 1 dark, 2 light. `None` when the value is not a number at all.
fn scheme(value: &OwnedValue) -> Option<Option<bool>> {
    let value = unwrap_variant(value.try_clone().ok()?);
    let n = u32::try_from(&value).ok()?;
    Some(match n {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    })
}

/// Red, green and blue from 0 to 1; anything outside that range means no accent is set.
fn accent(value: &OwnedValue) -> Option<Color32> {
    let value = unwrap_variant(value.try_clone().ok()?);
    let (r, g, b) = <(f64, f64, f64)>::try_from(value).ok()?;
    let channel = |c: f64| (0.0..=1.0).contains(&c).then(|| (c * 255.0).round() as u8);
    Some(Color32::from_rgb(channel(r)?, channel(g)?, channel(b)?))
}

fn text(value: &OwnedValue) -> Option<String> {
    let value = unwrap_variant(value.try_clone().ok()?);
    let s = String::try_from(value).ok()?;
    (!s.is_empty()).then_some(s)
}
