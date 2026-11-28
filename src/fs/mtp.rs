//! Phones and media players over MTP: they have no block device, so `lsblk` never lists them, and
//! nothing in a plain Wayland session mounts them either. GVfs knows them, so ask its MTP volume
//! monitor what is attached and mount what was just plugged in; the mount then shows up under
//! `$XDG_RUNTIME_DIR/gvfs`, where the Network section of the sidebar already lists it.
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

use super::network::{decode, gvfs_root};

/// The volume monitor GVfs loads for MTP devices, and the object every remote monitor answers on.
const MONITOR: &str = "org.gtk.vfs.MTPVolumeMonitor";
const OBJECT: &str = "/org/gtk/Private/RemoteVolumeMonitor";
const INTERFACE: &str = "org.gtk.Private.RemoteVolumeMonitor";

/// The phones that were attached at the last look, so one the user unmounted is left alone until
/// it is plugged in again.
static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Mount the phones that were just plugged in and are not mounted yet.
pub fn mount_new() {
    let Some(attached) = attached() else { return };
    let Ok(mut seen) = SEEN.lock() else { return };
    for uri in &attached {
        let plugged_in = seen.iter().any(|old| old == uri);
        if !plugged_in && local_path(uri).is_none() {
            // A phone that refuses to mount is not asked again until it is plugged in again
            let _ = gio(&["mount", uri]);
        }
    }
    seen.clone_from(&attached);
}

/// The addresses of the phones GVfs sees: `mtp://Xiaomi_Redmi_12_5G_52adac63b325/`. `None` when
/// the monitor does not answer, which is what happens without `gvfs-mtp` or a session D-Bus.
fn attached() -> Option<Vec<String>> {
    let connection = Connection::session().ok()?;
    let proxy = zbus::blocking::proxy::Builder::<Proxy>::new(&connection)
        .destination(MONITOR)
        .ok()?
        .path(OBJECT)
        .ok()?
        .interface(INTERFACE)
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok()?;
    let listing: Listing = proxy.call("List", &()).ok()?;
    let uris = listing.volumes.iter().filter_map(volume_uri).collect();
    Some(uris)
}

/// The monitor's `List()` reply, as GVfs serializes it:
/// `(a(ssssbbbbbbbbuasa{ss}sa{sv}) a(ssssssbbssa{ss}sa{sv}) a(ssssssbsassa{sv}))`.
#[derive(serde::Deserialize, zbus::zvariant::Type)]
struct Listing {
    /// Neither drives nor mounts are read here, but a reply is read whole or not at all.
    #[allow(dead_code)]
    drives: Vec<Reply17>,
    volumes: Vec<Reply13>,
    #[allow(dead_code)]
    mounts: Vec<Reply11>,
}

#[derive(serde::Deserialize, zbus::zvariant::Type)]
#[allow(dead_code)]
struct Reply17(String, String, String, String, bool, bool, bool, bool, bool, bool, bool, bool, u32, Vec<String>, HashMap<String, String>, String, HashMap<String, OwnedValue>);

/// A volume: `(id, name, icon, icons, uuid, activation_uri, can_mount, should_automount, …,
/// attributes)`. The address is looked for among the strings, so a field GVfs moves costs nothing.
#[derive(serde::Deserialize, zbus::zvariant::Type)]
#[allow(dead_code)]
struct Reply13(String, String, String, String, String, String, bool, bool, String, String, HashMap<String, String>, String, HashMap<String, OwnedValue>);

#[derive(serde::Deserialize, zbus::zvariant::Type)]
#[allow(dead_code)]
struct Reply11(String, String, String, String, String, String, bool, String, Vec<String>, String, HashMap<String, OwnedValue>);

fn volume_uri(volume: &Reply13) -> Option<String> {
    let strings = [&volume.0, &volume.1, &volume.2, &volume.3, &volume.4, &volume.5];
    strings.into_iter().find(|text| text.starts_with("mtp://")).cloned()
}

/// The folder GVfs mounts `uri` at, while it is mounted.
fn local_path(uri: &str) -> Option<PathBuf> {
    let host = host_of(uri)?;
    let entries = std::fs::read_dir(gvfs_root()).ok()?;
    entries.flatten().map(|entry| entry.path()).find(|path| {
        // GVfs names the folder after the mount spec: `mtp:host=Redmi_12_5G_AB12CD34`
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("mtp:host="))
            .is_some_and(|name| decode(name) == host)
    })
}

/// The host of `mtp://Xiaomi_Redmi_12_5G_52adac63b325/`, which is the phone's model and serial.
fn host_of(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("mtp://")?;
    let host = rest.split('/').next().filter(|host| !host.is_empty())?;
    Some(decode(host))
}

/// One run of `gio`, which mounts and unmounts. Phones ask no questions, so its input stays closed.
fn gio(args: &[&str]) -> Result<(), String> {
    let out = Command::new("gio")
        .args(args)
        // Messages are read, not shown
        .env("LC_ALL", "C.UTF-8")
        .env_remove("LANGUAGE")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("gio: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(said.strip_prefix("gio: ").unwrap_or(&said).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(uri: &str) -> Reply13 {
        Reply13(String::new(), "Redmi 12 5G".into(), "phone".into(), String::new(), String::new(), uri.into(), true, true, String::new(), String::new(), HashMap::new(), String::new(), HashMap::new())
    }

    #[test]
    fn the_address_of_a_volume_is_the_one_that_starts_with_mtp() {
        assert_eq!(volume_uri(&volume("mtp://Xiaomi_Redmi_12_5G_52adac63b325/")).as_deref(), Some("mtp://Xiaomi_Redmi_12_5G_52adac63b325/"));
    }

    #[test]
    fn a_volume_without_an_address_is_not_a_phone() {
        assert_eq!(volume_uri(&volume("")), None);
    }

    #[test]
    fn addresses_give_the_host_gvfs_names_the_folder_after() {
        assert_eq!(host_of("mtp://Xiaomi_Redmi_12_5G_52adac63b325/").as_deref(), Some("Xiaomi_Redmi_12_5G_52adac63b325"));
        assert_eq!(host_of("mtp://My%20Phone_AB12CD34/").as_deref(), Some("My Phone_AB12CD34"));
        assert_eq!(host_of("sftp://example.com/"), None);
    }
}
