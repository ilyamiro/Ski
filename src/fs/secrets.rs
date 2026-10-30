//! Server passwords in the desktop's keyring, through the Secret Service D-Bus API that GNOME
//! Keyring, KWallet and KeePassXC provide. They are filed the way GVfs and Nautilus file them
//! (the `org.gnome.keyring.NetworkPassword` attributes), so a password saved by either is found
//! by the other. Every call blocks, and a locked keyring may show its unlock prompt, so this runs
//! on the connection threads only.
use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::network::Address;

const DEST: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE: &str = "org.freedesktop.Secret.Service";
const ITEM: &str = "org.freedesktop.Secret.Item";
const PROMPT: &str = "org.freedesktop.Secret.Prompt";
const SCHEMA: &str = "org.gnome.keyring.NetworkPassword";
/// Where new items go: the default collection, else the login keyring by name.
const COLLECTIONS: [&str; 2] = ["/org/freedesktop/secrets/aliases/default", "/org/freedesktop/secrets/collection/login"];

/// `(session, parameters, value, content type)`, as the Secret Service passes secrets.
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

struct Keyring {
    connection: Connection,
    session: OwnedObjectPath,
}

impl Keyring {
    /// A session with the keyring that sends secrets as they are; D-Bus does not leave this computer.
    fn open() -> zbus::Result<Keyring> {
        let connection = Connection::session()?;
        let service = Proxy::new(&connection, DEST, SERVICE_PATH, SERVICE)?;
        let (_, session): (OwnedValue, OwnedObjectPath) = service.call("OpenSession", &("plain", Value::from("")))?;
        Ok(Keyring { connection, session })
    }

    fn service(&self) -> zbus::Result<Proxy<'_>> {
        Proxy::new(&self.connection, DEST, SERVICE_PATH, SERVICE)
    }

    /// Items whose attributes include all of `attributes`, unlocking the keyring when it is locked.
    fn search(&self, attributes: &HashMap<&str, String>) -> zbus::Result<Vec<OwnedObjectPath>> {
        let service = self.service()?;
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service.call("SearchItems", &(attributes,))?;
        if !unlocked.is_empty() || locked.is_empty() {
            return Ok(unlocked);
        }
        let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = service.call("Unlock", &(&locked,))?;
        if !self.prompt(&prompt)? {
            return Ok(Vec::new());
        }
        let (unlocked, _): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service.call("SearchItems", &(attributes,))?;
        Ok(unlocked)
    }

    /// Show the keyring's own prompt (`/` means none is needed); false when it was dismissed.
    fn prompt(&self, path: &OwnedObjectPath) -> zbus::Result<bool> {
        if path.as_str() == "/" {
            return Ok(true);
        }
        let prompt = Proxy::new(&self.connection, DEST, path.as_str(), PROMPT)?;
        let mut completed = prompt.receive_signal("Completed")?;
        prompt.call::<_, _, ()>("Prompt", &("",))?;
        let Some(message) = completed.next() else { return Ok(false) };
        let (dismissed, _): (bool, OwnedValue) = message.body().deserialize()?;
        Ok(!dismissed)
    }

    fn secret(&self, item: &OwnedObjectPath) -> zbus::Result<(String, HashMap<String, String>)> {
        let proxy = Proxy::new(&self.connection, DEST, item.as_str(), ITEM)?;
        let secret: Secret = proxy.call("GetSecret", &(&self.session,))?;
        let attributes: HashMap<String, String> = proxy.get_property("Attributes")?;
        Ok((String::from_utf8_lossy(&secret.2).into_owned(), attributes))
    }
}

/// The attributes GVfs files a server's password under. `with_user` leaves the user out, to find
/// the password of whoever logged in to the server last.
fn attributes(address: &Address, with_user: bool) -> HashMap<&'static str, String> {
    let mut map = HashMap::new();
    map.insert("server", address.host.to_lowercase());
    map.insert("protocol", address.scheme.name().to_owned());
    if with_user && let Some(user) = &address.user {
        map.insert("user", user.clone());
    }
    if let Some(share) = address.share() {
        map.insert("object", share.to_owned());
    }
    if let Some(port) = address.port.filter(|p| *p != address.scheme.default_port()) {
        map.insert("port", port.to_string());
    }
    map
}

/// The saved user, domain and password for `address` (the top of its server or share).
pub fn lookup(address: &Address) -> Option<(String, String, String)> {
    let keyring = Keyring::open().ok()?;
    let mut items = keyring.search(&attributes(address, true)).ok()?;
    if items.is_empty() && address.user.is_some() {
        return None;
    }
    if items.is_empty() {
        items = keyring.search(&attributes(address, false)).ok()?;
    }
    let (password, found) = keyring.secret(items.first()?).ok()?;
    let user = found.get("user").cloned().or_else(|| address.user.clone()).unwrap_or_default();
    let domain = found.get("domain").cloned().or_else(|| address.domain.clone()).unwrap_or_default();
    Some((user, domain, password))
}

/// Save the password for `address`, replacing one saved before.
pub fn store(address: &Address, user: &str, domain: &str, password: &str) -> Result<(), String> {
    let keyring = Keyring::open().map_err(|e| e.to_string())?;
    let mut attrs = attributes(address, false);
    attrs.insert("xdg:schema", SCHEMA.to_owned());
    if !user.is_empty() {
        attrs.insert("user", user.to_owned());
    }
    if !domain.is_empty() {
        attrs.insert("domain", domain.to_owned());
    }
    let label = Address { user: Some(user.to_owned()).filter(|u| !u.is_empty()), domain: None, ..address.clone() }.display();
    let mut properties: HashMap<&str, Value> = HashMap::new();
    properties.insert("org.freedesktop.Secret.Item.Label", Value::from(label));
    properties.insert("org.freedesktop.Secret.Item.Attributes", Value::from(attrs));
    let secret = (keyring.session.clone(), Vec::<u8>::new(), password.as_bytes().to_vec(), "text/plain".to_owned());
    let mut last = String::from("no keyring");
    for collection in COLLECTIONS {
        let created = Proxy::new(&keyring.connection, DEST, collection, "org.freedesktop.Secret.Collection")
            .and_then(|c| c.call::<_, _, (OwnedObjectPath, OwnedObjectPath)>("CreateItem", &(&properties, &secret, true)));
        match created {
            Ok((item, prompt)) if item.as_str() != "/" || keyring.prompt(&prompt).unwrap_or(false) => return Ok(()),
            Ok(_) => last = "dismissed".to_owned(),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// Remove every password saved for `address`.
pub fn delete(address: &Address) -> Result<(), String> {
    let keyring = Keyring::open().map_err(|e| e.to_string())?;
    let items = keyring.search(&attributes(address, true)).map_err(|e| e.to_string())?;
    for item in items {
        let proxy = Proxy::new(&keyring.connection, DEST, item.as_str(), ITEM).map_err(|e| e.to_string())?;
        let prompt: OwnedObjectPath = proxy.call("Delete", &()).map_err(|e| e.to_string())?;
        keyring.prompt(&prompt).map_err(|e| e.to_string())?;
    }
    Ok(())
}
