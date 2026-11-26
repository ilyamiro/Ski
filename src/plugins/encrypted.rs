//! Encrypted drives: LUKS and BitLocker volumes in the sidebar. A click on a locked one asks for
//! its passphrase, unlocks it and opens what it holds; the lock button unmounts and locks it again.
//! Volumes are listed with `lsblk`. Unlocking, mounting and locking go through UDisks2 on D-Bus, so
//! the passphrase is never on a command line, and polkit asks for the admin password when the
//! system wants one (an internal disk, say).
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::Value;

use super::{Plugin, Reaction, SidebarEntry};
use crate::fs::drives::{MOUNT_ROOTS, mount_points};
use crate::i18n::{tr, trf};
use crate::ui::icons;
use crate::ui::material::{self, FooterSpec};
use crate::ui::theme::SerpantinumTheme;

/// How often the list of volumes is read again, to see drives plugged in or unlocked elsewhere.
const REFRESH: Duration = Duration::from_secs(4);

/// The `lsblk` filesystem types of encrypted containers UDisks2 can unlock.
const ENCRYPTED: &[&str] = &["crypto_LUKS", "BitLocker"];

/// An encrypted container the user can unlock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// `/dev/sdb1`, the encrypted device itself.
    pub dev: String,
    pub name: String,
    pub unlocked: bool,
    /// Where what it holds is mounted.
    pub mount: Option<PathBuf>,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    node.get("children").and_then(Value::as_array).into_iter().flatten()
}

/// The system itself uses something below `node`: it is swap or mounted outside the usual mount
/// folders (`/`, `/home`). Such a volume was unlocked at boot and is not one to lock from here.
fn used_by_system(node: &Value) -> bool {
    text(node, "fstype") == Some("swap")
        || mount_points(node).iter().any(|m| !MOUNT_ROOTS.iter().any(|r| Path::new(m).starts_with(r)))
        || children(node).any(used_by_system)
}

/// The first folder something below `node` is mounted at.
fn first_mount(node: &Value) -> Option<PathBuf> {
    mount_points(node).first().map(PathBuf::from).or_else(|| children(node).find_map(first_mount))
}

fn walk(node: &Value, out: &mut Vec<Volume>) {
    if !text(node, "fstype").is_some_and(|f| ENCRYPTED.contains(&f)) {
        children(node).for_each(|c| walk(c, out));
        return;
    }
    let Some(dev) = text(node, "path") else { return };
    if used_by_system(node) {
        return;
    }
    // The container's own label, else that of the filesystem inside once it is unlocked
    let label = text(node, "label").or_else(|| children(node).find_map(|c| text(c, "label")));
    let name = label.map(str::to_owned).unwrap_or_else(|| trf("{} Encrypted Volume", &[&text(node, "size").unwrap_or("?")]));
    out.push(Volume { dev: dev.to_owned(), name, unlocked: children(node).next().is_some(), mount: first_mount(node) });
}

/// The encrypted volumes in `lsblk`'s JSON, but for those the system runs on.
pub fn parse_lsblk(json: &str) -> Vec<Volume> {
    let mut out = Vec::new();
    if let Ok(root) = serde_json::from_str::<Value>(json) {
        for node in root["blockdevices"].as_array().into_iter().flatten() {
            walk(node, &mut out);
        }
    }
    // A volume on RAID or multipath shows up under each of its disks
    let mut seen = HashSet::new();
    out.retain(|v| seen.insert(v.dev.clone()));
    out.sort_by_key(|v| v.name.to_lowercase());
    out
}

fn list() -> Vec<Volume> {
    // `mountpoints` is missing from old `lsblk`, which then refuses the whole list
    const COLUMNS: [&str; 2] = ["NAME,PATH,LABEL,SIZE,MOUNTPOINT,MOUNTPOINTS,TYPE,FSTYPE", "NAME,PATH,LABEL,SIZE,MOUNTPOINT,TYPE,FSTYPE"];
    for columns in COLUMNS {
        if let Ok(o) = Command::new("lsblk").args(["-J", "-o", columns]).output()
            && o.status.success()
        {
            return parse_lsblk(&String::from_utf8_lossy(&o.stdout));
        }
    }
    Vec::new()
}

/// The calls to UDisks2. Each one blocks, and polkit may show its password prompt, so they run on
/// threads of their own.
mod udisks {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, Value};

    const DEST: &str = "org.freedesktop.UDisks2";
    const MANAGER: &str = "/org/freedesktop/UDisks2/Manager";
    const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";
    const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";

    type Options<'a> = HashMap<&'a str, Value<'a>>;

    /// What the user can read of a failure: the message UDisks2 sent, without the error's name.
    fn describe(err: zbus::Error) -> String {
        match err {
            zbus::Error::MethodError(name, message, _) => message.unwrap_or_else(|| name.to_string()),
            other => other.to_string(),
        }
    }

    /// Properties are read fresh each time: they change as the volume is unlocked and mounted.
    fn proxy<'a>(c: &Connection, path: &'a str, interface: &'a str) -> zbus::Result<Proxy<'a>> {
        zbus::blocking::proxy::Builder::<Proxy>::new(c).destination(DEST)?.path(path)?.interface(interface)?.cache_properties(zbus::proxy::CacheProperties::No).build()
    }

    /// The UDisks2 object of the block device `dev`.
    fn resolve(c: &Connection, dev: &str) -> zbus::Result<OwnedObjectPath> {
        let manager = proxy(c, MANAGER, "org.freedesktop.UDisks2.Manager")?;
        let spec: Options = HashMap::from([("path", Value::from(dev))]);
        let found: Vec<OwnedObjectPath> = manager.call("ResolveDevice", &(spec, Options::new()))?;
        found.into_iter().next().ok_or_else(|| zbus::Error::Failure(format!("{dev} is not known to UDisks2")))
    }

    /// The unlocked device inside the container `block`, if it is unlocked.
    fn cleartext(c: &Connection, block: &str) -> zbus::Result<Option<OwnedObjectPath>> {
        let path: OwnedObjectPath = proxy(c, block, ENCRYPTED)?.get_property("CleartextDevice")?;
        Ok((path.as_str() != "/").then_some(path))
    }

    fn mount_points(fs: &Proxy) -> zbus::Result<Vec<PathBuf>> {
        let raw: Vec<Vec<u8>> = fs.get_property("MountPoints")?;
        Ok(raw.into_iter().map(|bytes| PathBuf::from(String::from_utf8_lossy(bytes.strip_suffix(&[0]).unwrap_or(&bytes)).into_owned())).collect())
    }

    /// Mounts the filesystem `cleartext`, or finds where it already is.
    fn mount_cleartext(c: &Connection, cleartext: &str) -> zbus::Result<PathBuf> {
        let mut tries = 0;
        loop {
            let fs = proxy(c, cleartext, FILESYSTEM)?;
            match mount_points(&fs) {
                Ok(points) if !points.is_empty() => return Ok(points[0].clone()),
                Ok(_) => return fs.call::<_, _, String>("Mount", &(Options::new(),)).map(PathBuf::from),
                // A device just unlocked is probed before it shows its filesystem
                Err(_) if tries < 10 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(_) => return Err(zbus::Error::Failure(crate::i18n::tr("It holds no filesystem that can be mounted").to_owned())),
            }
        }
    }

    /// Unlocks `dev` with `passphrase`. A wrong passphrase is an error; the volume stays locked.
    pub fn unlock(dev: &str, passphrase: &str) -> Result<(), String> {
        let c = Connection::system().map_err(describe)?;
        let block = resolve(&c, dev).map_err(describe)?;
        if cleartext(&c, &block).map_err(describe)?.is_none() {
            proxy(&c, &block, ENCRYPTED).and_then(|p| p.call::<_, _, OwnedObjectPath>("Unlock", &(passphrase, Options::new()))).map_err(describe)?;
        }
        Ok(())
    }

    /// Mounts what the unlocked `dev` holds; the folder it is at.
    pub fn mount(dev: &str) -> Result<PathBuf, String> {
        let c = Connection::system().map_err(describe)?;
        let block = resolve(&c, dev).map_err(describe)?;
        let inner = cleartext(&c, &block).map_err(describe)?.ok_or_else(|| crate::i18n::tr("The drive is locked").to_owned())?;
        mount_cleartext(&c, &inner).map_err(describe)
    }

    /// Unmounts what `dev` holds and locks it.
    pub fn lock(dev: &str) -> Result<(), String> {
        let c = Connection::system().map_err(describe)?;
        let block = resolve(&c, dev).map_err(describe)?;
        if let Some(inner) = cleartext(&c, &block).map_err(describe)?
            && let Ok(fs) = proxy(&c, &inner, FILESYSTEM)
            && mount_points(&fs).is_ok_and(|p| !p.is_empty())
        {
            fs.call::<_, _, ()>("Unmount", &(Options::new(),)).map_err(describe)?;
        }
        proxy(&c, &block, ENCRYPTED).and_then(|p| p.call::<_, _, ()>("Lock", &(Options::new(),))).map_err(describe)
    }
}

enum Msg {
    Listed(Vec<Volume>),
    /// The passphrase was refused or the drive could not be unlocked; the dialog stays open.
    UnlockFailed { dev: String, name: String, err: String },
    /// Unlocked (or already so) and then mounted, or not.
    Mounted { dev: String, name: String, result: Result<PathBuf, String> },
    Locked { dev: String, name: String, mount: Option<PathBuf>, result: Result<(), String> },
}

/// The passphrase dialog.
struct Prompt {
    dev: String,
    name: String,
    passphrase: String,
    error: Option<String>,
    working: bool,
    first_frame: bool,
}

pub struct Encrypted {
    volumes: Vec<Volume>,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    listed_at: Option<Instant>,
    listing: bool,
    /// Volumes being unlocked, mounted or locked.
    working: HashSet<String>,
    prompt: Option<Prompt>,
    /// The folder the listing shows, to leave a volume before it is locked.
    dir: Option<PathBuf>,
    reaction: Option<Reaction>,
    messages: Vec<String>,
}

impl Default for Encrypted {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { volumes: Vec::new(), tx, rx, listed_at: None, listing: false, working: HashSet::new(), prompt: None, dir: None, reaction: None, messages: Vec::new() }
    }
}

impl Encrypted {
    fn spawn(&mut self, dev: &str, work: impl FnOnce() -> Msg + Send + 'static) {
        self.working.insert(dev.to_owned());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }

    fn start_unlock(&mut self, prompt: &mut Prompt) {
        prompt.working = true;
        prompt.error = None;
        let (dev, name, passphrase) = (prompt.dev.clone(), prompt.name.clone(), std::mem::take(&mut prompt.passphrase));
        self.spawn(&prompt.dev, move || match udisks::unlock(&dev, &passphrase) {
            Ok(()) => Msg::Mounted { result: udisks::mount(&dev), dev, name },
            Err(err) => Msg::UnlockFailed { dev, name, err },
        });
    }

    fn start_mount(&mut self, volume: &Volume) {
        let (dev, name) = (volume.dev.clone(), volume.name.clone());
        self.spawn(&volume.dev, move || Msg::Mounted { result: udisks::mount(&dev), dev, name });
    }

    fn start_lock(&mut self, volume: &Volume) {
        let (dev, name, mount) = (volume.dev.clone(), volume.name.clone(), volume.mount.clone());
        self.spawn(&volume.dev, move || Msg::Locked { result: udisks::lock(&dev), dev, name, mount });
    }
}

impl Plugin for Encrypted {
    fn id(&self) -> &'static str {
        "encrypted"
    }

    fn name(&self) -> &'static str {
        tr("Encrypted Drives")
    }

    fn description(&self) -> &'static str {
        tr("Lists LUKS and BitLocker drives in the sidebar, unlocks them with their passphrase and mounts them")
    }

    fn icon(&self) -> &'static str {
        icons::LOCK
    }

    fn directory_changed(&mut self, dir: &Path) {
        self.dir = Some(dir.to_path_buf());
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Listed(volumes) => {
                    self.listing = false;
                    changed = volumes != self.volumes;
                    self.volumes = volumes;
                }
                Msg::UnlockFailed { dev, name, err } => {
                    self.working.remove(&dev);
                    match self.prompt.as_mut().filter(|p| p.dev == dev) {
                        Some(prompt) => {
                            prompt.working = false;
                            prompt.first_frame = true;
                            prompt.error = Some(err);
                        }
                        None => self.messages.push(trf("Could not unlock “{}”: {}", &[&name, &err])),
                    }
                }
                Msg::Mounted { dev, name, result } => {
                    self.working.remove(&dev);
                    if self.prompt.as_ref().is_some_and(|p| p.dev == dev) {
                        self.prompt = None;
                    }
                    match result {
                        Ok(path) => self.reaction = Some(Reaction::Open(path)),
                        Err(err) => self.messages.push(trf("Could not mount “{}”: {}", &[&name, &err])),
                    }
                    self.listed_at = None;
                }
                Msg::Locked { dev, name, mount, result } => {
                    self.working.remove(&dev);
                    match result {
                        Ok(()) => {
                            self.messages.push(trf("Locked “{}”", &[&name]));
                            // The listing showed a folder of the volume, which is gone now
                            if let (Some(mount), Some(dir)) = (&mount, &self.dir)
                                && dir.starts_with(mount)
                            {
                                self.reaction = Some(Reaction::Open(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))));
                            }
                        }
                        Err(err) => self.messages.push(trf("Could not lock “{}”: {}", &[&name, &err])),
                    }
                    self.listed_at = None;
                }
            }
        }
        if !self.listing && self.listed_at.is_none_or(|t| t.elapsed() >= REFRESH) {
            self.listing = true;
            self.listed_at = Some(Instant::now());
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Listed(list()));
            });
        }
        changed
    }

    fn busy(&self) -> bool {
        !self.working.is_empty()
    }

    fn reaction(&mut self) -> Reaction {
        self.reaction.take().unwrap_or_default()
    }

    fn sidebar(&self) -> Vec<SidebarEntry> {
        self.volumes
            .iter()
            .map(|v| {
                let working = self.working.contains(&v.dev);
                SidebarEntry {
                    id: v.dev.clone(),
                    label: if working { format!("{} …", v.name) } else { v.name.clone() },
                    icon: if v.unlocked { icons::LOCK_OPEN } else { icons::LOCK },
                    button: (v.unlocked && !working).then(|| (icons::LOCK, tr("Lock"))),
                }
            })
            .collect()
    }

    fn sidebar_click(&mut self, id: &str) -> Reaction {
        let Some(volume) = self.volumes.iter().find(|v| v.dev == id).cloned() else { return Reaction::None };
        if self.working.contains(&volume.dev) {
            return Reaction::None;
        }
        match (&volume.mount, volume.unlocked) {
            (Some(mount), _) => return Reaction::Open(mount.clone()),
            (None, true) => self.start_mount(&volume),
            (None, false) => self.prompt = Some(Prompt { dev: volume.dev, name: volume.name, passphrase: String::new(), error: None, working: false, first_frame: true }),
        }
        Reaction::None
    }

    fn sidebar_edit(&mut self, id: &str) {
        if let Some(volume) = self.volumes.iter().find(|v| v.dev == id && v.unlocked).cloned()
            && !self.working.contains(&volume.dev)
        {
            self.start_lock(&volume);
        }
    }

    fn window_title(&self) -> Option<&'static str> {
        self.prompt.as_ref().map(|_| tr("Unlock Drive"))
    }

    fn window_width(&self) -> f32 {
        420.0
    }

    fn window_subtitle(&self) -> Option<String> {
        self.prompt.as_ref().map(|p| format!("{} · {}", p.name, p.dev))
    }

    fn close_window(&mut self) {
        self.prompt = None;
    }

    fn window(&mut self, ui: &mut egui::Ui, theme: &SerpantinumTheme) -> bool {
        let Some(mut prompt) = self.prompt.take() else { return false };
        let mut submit = false;
        ui.label(egui::RichText::new(tr("This drive is encrypted. Enter its passphrase to unlock it.")).size(material::FONT_BODY).color(theme.subtext0));
        ui.add_space(4.0);
        material::input_frame(theme).show(ui, |ui| {
            let field = egui::TextEdit::singleline(&mut prompt.passphrase)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text(tr("Passphrase"))
                .password(true)
                .interactive(!prompt.working)
                .font(egui::FontId::proportional(material::FONT_SUBTITLE));
            let response = ui.add(field);
            if std::mem::take(&mut prompt.first_frame) {
                response.request_focus();
            }
            submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
        if let Some(err) = &prompt.error {
            ui.label(egui::RichText::new(trf("Could not unlock: {}", &[err])).size(material::FONT_CAPTION).color(theme.red));
        }
        let ready = !prompt.passphrase.is_empty() && !prompt.working;
        let label = if prompt.working { tr("Unlocking…") } else { tr("Unlock") };
        let footer = material::dialog_footer(ui, theme, FooterSpec { enabled: ready, ..FooterSpec::new("encrypted_unlock", icons::LOCK_OPEN, label) });
        if footer.cancel {
            return false;
        }
        if (footer.primary || submit) && ready {
            self.start_unlock(&mut prompt);
        }
        self.prompt = Some(prompt);
        true
    }

    fn messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"blockdevices":[
      {"name":"nvme0n1","path":"/dev/nvme0n1","label":null,"size":"500G","mountpoint":null,"mountpoints":[null],"type":"disk","fstype":null,"children":[
        {"name":"nvme0n1p1","path":"/dev/nvme0n1p1","label":null,"size":"1G","mountpoint":"/boot","mountpoints":["/boot"],"type":"part","fstype":"vfat"},
        {"name":"nvme0n1p2","path":"/dev/nvme0n1p2","label":null,"size":"499G","mountpoint":null,"mountpoints":[null],"type":"part","fstype":"crypto_LUKS","children":[
          {"name":"cryptroot","path":"/dev/mapper/cryptroot","label":null,"size":"499G","mountpoint":"/","mountpoints":["/nix/store","/"],"type":"crypt","fstype":"ext4"}]}]},
      {"name":"sda","path":"/dev/sda","label":null,"size":"64G","mountpoint":null,"mountpoints":[null],"type":"disk","fstype":null,"children":[
        {"name":"sda1","path":"/dev/sda1","label":null,"size":"32G","mountpoint":null,"mountpoints":[null],"type":"part","fstype":"crypto_LUKS"},
        {"name":"sda2","path":"/dev/sda2","label":"Vault","size":"32G","mountpoint":null,"mountpoints":[null],"type":"part","fstype":"crypto_LUKS","children":[
          {"name":"luks-1234","path":"/dev/mapper/luks-1234","label":"Photos","size":"32G","mountpoint":"/run/media/me/Photos","mountpoints":["/run/media/me/Photos"],"type":"crypt","fstype":"ext4"}]}]},
      {"name":"sdb","path":"/dev/sdb","label":null,"size":"16G","mountpoint":null,"mountpoints":[null],"type":"disk","fstype":null,"children":[
        {"name":"sdb1","path":"/dev/sdb1","label":"WIN","size":"16G","mountpoint":null,"mountpoints":[null],"type":"part","fstype":"BitLocker"},
        {"name":"sdb2","path":"/dev/sdb2","label":null,"size":"2G","mountpoint":null,"mountpoints":[null],"type":"part","fstype":"crypto_LUKS","children":[
          {"name":"cryptswap","path":"/dev/mapper/cryptswap","label":null,"size":"2G","mountpoint":"[SWAP]","mountpoints":["[SWAP]"],"type":"crypt","fstype":"swap"}]}]}]}"#;

    #[test]
    fn lists_encrypted_volumes_but_not_those_the_system_runs_on() {
        let volumes = parse_lsblk(SAMPLE);
        let devs: Vec<&str> = volumes.iter().map(|v| v.dev.as_str()).collect();
        assert_eq!(devs, vec!["/dev/sda1", "/dev/sda2", "/dev/sdb1"]);
    }

    #[test]
    fn knows_which_are_unlocked_and_where_they_are_mounted() {
        let volumes = parse_lsblk(SAMPLE);
        let locked = volumes.iter().find(|v| v.dev == "/dev/sda1").unwrap();
        assert_eq!((locked.name.as_str(), locked.unlocked, &locked.mount), ("32G Encrypted Volume", false, &None));
        let open = volumes.iter().find(|v| v.dev == "/dev/sda2").unwrap();
        assert_eq!((open.name.as_str(), open.unlocked), ("Vault", true));
        assert_eq!(open.mount, Some(PathBuf::from("/run/media/me/Photos")));
        let bitlocker = volumes.iter().find(|v| v.dev == "/dev/sdb1").unwrap();
        assert_eq!((bitlocker.name.as_str(), bitlocker.unlocked), ("WIN", false));
    }

    #[test]
    fn an_unlabelled_container_takes_the_name_of_what_it_holds() {
        let json = r#"{"blockdevices":[{"name":"sdc1","path":"/dev/sdc1","label":null,"size":"8G","mountpoint":null,"type":"part","fstype":"crypto_LUKS","children":[
          {"name":"luks-9","path":"/dev/mapper/luks-9","label":"Backup","size":"8G","mountpoint":null,"type":"crypt","fstype":"ext4"}]}]}"#;
        let volumes = parse_lsblk(json);
        assert_eq!(volumes.len(), 1);
        assert_eq!((volumes[0].name.as_str(), volumes[0].unlocked, &volumes[0].mount), ("Backup", true, &None));
    }

    #[test]
    fn a_volume_on_raid_is_listed_once() {
        let member = |disk: &str| format!(r#"{{"name":"{disk}","path":"/dev/{disk}","label":null,"size":"1T","mountpoint":null,"type":"disk","fstype":"linux_raid_member","children":[
          {{"name":"md0","path":"/dev/md0","label":null,"size":"1T","mountpoint":null,"type":"raid1","fstype":"crypto_LUKS"}}]}}"#);
        let json = format!(r#"{{"blockdevices":[{},{}]}}"#, member("sda"), member("sdb"));
        assert_eq!(parse_lsblk(&json).len(), 1);
    }
}
