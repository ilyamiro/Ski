//! Servers opened as folders: Windows shares (SMB), SSH (SFTP), FTP, WebDAV, AFP and NFS.
//!
//! Ski never talks to a server itself. A server is mounted as a FUSE filesystem and browsed like
//! any other folder, so copying, previews, "Open with" and dragging to other programs work on it
//! unchanged, the way Nautilus does it. GVfs mounts it when the desktop has GVfs (`gio mount`,
//! the folders showing up under `$XDG_RUNTIME_DIR/gvfs`); without GVfs, `sshfs` or `rclone` mount
//! it under `$XDG_RUNTIME_DIR/ski-net`. Either way the questions a server asks (a password, an
//! unknown host key) come back to the window as `Prompt`s and their answers go back as `Reply`s.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock, mpsc};

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use crate::i18n::{tr, trf};

/// A way of reaching files on another computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Smb,
    Sftp,
    Ftp,
    Ftps,
    Dav,
    Davs,
    Afp,
    Nfs,
}

impl Scheme {
    /// The scheme as written at the start of an address, including the names other programs use.
    fn from_name(name: &str) -> Option<Scheme> {
        Some(match name.to_ascii_lowercase().as_str() {
            "smb" | "cifs" => Scheme::Smb,
            "sftp" | "ssh" | "scp" | "fish" => Scheme::Sftp,
            "ftp" => Scheme::Ftp,
            "ftps" | "ftpes" => Scheme::Ftps,
            "dav" | "webdav" | "http" => Scheme::Dav,
            "davs" | "webdavs" | "https" => Scheme::Davs,
            "afp" => Scheme::Afp,
            "nfs" => Scheme::Nfs,
            _ => return None,
        })
    }

    /// How GVfs spells it, which is also what addresses are shown with.
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Smb => "smb",
            Scheme::Sftp => "sftp",
            Scheme::Ftp => "ftp",
            Scheme::Ftps => "ftps",
            Scheme::Dav => "dav",
            Scheme::Davs => "davs",
            Scheme::Afp => "afp",
            Scheme::Nfs => "nfs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scheme::Smb => tr("Windows share (SMB)"),
            Scheme::Sftp => tr("SSH (SFTP)"),
            Scheme::Ftp => "FTP",
            Scheme::Ftps => tr("FTP with TLS"),
            Scheme::Dav => "WebDAV",
            Scheme::Davs => tr("Secure WebDAV"),
            Scheme::Afp => tr("Apple file sharing (AFP)"),
            Scheme::Nfs => "NFS",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Smb => 445,
            Scheme::Sftp => 22,
            Scheme::Ftp | Scheme::Ftps => 21,
            Scheme::Dav => 80,
            Scheme::Davs => 443,
            Scheme::Afp => 548,
            Scheme::Nfs => 2049,
        }
    }

    /// Windows shares log in to a domain (workgroup) as well as with a user name.
    pub fn has_domain(self) -> bool {
        self == Scheme::Smb
    }

    /// Servers that can be used without an account: anonymous FTP, guest access to a share.
    pub fn allows_anonymous(self) -> bool {
        matches!(self, Scheme::Smb | Scheme::Ftp | Scheme::Ftps)
    }

    /// Wording for the button that logs in without an account.
    pub fn anonymous_label(self) -> &'static str {
        if self == Scheme::Smb { tr("Connect as guest") } else { tr("Connect anonymously") }
    }
}

/// Where a folder on a server is: `sftp://me@host:2222/home/me`. The path is kept decoded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    pub scheme: Scheme,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// The Windows domain or workgroup, written `DOMAIN;user@` in an address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Always starts with `/`; `/` alone is the top of the server.
    pub path: String,
}

/// What may stay unescaped in the path of an address.
const PATH_SET: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}').add(b'[').add(b']').add(b'\\').add(b'^').add(b'|');
/// What may stay unescaped in a user or domain name.
const USER_SET: &AsciiSet = &PATH_SET.add(b'/').add(b'@').add(b':').add(b';');
/// The same for an address shown to people, which only escapes what would change its meaning.
const SHOWN_USER_SET: &AsciiSet = &CONTROLS.add(b'%').add(b'/').add(b':').add(b';');

fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

impl Address {
    /// Read an address as people type it: `smb://nas/share`, `sftp://me@host:2222/dir`,
    /// `\\nas\share`, `//nas/share` or `me@host:/dir`. A password in the address is dropped;
    /// `parse_login` keeps it.
    pub fn parse(text: &str) -> Option<Address> {
        Self::parse_login(text).map(|(address, _)| address)
    }

    /// `parse`, with the password of a `user:password@host` address.
    pub fn parse_login(text: &str) -> Option<(Address, Option<String>)> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // Windows writes shares as \\server\share\folder
        if let Some(rest) = text.strip_prefix("\\\\") {
            return Self::parse_login(&format!("smb://{}", rest.replace('\\', "/")));
        }
        if let Some(rest) = text.strip_prefix("//").filter(|r| !r.starts_with('/')) {
            return Self::parse_login(&format!("smb://{rest}"));
        }
        let Some((scheme, rest)) = text.split_once("://") else {
            return Self::parse_scp(text).map(|a| (a, None));
        };
        let scheme = Scheme::from_name(scheme)?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
            None => (None, authority),
        };
        let (host, port) = split_host_port(hostport)?;
        let (mut user, mut domain, mut password) = (None, None, None);
        if let Some(info) = userinfo {
            let (name, pass) = match info.split_once(':') {
                Some((n, p)) => (n, Some(decode(p))),
                None => (info, None),
            };
            password = pass;
            // DOMAIN;user, and Windows' own DOMAIN\user
            let name = decode(name);
            let split = name.split_once(';').or_else(|| name.split_once('\\'));
            match split {
                Some((d, u)) if scheme.has_domain() => {
                    domain = Some(d.to_owned()).filter(|d| !d.is_empty());
                    user = Some(u.to_owned()).filter(|u| !u.is_empty());
                }
                _ => user = Some(name).filter(|u| !u.is_empty()),
            }
        }
        // A query or fragment is not part of a folder
        let path = path.split(['?', '#']).next().unwrap_or("/");
        Some((Address { scheme, user, domain, host, port, path: normalize_path(&decode(path)) }, password))
    }

    /// `me@host:/dir`, as scp and rsync write it.
    fn parse_scp(text: &str) -> Option<Address> {
        if text.starts_with(['/', '~', '.']) {
            return None;
        }
        let (user, rest) = text.split_once('@')?;
        let (host, path) = rest.split_once(':')?;
        let valid_host = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if user.is_empty() || user.contains('/') || !valid_host {
            return None;
        }
        let path = if path.starts_with('/') { path.to_owned() } else { format!("/{path}") };
        Some(Address { scheme: Scheme::Sftp, user: Some(user.to_owned()), domain: None, host: host.to_owned(), port: None, path: normalize_path(&path) })
    }

    /// Whether the text looks like a server address rather than a path on this computer.
    pub fn looks_like(text: &str) -> bool {
        let text = text.trim();
        text.contains("://") || text.starts_with("\\\\")
    }

    fn userinfo(&self, set: &'static AsciiSet) -> String {
        let user = self.user.as_deref().map(|u| utf8_percent_encode(u, set).to_string());
        match (&self.domain, user) {
            (Some(domain), Some(user)) => format!("{};{user}@", utf8_percent_encode(domain, set)),
            (Some(domain), None) => format!("{};@", utf8_percent_encode(domain, set)),
            (None, Some(user)) => format!("{user}@"),
            (None, None) => String::new(),
        }
    }

    fn hostport(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        match self.port {
            Some(port) => format!("{host}:{port}"),
            None => host,
        }
    }

    /// The address for programs: escaped, without a password.
    pub fn uri(&self) -> String {
        format!("{}://{}{}{}", self.scheme.name(), self.userinfo(USER_SET), self.hostport(), utf8_percent_encode(&self.path, PATH_SET))
    }

    /// The address for people: as `uri`, but the path is left readable.
    pub fn display(&self) -> String {
        format!("{}://{}{}{}", self.scheme.name(), self.userinfo(SHOWN_USER_SET), self.hostport(), self.path)
    }

    /// The share of a Windows server, the first folder of its path.
    pub fn share(&self) -> Option<&str> {
        (self.scheme == Scheme::Smb).then(|| self.path.trim_start_matches('/').split('/').next()).flatten().filter(|s| !s.is_empty())
    }

    /// A short name for the sidebar: "media on nas", "me@host".
    pub fn name(&self) -> String {
        let host = match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        };
        if let Some(share) = self.share() {
            return trf("{} on {}", &[&share, &host]);
        }
        match &self.user {
            Some(user) => format!("{user}@{host}"),
            None => host,
        }
    }

    /// The top of what one mount holds: the share of a Windows server, the server itself otherwise.
    pub fn root(&self) -> Address {
        let path = match self.share() {
            Some(share) => format!("/{share}"),
            None => "/".to_owned(),
        };
        Address { path, ..self.clone() }
    }

    /// Where `self` lies inside its root, without a leading slash: `docs/2024` for `smb://nas/share/docs/2024`.
    pub fn inside_root(&self) -> String {
        let root = self.root().path;
        self.path.strip_prefix(&root).unwrap_or(&self.path).trim_start_matches('/').to_owned()
    }

    /// The same address one folder deeper.
    pub fn join(&self, rest: &str) -> Address {
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() {
            return self.clone();
        }
        Address { path: normalize_path(&format!("{}/{rest}", self.path.trim_end_matches('/'))), ..self.clone() }
    }

    /// Whether both are served by the same mount. A missing user or port matches any.
    pub fn same_mount(&self, other: &Address) -> bool {
        let same_user = match (&self.user, &other.user) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        let port = |a: &Address| a.port.unwrap_or(a.scheme.default_port());
        let share = |a: &Address| a.share().map(str::to_lowercase);
        let dav = |s: Scheme| matches!(s, Scheme::Dav | Scheme::Davs);
        let schemes = self.scheme == other.scheme || dav(self.scheme) && dav(other.scheme);
        schemes && self.host.eq_ignore_ascii_case(&other.host) && port(self) == port(other) && same_user && share(self) == share(other)
    }

    /// What the keyring files a password of this server under.
    fn keyring_key(&self) -> String {
        format!("{}://{}@{}:{}/{}", self.scheme.name(), self.user.as_deref().unwrap_or(""), self.host.to_lowercase(), self.port.unwrap_or(self.scheme.default_port()), self.share().unwrap_or(""))
    }
}

fn split_host_port(text: &str) -> Option<(String, Option<u16>)> {
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        (host.to_owned(), after.strip_prefix(':'))
    } else {
        match text.rsplit_once(':') {
            Some((h, p)) => (h.to_owned(), Some(p)),
            None => (text.to_owned(), None),
        }
    };
    if host.is_empty() || host.contains(['/', '@', ' ']) {
        return None;
    }
    let port = match port.filter(|p| !p.is_empty()) {
        Some(p) => Some(p.parse::<u16>().ok()?),
        None => None,
    };
    Some((host, port))
}

/// `/a//b/./c/` → `/a/b/c`; `..` steps out but never above the top.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

// ---------------------------------------------------------------------------------------------
// Mounts: which servers are connected, and where their folders are on this computer
// ---------------------------------------------------------------------------------------------

/// What mounted a server, which decides how it is disconnected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountKind {
    /// GVfs, under `$XDG_RUNTIME_DIR/gvfs`.
    Gvfs,
    /// `sshfs` or `rclone`, started by Ski under `$XDG_RUNTIME_DIR/ski-net`.
    Helper,
    /// Mounted by the system or by hand (`/etc/fstab`, `mount -t cifs`).
    System,
}

/// A connected server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub name: String,
    /// The top of the mount on the server, when it can be told.
    pub address: Option<Address>,
    /// The folder the server's files appear in.
    pub local: PathBuf,
    pub kind: MountKind,
}

impl Mount {
    /// Phones and cameras that GVfs mounted (MTP, PTP, iOS) rather than servers.
    pub fn is_device(&self) -> bool {
        self.kind == MountKind::Gvfs && self.address.is_none()
    }
}

/// `$XDG_RUNTIME_DIR`, or the folder systemd would make for it.
fn runtime_dir() -> PathBuf {
    dirs::runtime_dir().unwrap_or_else(|| {
        // SAFETY: `getuid` has no preconditions.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
    })
}

/// Where GVfs shows mounted locations as folders.
pub fn gvfs_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| runtime_dir().join("gvfs"))
}

/// Where `sshfs` and `rclone` mount servers for Ski.
fn helper_root() -> PathBuf {
    runtime_dir().join("ski-net")
}

/// The addresses of the servers Ski mounted with a helper, kept beyond a restart so a pinned folder
/// on one can still be reconnected.
fn helper_records() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::data_dir).map(|d| d.join("ski").join("mounts"))
}

/// Whether files on servers get previews and folder counts, which reads them over the network.
static REMOTE_PREVIEWS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_remote_previews(on: bool) {
    REMOTE_PREVIEWS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether `path` may be read for a preview or a folder count without being asked to.
pub fn previews_allowed(path: &Path) -> bool {
    REMOTE_PREVIEWS.load(std::sync::atomic::Ordering::Relaxed) || !is_remote(path)
}

/// Mount folders known to belong to servers, refreshed by every `list_mounts`.
static REMOTE_ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());
static MOUNTS: RwLock<Vec<Mount>> = RwLock::new(Vec::new());
static LISTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether `path` lies on a server (or phone), where reading files is slow and costs data.
pub fn is_remote(path: &Path) -> bool {
    if path.starts_with(gvfs_root()) {
        return path != gvfs_root();
    }
    REMOTE_ROOTS.read().is_ok_and(|roots| roots.iter().any(|r| path.starts_with(r)))
}

/// Every connected server and GVfs device. Reads `/proc/self/mounts` and a folder listing, so it
/// is quick, but it is still meant for a background thread.
pub fn list_mounts() -> Vec<Mount> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(gvfs_root()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let address = address_of_gvfs_name(&name);
            let label = address.as_ref().map(Address::name).unwrap_or_else(|| device_name(&name));
            found.push(Mount { name: label, address, local: entry.path(), kind: MountKind::Gvfs });
        }
    }
    let helper = helper_root();
    for (source, point, kind) in proc_mounts() {
        if point.parent() == Some(helper.as_path()) {
            let address = point.file_name().and_then(|id| read_record(&id.to_string_lossy()));
            let name = address.as_ref().map(Address::name).unwrap_or_else(|| point.file_name().unwrap_or_default().to_string_lossy().into_owned());
            found.push(Mount { name, address, local: point, kind: MountKind::Helper });
        } else if let Some(address) = address_of_system_mount(&source, &kind) {
            found.push(Mount { name: address.name(), address: Some(address), local: point, kind: MountKind::System });
        }
    }
    found.sort_by_key(|m| m.name.to_lowercase());
    if let Ok(mut roots) = REMOTE_ROOTS.write() {
        *roots = found.iter().filter(|m| m.kind != MountKind::Gvfs).map(|m| m.local.clone()).collect();
    }
    if let Ok(mut mounts) = MOUNTS.write() {
        mounts.clone_from(&found);
    }
    LISTED.store(true, std::sync::atomic::Ordering::Relaxed);
    found
}

/// Mount points with their source and filesystem type, from `/proc/self/mounts`.
fn proc_mounts() -> Vec<(String, PathBuf, String)> {
    let Ok(text) = std::fs::read_to_string("/proc/self/mounts") else { return Vec::new() };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (source, point, kind) = (fields.next()?, fields.next()?, fields.next()?);
            Some((unescape(source), PathBuf::from(unescape(point)), kind.to_owned()))
        })
        .collect()
}

/// `/proc/self/mounts` writes spaces and the like as octal escapes (`\040`).
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\').then(|| bytes.get(i + 1..i + 4)).flatten().and_then(|d| std::str::from_utf8(d).ok()).and_then(|d| u8::from_str_radix(d, 8).ok());
        match octal {
            Some(b) => {
                out.push(b);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_mount_point(path: &Path) -> bool {
    proc_mounts().iter().any(|(_, point, _)| point == path)
}

/// A server mounted outside Ski: `//nas/share` (cifs), `host:/export` (nfs), `me@host:/dir` (sshfs).
fn address_of_system_mount(source: &str, kind: &str) -> Option<Address> {
    match kind {
        "cifs" | "smb3" | "smbfs" => Address::parse(source),
        "nfs" | "nfs4" => {
            let (host, path) = source.split_once(':')?;
            Some(Address { scheme: Scheme::Nfs, user: None, domain: None, host: host.trim_matches(['[', ']']).to_owned(), port: None, path: normalize_path(path) })
        }
        "fuse.sshfs" => Address::parse_scp(source).or_else(|| {
            let (host, path) = source.split_once(':')?;
            Some(Address { scheme: Scheme::Sftp, user: None, domain: None, host: host.to_owned(), port: None, path: normalize_path(path) })
        }),
        "davfs" | "fuse.davfs2" => Address::parse(source),
        _ => None,
    }
}

/// GVfs names a mount after its mount spec: `sftp:host=example.com,user=me`,
/// `smb-share:server=nas,share=media`, `dav:host=cloud,ssl=true,prefix=%2Fdav`.
pub fn address_of_gvfs_name(name: &str) -> Option<Address> {
    let (kind, spec) = name.split_once(':')?;
    let fields: HashMap<&str, String> = spec.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k, decode(v))).collect();
    let get = |key: &str| fields.get(key).cloned().filter(|v| !v.is_empty());
    let port = get("port").and_then(|p| p.parse().ok());
    let address = |scheme: Scheme, host: String, path: String| Address { scheme, user: get("user"), domain: get("domain"), host, port, path: normalize_path(&path) };
    Some(match kind {
        "sftp" => address(Scheme::Sftp, get("host")?, "/".into()),
        "ftp" => address(Scheme::Ftp, get("host")?, "/".into()),
        "ftps" => address(Scheme::Ftps, get("host")?, "/".into()),
        "smb-share" => address(Scheme::Smb, get("server")?, format!("/{}", get("share")?)),
        "dav" | "davs" => {
            let secure = kind == "davs" || get("ssl").is_some_and(|s| s == "true");
            address(if secure { Scheme::Davs } else { Scheme::Dav }, get("host")?, get("prefix").unwrap_or_else(|| "/".into()))
        }
        "afp-volume" => address(Scheme::Afp, get("host")?, format!("/{}", get("volume")?)),
        "nfs" => address(Scheme::Nfs, get("host")?, get("prefix").unwrap_or_else(|| "/".into())),
        _ => return None,
    })
}

/// A readable name for a GVfs mount that is not a server: `mtp:host=Google_Pixel_7_ABC` → "Google Pixel 7".
fn device_name(name: &str) -> String {
    let Some((_, spec)) = name.split_once(':') else { return name.to_owned() };
    let host = spec.split(',').find_map(|kv| kv.strip_prefix("host=")).map(decode).unwrap_or_else(|| spec.to_owned());
    // MTP adds the serial number after the model
    let words: Vec<&str> = host.split('_').filter(|w| !w.is_empty()).collect();
    let words = match words.split_last() {
        Some((last, rest)) if !rest.is_empty() && last.len() >= 8 && last.chars().all(|c| c.is_ascii_alphanumeric()) && last.chars().any(|c| c.is_ascii_digit()) => rest,
        _ => &words[..],
    };
    words.join(" ")
}

/// The mount `path` belongs to, from the last listing; GVfs mounts are found even before one ran.
pub fn mount_of(path: &Path) -> Option<Mount> {
    if let Ok(rest) = path.strip_prefix(gvfs_root())
        && let Some(first) = rest.components().next()
    {
        let name = first.as_os_str().to_string_lossy();
        let local = gvfs_root().join(first);
        let address = address_of_gvfs_name(&name);
        let label = address.as_ref().map(Address::name).unwrap_or_else(|| device_name(&name));
        return Some(Mount { name: label, address, local, kind: MountKind::Gvfs });
    }
    if let Ok(rest) = path.strip_prefix(helper_root())
        && let Some(first) = rest.components().next()
    {
        let id = first.as_os_str().to_string_lossy();
        let address = read_record(&id);
        let name = address.as_ref().map(Address::name).unwrap_or_else(|| id.clone().into_owned());
        return Some(Mount { name, address, local: helper_root().join(first), kind: MountKind::Helper });
    }
    MOUNTS.read().ok()?.iter().filter(|m| path.starts_with(&m.local)).max_by_key(|m| m.local.as_os_str().len()).cloned()
}

/// The server address of a folder that lies on a server, whether or not it is connected now.
pub fn address_of(path: &Path) -> Option<Address> {
    let mount = mount_of(path)?;
    let rest = path.strip_prefix(&mount.local).ok()?;
    Some(mount.address?.join(&rest.to_string_lossy()))
}

/// `path` as a server address when it lies on a server: what the path field shows.
pub fn display_path(path: &Path) -> Option<String> {
    address_of(path).map(|a| a.display())
}

/// Whether `path` is on a server that is not connected, so opening it should connect first. Goes
/// by the last listing: FUSE keeps answering for a folder for a moment after its mount went away.
pub fn is_disconnected(path: &Path) -> bool {
    let Some(mount) = mount_of(path) else { return false };
    if mount.kind == MountKind::System || mount.address.is_none() {
        return false;
    }
    if !LISTED.load(std::sync::atomic::Ordering::Relaxed) {
        list_mounts();
    }
    MOUNTS.read().is_ok_and(|mounts| !mounts.iter().any(|m| m.local == mount.local))
}

/// Where a connected server shows `address`, without asking the server.
pub fn local_path(address: &Address) -> Option<PathBuf> {
    let mounts = MOUNTS.read().ok()?;
    let mount = mounts.iter().filter_map(|m| Some((m, m.address.as_ref()?))).find(|(_, a)| a.same_mount(address) && address.path.starts_with(&a.path))?;
    let rest = address.path.strip_prefix(&mount.1.path).unwrap_or("").trim_start_matches('/');
    Some(if rest.is_empty() { mount.0.local.clone() } else { mount.0.local.join(rest) })
}

/// Folders that continue an address typed into the path field, as addresses. `None` when the text
/// is not an address; empty when its server is not connected.
pub fn completions(text: &str) -> Option<Vec<String>> {
    if !Address::looks_like(text) {
        return None;
    }
    let Some(local) = Address::parse(text).and_then(|a| local_path(&a)) else { return Some(Vec::new()) };
    let mut typed = local.to_string_lossy().into_owned();
    // The address drops a final slash, which asks for what is inside rather than for names that go on
    if text.ends_with('/') && !typed.ends_with('/') {
        typed.push('/');
    }
    Some(crate::fs::complete::dir_completions(&typed).into_iter().filter_map(|p| display_path(Path::new(&p))).collect())
}

// ---------------------------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------------------------

/// A question the server asks while connecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// The server wants an account. `user` is the name it suggests; `fixed_user` is set when the
    /// name is already decided and only the password is asked for. `retry` follows a refused password.
    Login { user: Option<String>, domain: Option<String>, fixed_user: bool, anonymous: bool, retry: bool },
    /// A choice, such as whether to trust a server never seen before. The first choice goes ahead.
    Question { message: String, choices: Vec<String> },
}

/// The answer to a `Prompt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Login { user: String, domain: String, password: String, remember: bool },
    Anonymous,
    Choice(usize),
    Cancel,
}

/// What a connection reports to the window.
#[derive(Debug)]
pub enum Progress {
    Ask(Prompt),
    /// Connected, with where the address asked for shows on this computer; `Err(None)` was cancelled.
    Done(Result<PathBuf, Option<String>>),
}

/// Connect to `address`, reporting through `report`. Answers to `Progress::Ask` go into the
/// returned sender; dropping it cancels whatever is asked.
pub fn connect(address: Address, password: Option<String>, report: impl Fn(Progress) + Send + 'static) -> mpsc::Sender<Reply> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut session = Session { report: Box::new(report), replies: rx, preset: password, tried_saved: false, refused: 0, used: None, remember: false };
        let result = session.run(&address);
        if result.is_ok() {
            session.keep_password(&address);
            list_mounts();
        }
        (session.report)(Progress::Done(result.map_err(|f| match f {
            Failure::Cancelled => None,
            Failure::Message(m) | Failure::Unsupported(m) => Some(m),
        })));
    });
    tx
}

/// An account to log in with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Login {
    user: String,
    domain: String,
    password: String,
}

enum Failure {
    Cancelled,
    Message(String),
    /// GVfs cannot reach this kind of server (its backend is missing); a helper may.
    Unsupported(String),
}

/// What `Session::login` decided.
enum Account {
    Login(Login),
    Anonymous,
}

struct Session {
    report: Box<dyn Fn(Progress) + Send>,
    replies: mpsc::Receiver<Reply>,
    /// A password typed into the address.
    preset: Option<String>,
    tried_saved: bool,
    /// How many accounts the server refused.
    refused: usize,
    /// The account of the last attempt, saved once it worked when `remember`.
    used: Option<Login>,
    remember: bool,
}

impl Session {
    fn run(&mut self, address: &Address) -> Result<PathBuf, Failure> {
        let helpers_only = std::env::var("SKI_NETWORK_BACKEND").is_ok_and(|b| b == "fuse");
        if !helpers_only && find_program("gio").is_some() {
            match self.run_gvfs(address) {
                Err(Failure::Unsupported(message)) => {
                    if !helper_can(address.scheme) {
                        return Err(Failure::Message(message));
                    }
                }
                other => return other,
            }
        }
        self.run_helper(address)
    }

    fn ask(&self, prompt: Prompt) -> Reply {
        (self.report)(Progress::Ask(prompt));
        self.replies.recv().unwrap_or(Reply::Cancel)
    }

    /// An account for `address`: one typed into the address, then a saved one, then the person's answer.
    /// `user` is what the server suggests; `fixed_user` means the server only asks for a password.
    fn login(&mut self, address: &Address, user: Option<String>, domain: Option<String>, fixed_user: bool) -> Option<Account> {
        let user = user.or_else(|| address.user.clone());
        if let Some(password) = self.preset.take() {
            return Some(Account::Login(Login { user: user.unwrap_or_default(), domain: domain.or_else(|| address.domain.clone()).unwrap_or_default(), password }));
        }
        if !self.tried_saved {
            self.tried_saved = true;
            let lookup = Address { user: user.clone(), ..address.clone() };
            if let Some(saved) = saved_login(&lookup).filter(|l| !fixed_user || user.as_deref().is_none_or(|u| u == l.user)) {
                return Some(Account::Login(saved));
            }
        }
        let prompt = Prompt::Login {
            user,
            domain: domain.or_else(|| address.domain.clone()),
            fixed_user,
            anonymous: address.scheme.allows_anonymous(),
            retry: self.refused > 0,
        };
        match self.ask(prompt) {
            Reply::Login { user, domain, password, remember } => {
                self.remember = remember;
                Some(Account::Login(Login { user, domain, password }))
            }
            Reply::Anonymous => Some(Account::Anonymous),
            Reply::Choice(_) | Reply::Cancel => None,
        }
    }

    /// Keep the password that worked, if the person asked for that.
    fn keep_password(&self, address: &Address) {
        let Some(login) = self.used.as_ref().filter(|_| self.remember) else { return };
        let address = Address { user: Some(login.user.clone()).filter(|u| !u.is_empty()), domain: Some(login.domain.clone()).filter(|d| !d.is_empty()), ..address.root() };
        if crate::fs::secrets::store(&address, &login.user, &login.domain, &login.password).is_err() {
            // Without a keyring the password lasts until Ski closes
            if let Ok(mut memory) = SESSION_PASSWORDS.lock() {
                memory.insert(Address { user: None, ..address.clone() }.keyring_key(), login.clone());
                memory.insert(address.keyring_key(), login.clone());
            }
        }
    }

    // --- GVfs ---------------------------------------------------------------------------------

    fn run_gvfs(&mut self, address: &Address) -> Result<PathBuf, Failure> {
        let mut target = address.clone();
        let mut anonymous = false;
        loop {
            match self.gvfs_attempt(&target, anonymous)? {
                Attempt::Mounted => break,
                Attempt::Again { user, anonymous: anon } => {
                    if let Some(user) = user {
                        target.user = Some(user);
                    }
                    anonymous = anon;
                }
            }
        }
        gvfs_local_path(&target)
    }

    /// One run of `gio mount`, answering its questions on its standard input.
    fn gvfs_attempt(&mut self, address: &Address, anonymous: bool) -> Result<Attempt, Failure> {
        let mut command = Command::new("gio");
        command.arg("mount");
        if anonymous {
            command.arg("--anonymous");
        }
        // Prompts are matched in English
        let mut child = command
            .arg(address.uri())
            .env("LC_ALL", "C.UTF-8")
            .env_remove("LANGUAGE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Failure::Message(e.to_string()))?;
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or_else(|| Failure::Message("gio".into()))?;
        let errors = child.stderr.take().map(|mut err| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = err.read_to_string(&mut text);
                text
            })
        });

        let mut buffer = String::new();
        let mut chunk = [0u8; 4096];
        // The account being entered: user, domain and password are asked one after the other
        let mut entering: Option<Login> = None;
        let mut outcome: Option<Result<Attempt, Failure>> = None;
        loop {
            let n = match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk[..n]));
            let Some((prompt, message)) = parse_gio_prompt(&buffer) else { continue };
            buffer.clear();
            let answer = match prompt {
                GioPrompt::Choice(choices) => match self.ask(Prompt::Question { message, choices }) {
                    Reply::Choice(i) => (i + 1).to_string(),
                    _ => {
                        outcome = Some(Err(Failure::Cancelled));
                        break;
                    }
                },
                GioPrompt::User(_) | GioPrompt::Domain(_) | GioPrompt::Password => {
                    if entering.is_none() {
                        let (suggested, fixed) = match &prompt {
                            GioPrompt::User(default) => (default.clone(), false),
                            _ => (user_in_message(&message).or_else(|| address.user.clone()), true),
                        };
                        let domain = match &prompt {
                            GioPrompt::Domain(default) => default.clone(),
                            _ => None,
                        };
                        match self.login(address, suggested.clone(), domain, fixed) {
                            None => {
                                outcome = Some(Err(Failure::Cancelled));
                                break;
                            }
                            Some(Account::Anonymous) => {
                                outcome = Some(Ok(Attempt::Again { user: None, anonymous: true }));
                                break;
                            }
                            // The name is part of the address GVfs was given: try again with the new one
                            Some(Account::Login(login)) if fixed && !login.user.is_empty() && suggested.as_deref() != Some(login.user.as_str()) => {
                                self.preset = Some(login.password.clone());
                                self.used = Some(login.clone());
                                outcome = Some(Ok(Attempt::Again { user: Some(login.user), anonymous: false }));
                                break;
                            }
                            Some(Account::Login(login)) => entering = Some(login),
                        }
                    }
                    let login = entering.clone().unwrap_or(Login { user: String::new(), domain: String::new(), password: String::new() });
                    match prompt {
                        GioPrompt::User(_) => login.user,
                        GioPrompt::Domain(_) => login.domain,
                        _ => {
                            self.used = entering.take();
                            self.refused += 1;
                            login.password
                        }
                    }
                }
            };
            let written = stdin.as_mut().is_some_and(|s| s.write_all(format!("{answer}\n").as_bytes()).and_then(|_| s.flush()).is_ok());
            if !written {
                break;
            }
        }
        // Closing its input makes gio give up on a question it still waits for
        drop(stdin);
        let status = child.wait().map_err(|e| Failure::Message(e.to_string()))?;
        let errors = errors.and_then(|t| t.join().ok()).unwrap_or_default();
        if let Some(outcome) = outcome {
            return outcome;
        }
        if status.success() || errors.contains("already mounted") {
            // Whatever was asked last was accepted
            self.refused = 0;
            return Ok(Attempt::Mounted);
        }
        let message = clean_gio_error(&errors);
        if message.is_empty() {
            return Err(Failure::Cancelled);
        }
        if errors.contains("not supported") || errors.contains("Operation not supported") {
            return Err(Failure::Unsupported(message));
        }
        Err(Failure::Message(message))
    }

    // --- sshfs and rclone ---------------------------------------------------------------------

    fn run_helper(&mut self, address: &Address) -> Result<PathBuf, Failure> {
        let root = address.root();
        let id = helper_id(&root);
        let point = helper_root().join(&id);
        if !is_mount_point(&point) {
            std::fs::create_dir_all(&point).map_err(|e| Failure::Message(e.to_string()))?;
            let mounted = if address.scheme == Scheme::Sftp && find_program("sshfs").is_some() {
                self.sshfs(&root, &point)
            } else if find_program("rclone").is_some() && rclone_backend(address.scheme).is_some() {
                self.rclone(&root, &point)
            } else {
                Err(Failure::Message(missing_tools(address.scheme)))
            };
            match mounted {
                Ok(account) => write_record(&id, &Address { user: account.or(root.user.clone()), ..root.clone() }),
                Err(err) => {
                    let _ = std::fs::remove_dir(&point);
                    return Err(err);
                }
            }
        }
        let inside = address.inside_root();
        Ok(if inside.is_empty() { point } else { point.join(inside) })
    }

    /// Mount with sshfs: the keys of the ssh agent first, then a password. The user name that worked comes back.
    fn sshfs(&mut self, root: &Address, point: &Path) -> Result<Option<String>, Failure> {
        self.check_host_key(root)?;
        let mut login: Option<Login> = self.preset.take().map(|password| Login { user: root.user.clone().unwrap_or_default(), domain: String::new(), password });
        loop {
            let user = login.as_ref().map(|l| l.user.clone()).filter(|u| !u.is_empty()).or_else(|| root.user.clone());
            let target = match &user {
                Some(user) => format!("{user}@{}:/", bracket(&root.host)),
                None => format!("{}:/", bracket(&root.host)),
            };
            let mut command = Command::new("sshfs");
            command.arg(&target).arg(point).args(["-p", &root.port.unwrap_or(22).to_string()]);
            command.args(["-o", "reconnect,ServerAliveInterval=15,ServerAliveCountMax=3,StrictHostKeyChecking=yes"]);
            if login.is_some() {
                command.args(["-o", "password_stdin,PubkeyAuthentication=no"]);
            } else {
                command.args(["-o", "BatchMode=yes"]);
            }
            let output = run_with_input(command, login.as_ref().map(|l| format!("{}\n", l.password)))?;
            if output.0 {
                self.used = login.clone();
                return Ok(user);
            }
            let error = output.1;
            if !error.contains("Permission denied") && !error.contains("Authentication failed") && !error.contains("password") {
                return Err(Failure::Message(last_line(&error)));
            }
            if login.is_some() {
                self.refused += 1;
            }
            login = match self.login(root, user, None, false) {
                Some(Account::Login(l)) => Some(l),
                _ => return Err(Failure::Cancelled),
            };
        }
    }

    /// Mount with rclone, after a listing has shown the account works (rclone only connects on first use).
    fn rclone(&mut self, root: &Address, point: &Path) -> Result<Option<String>, Failure> {
        if root.scheme == Scheme::Sftp {
            self.check_host_key(root)?;
        }
        let backend = rclone_backend(root.scheme).ok_or_else(|| Failure::Message(missing_tools(root.scheme)))?;
        let mut login: Option<Login> = self.preset.take().map(|password| Login { user: root.user.clone().unwrap_or_default(), domain: root.domain.clone().unwrap_or_default(), password });
        let mut anonymous = false;
        loop {
            let user = login.as_ref().map(|l| l.user.clone()).filter(|u| !u.is_empty()).or_else(|| root.user.clone());
            let domain = login.as_ref().map(|l| l.domain.clone()).filter(|d| !d.is_empty()).or_else(|| root.domain.clone());
            let (user, password) = if anonymous {
                (Some("anonymous".to_owned()), Some("anonymous@".to_owned()))
            } else {
                (user, login.as_ref().map(|l| l.password.clone()))
            };
            let remote = rclone_remote(root, user.as_deref(), domain.as_deref());
            // Always given, if only empty: rclone's FTP refuses a remote without one
            let env = [(format!("RCLONE_{}_PASS", backend.to_uppercase()), rclone_obscure(password.as_deref().unwrap_or(""))?)];
            let mut check = Command::new("rclone");
            check.args(["lsf", "--max-depth", "1", "--contimeout", "15s", "--timeout", "30s", "--retries", "1", "--low-level-retries", "1", "-q"]).arg(&remote);
            check.envs(env.iter().map(|(k, v)| (k, v)));
            let (ok, error) = run_with_input(check, None)?;
            if ok {
                let mut mount = Command::new("rclone");
                mount.arg("mount").arg(&remote).arg(point).args(["--daemon", "--vfs-cache-mode", "writes", "--dir-cache-time", "15s", "--contimeout", "15s"]);
                mount.envs(env.iter().map(|(k, v)| (k, v)));
                let (ok, error) = run_with_input(mount, None)?;
                if !ok {
                    return Err(Failure::Message(last_line(&error)));
                }
                self.used = login.clone().filter(|_| !anonymous);
                return Ok(user.filter(|_| !anonymous));
            }
            if !is_auth_error(&error) {
                return Err(Failure::Message(last_line(&error)));
            }
            if login.is_some() || anonymous {
                self.refused += 1;
            }
            match self.login(root, user, domain, false) {
                Some(Account::Login(l)) => {
                    login = Some(l);
                    anonymous = false;
                }
                Some(Account::Anonymous) => anonymous = true,
                None => return Err(Failure::Cancelled),
            }
        }
    }

    /// Ask before talking to an SSH server that `~/.ssh/known_hosts` does not know, as ssh would
    /// in a terminal; the keys it shows are added there once trusted.
    fn check_host_key(&mut self, root: &Address) -> Result<(), Failure> {
        let port = root.port.unwrap_or(22);
        let name = if port == 22 { root.host.clone() } else { format!("[{}]:{port}", root.host) };
        let known = Command::new("ssh-keygen").args(["-F", &name]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if known || find_program("ssh-keyscan").is_none() {
            return Ok(());
        }
        let scan = Command::new("ssh-keyscan").args(["-T", "10", "-p", &port.to_string(), &root.host]).stderr(Stdio::null()).output().map_err(|e| Failure::Message(e.to_string()))?;
        let keys = String::from_utf8_lossy(&scan.stdout).into_owned();
        if keys.trim().is_empty() {
            return Err(Failure::Message(trf("Could not reach {}", &[&root.host])));
        }
        let mut fingerprint = Command::new("ssh-keygen");
        fingerprint.args(["-l", "-f", "-"]);
        let (_, listed) = run_with_input_out(fingerprint, Some(keys.clone()))?;
        let print = listed.lines().find(|l| l.contains("ED25519")).or_else(|| listed.lines().next()).and_then(|l| l.split_whitespace().nth(1)).unwrap_or("?").to_owned();
        let message = format!("The identity of the remote computer ({}) is unknown.\nThe identity sent by the remote computer is {print}.", root.host);
        match self.ask(Prompt::Question { message, choices: vec!["Log In Anyway".into(), "Cancel Login".into()] }) {
            Reply::Choice(0) => {
                let dir = dirs::home_dir().map(|h| h.join(".ssh")).ok_or(Failure::Cancelled)?;
                {
                    use std::os::unix::fs::DirBuilderExt;
                    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
                }
                let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("known_hosts")).map_err(|e| Failure::Message(e.to_string()))?;
                file.write_all(keys.as_bytes()).map_err(|e| Failure::Message(e.to_string()))?;
                Ok(())
            }
            _ => Err(Failure::Cancelled),
        }
    }
}

enum Attempt {
    Mounted,
    /// Run `gio mount` again, with another user name or anonymously.
    Again { user: Option<String>, anonymous: bool },
}

/// A prompt `gio mount` printed and waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GioPrompt {
    User(Option<String>),
    Domain(Option<String>),
    Password,
    Choice(Vec<String>),
}

/// The prompt at the end of what `gio mount` printed, with the message before it. gio writes
/// `Message\nUser [default]: `, `Password: `, or a question as `Message\n[1] Yes\n[2] No\nChoice: `.
fn parse_gio_prompt(text: &str) -> Option<(GioPrompt, String)> {
    let (before, last) = match text.rfind('\n') {
        Some(i) => (&text[..i], &text[i + 1..]),
        None => ("", text),
    };
    let label = last.strip_suffix(": ")?;
    let (word, default) = match label.split_once(" [") {
        Some((w, d)) => (w, d.strip_suffix(']').map(str::to_owned)),
        None => (label, None),
    };
    let lines: Vec<&str> = before.lines().collect();
    let message = |lines: &[&str]| lines.iter().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    let prompt = match word {
        "User" => GioPrompt::User(default),
        "Domain" => GioPrompt::Domain(default),
        "Password" => GioPrompt::Password,
        "Choice" => {
            let first = lines.iter().position(|l| l.starts_with("[1] "))?;
            let choices = lines[first..].iter().filter_map(|l| l.split_once("] ").map(|(_, c)| c.to_owned())).collect();
            return Some((GioPrompt::Choice(choices), message(&lines[..first])));
        }
        _ => return None,
    };
    Some((prompt, message(&lines)))
}

/// The user GVfs names in "Enter password for “me” on “host”".
fn user_in_message(message: &str) -> Option<String> {
    let rest = message.split("password for “").nth(1)?;
    let (user, after) = rest.split_once('”')?;
    after.trim_start().starts_with("on").then(|| user.to_owned())
}

/// "gio: smb://nas/: Failed to mount Windows share: Connection refused" → the part after the address.
fn clean_gio_error(text: &str) -> String {
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let line = line.strip_prefix("gio: ").unwrap_or(line);
    // The address is followed by ": "; it may contain ":" itself (ports), so cut after "://…/: "
    let line = match line.find("://") {
        Some(start) => line[start..].find(": ").map_or(line, |i| &line[start + i + 2..]),
        None => line,
    };
    line.trim().to_owned()
}

/// Where GVfs shows `address` on this computer, asked of `gio info`.
fn gvfs_local_path(address: &Address) -> Result<PathBuf, Failure> {
    let output = Command::new("gio").args(["info", "-a", "standard::type"]).arg(address.uri()).env("LC_ALL", "C.UTF-8").env_remove("LANGUAGE").output().map_err(|e| Failure::Message(e.to_string()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if let Some(path) = text.lines().find_map(|l| l.strip_prefix("local path: ")) {
        return Ok(PathBuf::from(path.trim()));
    }
    if !output.status.success() {
        return Err(Failure::Message(clean_gio_error(&String::from_utf8_lossy(&output.stderr))));
    }
    Err(Failure::Message(tr("GVfs runs without its FUSE part (gvfsd-fuse), so the server can't be opened as a folder").to_owned()))
}

/// The backend of rclone that reaches this kind of server.
fn rclone_backend(scheme: Scheme) -> Option<&'static str> {
    match scheme {
        Scheme::Sftp => Some("sftp"),
        Scheme::Ftp | Scheme::Ftps => Some("ftp"),
        Scheme::Smb => Some("smb"),
        Scheme::Dav | Scheme::Davs => Some("webdav"),
        Scheme::Afp | Scheme::Nfs => None,
    }
}

fn helper_can(scheme: Scheme) -> bool {
    scheme == Scheme::Sftp && find_program("sshfs").is_some() || find_program("rclone").is_some() && rclone_backend(scheme).is_some()
}

fn missing_tools(scheme: Scheme) -> String {
    match scheme {
        Scheme::Sftp => tr("Install GVfs, sshfs or rclone to connect to servers").to_owned(),
        Scheme::Afp | Scheme::Nfs => tr("Install GVfs to connect to this kind of server").to_owned(),
        _ => tr("Install GVfs or rclone to connect to servers").to_owned(),
    }
}

/// An rclone remote made on the spot, such as `:sftp,host=example.com,user=me:/`.
fn rclone_remote(root: &Address, user: Option<&str>, domain: Option<&str>) -> String {
    let mut options = Vec::new();
    let mut put = |key: &str, value: &str| options.push(format!("{key}={}", rclone_quote(value)));
    let port = root.port.map(|p| p.to_string());
    let path = match root.scheme {
        Scheme::Dav | Scheme::Davs => {
            let base = Address { user: None, domain: None, path: root.path.clone(), ..root.clone() }.uri();
            let url = base.replacen("davs://", "https://", 1).replacen("dav://", "http://", 1);
            put("url", &url);
            if let Some(user) = user {
                put("user", user);
            }
            String::new()
        }
        _ => {
            put("host", &root.host);
            if let Some(port) = &port {
                put("port", port);
            }
            if let Some(user) = user {
                put("user", user);
            }
            match root.scheme {
                Scheme::Smb => {
                    if let Some(domain) = domain {
                        put("domain", domain);
                    }
                    root.share().unwrap_or_default().to_owned()
                }
                Scheme::Ftps => {
                    put("explicit_tls", "true");
                    String::new()
                }
                Scheme::Sftp => {
                    if let Some(known) = dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts")).filter(|k| k.is_file()) {
                        put("known_hosts_file", &known.to_string_lossy());
                    }
                    "/".to_owned()
                }
                _ => String::new(),
            }
        }
    };
    format!(":{},{}:{path}", rclone_backend(root.scheme).unwrap_or("local"), options.join(","))
}

/// A value of an rclone connection string, quoted when it holds a separator.
fn rclone_quote(value: &str) -> String {
    if value.contains([',', ':', '=', '"', '\'']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// rclone takes passwords in its own obscured form; it reads the password on its input, so it is
/// never seen in the list of processes.
fn rclone_obscure(password: &str) -> Result<String, Failure> {
    let mut command = Command::new("rclone");
    command.args(["obscure", "-"]);
    let (ok, out) = run_with_input_out(command, Some(password.to_owned()))?;
    if ok { Ok(out.trim().to_owned()) } else { Err(Failure::Message(last_line(&out))) }
}

fn is_auth_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    ["530", "login incorrect", "logon_failure", "logon failure", "authenticat", "401", "permission denied", "access denied", "unable to authenticate"].iter().any(|k| lower.contains(k))
}

/// Run a command, feed it `input`, and report whether it worked with what it wrote to stderr.
fn run_with_input(command: Command, input: Option<String>) -> Result<(bool, String), Failure> {
    run_capturing(command, input, false)
}

/// `run_with_input`, reporting what it wrote to stdout instead.
fn run_with_input_out(command: Command, input: Option<String>) -> Result<(bool, String), Failure> {
    run_capturing(command, input, true)
}

fn run_capturing(mut command: Command, input: Option<String>, stdout: bool) -> Result<(bool, String), Failure> {
    let mut child = command
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::Message(e.to_string()))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(text.as_bytes());
    }
    let output = child.wait_with_output().map_err(|e| Failure::Message(e.to_string()))?;
    let text = if stdout { &output.stdout } else { &output.stderr };
    Ok((output.status.success(), String::from_utf8_lossy(text).into_owned()))
}

fn last_line(text: &str) -> String {
    text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_owned()
}

fn bracket(host: &str) -> String {
    if host.contains(':') { format!("[{host}]") } else { host.to_owned() }
}

pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    // fusermount is setuid, and NixOS keeps those wrappers outside the usual PATH of a desktop session
    std::env::split_paths(&paths).chain([PathBuf::from("/run/wrappers/bin")]).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// The folder name a helper mount gets: readable and the same every time for the same server.
fn helper_id(root: &Address) -> String {
    let mut id = format!("{}-", root.scheme.name());
    if let Some(user) = &root.user {
        id.push_str(user);
        id.push('@');
    }
    id.push_str(&root.host);
    if let Some(port) = root.port {
        id.push_str(&format!("-{port}"));
    }
    if let Some(share) = root.share() {
        id.push('-');
        id.push_str(share);
    }
    id.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '@' | '-' | '_') { c } else { '_' }).collect()
}

fn write_record(id: &str, address: &Address) {
    let Some(dir) = helper_records() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string(address) {
        let _ = std::fs::write(dir.join(format!("{id}.json")), json);
    }
}

fn read_record(id: &str) -> Option<Address> {
    let text = std::fs::read_to_string(helper_records()?.join(format!("{id}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

// ---------------------------------------------------------------------------------------------
// Disconnecting
// ---------------------------------------------------------------------------------------------

/// Disconnect the server mounted at `mount.local`.
pub fn disconnect(mount: &Mount) -> Result<(), String> {
    let result = match mount.kind {
        MountKind::Gvfs => {
            let out = Command::new("gio").args(["mount", "-u"]).arg(&mount.local).env("LC_ALL", "C.UTF-8").output().map_err(|e| e.to_string())?;
            if out.status.success() { Ok(()) } else { Err(clean_gio_error(&String::from_utf8_lossy(&out.stderr))) }
        }
        MountKind::Helper | MountKind::System => {
            let attempts: [(&str, &[&str]); 3] = [("fusermount3", &["-u"]), ("fusermount", &["-u"]), ("umount", &[])];
            let mut last = Err(tr("Nothing to disconnect it with was found").to_owned());
            for (program, args) in attempts {
                let Some(path) = find_program(program) else { continue };
                match Command::new(path).args(args).arg(&mount.local).output() {
                    Ok(out) if out.status.success() => {
                        last = Ok(());
                        break;
                    }
                    Ok(out) => last = Err(last_line(&String::from_utf8_lossy(&out.stderr))),
                    Err(e) => last = Err(e.to_string()),
                }
            }
            if last.is_ok() && mount.kind == MountKind::Helper {
                let _ = std::fs::remove_dir(&mount.local);
            }
            last
        }
    };
    list_mounts();
    result
}

// ---------------------------------------------------------------------------------------------
// Passwords
// ---------------------------------------------------------------------------------------------

/// Passwords kept until Ski closes when there is no keyring to save them in.
static SESSION_PASSWORDS: LazyLock<Mutex<HashMap<String, Login>>> = LazyLock::new(Default::default);

/// A saved account for `address`: from this session, then from the keyring.
fn saved_login(address: &Address) -> Option<Login> {
    if let Ok(memory) = SESSION_PASSWORDS.lock()
        && let Some(login) = memory.get(&address.root().keyring_key()).or_else(|| memory.get(&Address { user: None, ..address.root() }.keyring_key()))
    {
        return Some(login.clone());
    }
    let (user, domain, password) = crate::fs::secrets::lookup(&address.root())?;
    Some(Login { user, domain, password })
}

/// Forget the saved passwords of `address`.
pub fn forget_password(address: &Address) -> Result<(), String> {
    if let Ok(mut memory) = SESSION_PASSWORDS.lock() {
        memory.retain(|key, _| *key != address.root().keyring_key() && *key != Address { user: None, ..address.root() }.keyring_key());
    }
    crate::fs::secrets::delete(&address.root())
}

// ---------------------------------------------------------------------------------------------
// Saved servers
// ---------------------------------------------------------------------------------------------

/// A server kept in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// Shown instead of the address; empty uses `Address::name`.
    #[serde(default)]
    pub name: String,
    pub address: Address,
}

impl Server {
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() { self.address.name() } else { self.name.clone() }
    }
}

/// The sidebar's servers and the addresses connected to lately, saved in `~/.config/ski/servers.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Servers {
    pub saved: Vec<Server>,
    /// Latest first.
    pub recent: Vec<Address>,
}

const RECENT_KEPT: usize = 8;

impl Servers {
    fn file() -> Option<PathBuf> {
        crate::fs::admin::config_override().or_else(dirs::config_dir).map(|d| d.join("ski").join("servers.json"))
    }

    pub fn load() -> Servers {
        Self::file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if crate::fs::admin::is_admin() {
            return;
        }
        let Some(file) = Self::file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(file, json);
        }
    }

    /// Put `address` first among the recent ones.
    pub fn used(&mut self, address: &Address) {
        self.recent.retain(|a| a != address);
        self.recent.insert(0, address.clone());
        self.recent.truncate(RECENT_KEPT);
    }

    /// The saved server `address` belongs to.
    pub fn find(&self, address: &Address) -> Option<usize> {
        self.saved.iter().position(|s| s.address.same_mount(address) && address.path.starts_with(&s.address.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> Address {
        Address::parse(text).unwrap_or_else(|| panic!("{text} did not parse"))
    }

    #[test]
    fn addresses_are_read_the_ways_people_write_them() {
        let a = addr("sftp://me@example.com:2222/home/me/");
        assert_eq!((a.scheme, a.user.as_deref(), a.host.as_str(), a.port, a.path.as_str()), (Scheme::Sftp, Some("me"), "example.com", Some(2222), "/home/me"));
        let a = addr("smb://WORK;bob@nas/Media%20Files/Films");
        assert_eq!((a.domain.as_deref(), a.user.as_deref(), a.share()), (Some("WORK"), Some("bob"), Some("Media Files")));
        assert_eq!(a.root().path, "/Media Files");
        assert_eq!(a.inside_root(), "Films");
        assert_eq!(addr(r"\\nas\share\dir").display(), "smb://nas/share/dir");
        assert_eq!(addr("//nas/share").uri(), "smb://nas/share");
        assert_eq!(addr("me@host:/srv/www").display(), "sftp://me@host/srv/www");
        assert_eq!(addr("ssh://host").scheme, Scheme::Sftp);
        assert_eq!(addr("https://cloud.example.com/remote.php/dav").scheme, Scheme::Davs);
        assert_eq!(addr("sftp://[::1]:22/").host, "::1");
        assert_eq!(addr("ftp://h/a/../b/./c").path, "/b/c");
        let (a, password) = Address::parse_login("ftp://user:s%40cret@ftp.example.com/pub").unwrap();
        assert_eq!((a.user.as_deref(), password.as_deref(), a.path.as_str()), (Some("user"), Some("s@cret"), "/pub"));
        assert!(Address::parse("/home/me").is_none());
        assert!(Address::parse("~/x").is_none());
        assert!(Address::parse("gopher://x").is_none());
        assert!(Address::parse("sftp://").is_none());
        assert!(Address::parse("sftp://host:99999/").is_none());
    }

    #[test]
    fn addresses_are_written_escaped_for_programs_and_plain_for_people() {
        let a = addr("smb://dom;us er@nas/My Share/a%23b");
        assert_eq!(a.uri(), "smb://dom;us%20er@nas/My%20Share/a%23b");
        assert_eq!(a.display(), "smb://dom;us er@nas/My Share/a#b");
        assert_eq!(addr("sftp://me@h/").join("x/y").display(), "sftp://me@h/x/y");
    }

    #[test]
    fn mounts_match_by_server_share_user_and_port() {
        assert!(addr("sftp://h/a").same_mount(&addr("sftp://me@h:22/")));
        assert!(!addr("sftp://h:2222/").same_mount(&addr("sftp://h/")));
        assert!(!addr("smb://nas/a").same_mount(&addr("smb://nas/b")));
        assert!(addr("smb://NAS/Media").same_mount(&addr("smb://nas/media/x")));
        assert!(!addr("sftp://a@h/").same_mount(&addr("sftp://b@h/")));
    }

    #[test]
    fn gvfs_mount_names_become_addresses() {
        assert_eq!(address_of_gvfs_name("sftp:host=example.com,user=me").unwrap().display(), "sftp://me@example.com/");
        assert_eq!(address_of_gvfs_name("ftp:host=127.0.0.1,port=2121").unwrap().display(), "ftp://127.0.0.1:2121/");
        let share = address_of_gvfs_name("smb-share:domain=WORK,server=nas,share=media%20files,user=bob").unwrap();
        assert_eq!(share.display(), "smb://WORK;bob@nas/media files");
        assert_eq!(address_of_gvfs_name("dav:host=cloud,ssl=true,prefix=%2Fremote.php%2Fdav").unwrap().display(), "davs://cloud/remote.php/dav");
        assert!(address_of_gvfs_name("mtp:host=Google_Pixel_7_1A2B3C4D5E").is_none());
        assert_eq!(device_name("mtp:host=Google_Pixel_7_1A2B3C4D5E"), "Google Pixel 7");
    }

    #[test]
    fn gvfs_paths_map_to_addresses_and_back() {
        let local = gvfs_root().join("sftp:host=example.com,user=me").join("srv/www");
        assert_eq!(display_path(&local).as_deref(), Some("sftp://me@example.com/srv/www"));
        assert!(is_remote(&local));
        assert!(!is_remote(Path::new("/home")));
        let share = gvfs_root().join("smb-share:server=nas,share=media").join("films");
        assert_eq!(display_path(&share).as_deref(), Some("smb://nas/media/films"));
    }

    #[test]
    fn gio_prompts_are_recognized() {
        let text = "Authentication Required\nEnter user and password for “127.0.0.1:2121”:\nUser [test]: ";
        let (prompt, message) = parse_gio_prompt(text).unwrap();
        assert_eq!(prompt, GioPrompt::User(Some("test".into())));
        assert!(message.starts_with("Authentication Required"));
        assert_eq!(parse_gio_prompt("Password: ").unwrap().0, GioPrompt::Password);
        assert_eq!(parse_gio_prompt("Domain [WORKGROUP]: ").unwrap().0, GioPrompt::Domain(Some("WORKGROUP".into())));
        let question = "The identity of the remote computer (h) is unknown.\nGo on?\n[1] Log In Anyway\n[2] Cancel Login\nChoice: ";
        let (prompt, message) = parse_gio_prompt(question).unwrap();
        assert_eq!(prompt, GioPrompt::Choice(vec!["Log In Anyway".into(), "Cancel Login".into()]));
        assert_eq!(message, "The identity of the remote computer (h) is unknown.\nGo on?");
        assert!(parse_gio_prompt("Authentication Required\n").is_none());
        assert!(parse_gio_prompt("Password: x").is_none());
        let text = "Authentication Required\nEnter password for “ilyamiro” on “localhost”:\nPassword: ";
        assert_eq!(user_in_message(&parse_gio_prompt(text).unwrap().1).as_deref(), Some("ilyamiro"));
    }

    #[test]
    fn gio_errors_lose_their_prefix() {
        assert_eq!(clean_gio_error("gio: ftp://127.0.0.1:2999/: Could not connect to 127.0.0.1: Connection refused\n"), "Could not connect to 127.0.0.1: Connection refused");
        assert_eq!(clean_gio_error("gio: smb://nas/x: Location is already mounted"), "Location is already mounted");
        assert_eq!(clean_gio_error(""), "");
    }

    #[test]
    fn rclone_remotes_are_spelled_out() {
        assert_eq!(rclone_remote(&addr("ftp://h:2121/"), Some("me"), None), ":ftp,host=h,port=2121,user=me:");
        assert_eq!(rclone_remote(&addr("smb://nas/media"), Some("bob"), Some("WORK")), ":smb,host=nas,user=bob,domain=WORK:media");
        assert_eq!(rclone_remote(&addr("davs://cloud/remote.php/dav"), None, None), ":webdav,url=\"https://cloud/remote.php/dav\":");
        assert_eq!(rclone_quote("a,b"), "\"a,b\"");
        assert_eq!(helper_id(&addr("smb://bob@nas/my share").root()), "smb-bob@nas-my_share");
    }

    #[test]
    fn recent_servers_stay_few_and_unique() {
        let mut servers = Servers::default();
        for i in 0..12 {
            servers.used(&addr(&format!("sftp://h{i}/")));
        }
        servers.used(&addr("sftp://h5/"));
        assert_eq!(servers.recent.len(), RECENT_KEPT);
        assert_eq!(servers.recent[0].host, "h5");
        assert_eq!(servers.recent.iter().filter(|a| a.host == "h5").count(), 1);
    }
}
