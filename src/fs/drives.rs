//! Removable and extra drives: list them with `lsblk`, mount and eject them with `udisksctl`.
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::places::{Place, PlaceGroup};
use crate::i18n::trf;
use crate::ui::icons;

/// The block device behind a Devices entry of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// `/dev/sdb1`
    pub dev: String,
    pub mounted: bool,
    /// Can be powered off, as USB sticks and cards can.
    pub removable: bool,
    /// Mounted by the system itself (`/boot`, `/home`...), so it is not offered for unmounting.
    pub system: bool,
}

/// Where the desktop and the user put drives. A drive mounted anywhere else was mounted by the system.
const MOUNT_ROOTS: &[&str] = &["/run/media", "/media", "/mnt"];

/// Filesystems that are not something to browse.
const NOT_BROWSABLE: &[&str] = &["swap", "LVM2_member", "crypto_LUKS", "linux_raid_member", "zfs_member", "isw_raid_member", "bcache", "ddf_raid_member"];

/// Partitions that belong to a system's boot rather than hold files, listed only while mounted:
/// EFI system, Microsoft reserved, Windows recovery and BIOS boot (GPT), EFI and hidden recovery (MBR).
const BOOT_PART_TYPES: &[&str] = &[
    "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
    "e3c9e316-0b5c-4db8-817d-f92df00215ae",
    "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
    "21686148-6449-6e6f-744e-656564454649",
    "0xef",
    "0x27",
];

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => s == "1" || s == "true",
        Value::Number(n) => n.as_u64() == Some(1),
        _ => false,
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Every folder a device is mounted at: `mountpoints` from newer `lsblk`, else `mountpoint`.
fn mount_points(node: &Value) -> Vec<&str> {
    let mut all: Vec<&str> = node.get("mountpoints").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    if all.is_empty() {
        all.extend(text(node, "mountpoint"));
    }
    all.retain(|m| !m.is_empty() && m.starts_with('/'));
    all
}

fn walk(node: &Value, inherited_removable: bool, all: bool, out: &mut Vec<Place>) {
    let removable = inherited_removable || truthy(&node["rm"]) || truthy(&node["hotplug"]);
    let kind = text(node, "type").unwrap_or_default();
    let fstype = text(node, "fstype");
    let mounts = mount_points(node);
    // A user's mount wins over a system one when the device is mounted at both
    let mount = mounts.iter().find(|m| MOUNT_ROOTS.iter().any(|r| Path::new(m).starts_with(r))).or(mounts.first()).map(PathBuf::from);
    let has_children = node.get("children").and_then(Value::as_array).is_some_and(|c| !c.is_empty());
    let boot_part = text(node, "parttype").is_some_and(|t| BOOT_PART_TYPES.contains(&t.to_ascii_lowercase().as_str()));

    let browsable = fstype.is_some_and(|f| !NOT_BROWSABLE.contains(&f)) && !matches!(kind, "loop" | "rom") && !has_children;
    let user_mount = mount.as_ref().is_some_and(|m| MOUNT_ROOTS.iter().any(|r| m.starts_with(r)));
    // The system's own root is the sidebar's Computer. Without `all`, only what the user plugged in or mounted
    let listed = browsable
        && !mounts.contains(&"/")
        && (mount.is_some() || !boot_part)
        && (all || user_mount || (mount.is_none() && removable));
    if listed && let Some(dev) = text(node, "path") {
        let label = text(node, "label").map(str::to_owned).or_else(|| {
            mount.as_ref().and_then(|m| m.file_name()).map(|n| n.to_string_lossy().into_owned())
        });
        let name = label.unwrap_or_else(|| trf("{} Volume", &[&text(node, "size").unwrap_or("Drive")]));
        let system = mount.is_some() && !user_mount;
        let icon = if removable { icons::USB } else { icons::HARD_DISK };
        let mut place = Place::new(name, mount.clone().unwrap_or_default(), icon, PlaceGroup::Devices);
        place.drive = Some(Drive { dev: dev.to_owned(), mounted: mount.is_some(), removable, system });
        out.push(place);
    }
    for child in node.get("children").and_then(Value::as_array).into_iter().flatten() {
        walk(child, removable, all, out);
    }
}

/// The drives in `lsblk`'s JSON; `all` lists internal disks and partitions too, not just removable ones and user mounts.
pub fn parse_lsblk(json: &str, all: bool) -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();
    if let Ok(root) = serde_json::from_str::<Value>(json) {
        for node in root["blockdevices"].as_array().into_iter().flatten() {
            walk(node, false, all, &mut out);
        }
    }
    // A device under several holders (RAID, multipath) shows up once per holder
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| p.drive.as_ref().is_none_or(|d| seen.insert(d.dev.clone())));
    // The computer's own disks first, then what is plugged in
    out.sort_by_key(|p| (p.drive.as_ref().is_some_and(|d| d.removable), p.name.to_lowercase()));
    out
}

/// Drives to show, or what sits in the usual mount folders when `lsblk` is missing.
pub fn list(all: bool) -> Vec<Place> {
    // `mountpoints` and `parttype` are missing from old `lsblk`, which then refuses the whole list
    const COLUMNS: [&str; 2] = ["NAME,PATH,LABEL,SIZE,MOUNTPOINT,MOUNTPOINTS,RM,HOTPLUG,TYPE,FSTYPE,PARTTYPE", "NAME,PATH,LABEL,SIZE,MOUNTPOINT,RM,HOTPLUG,TYPE,FSTYPE"];
    for columns in COLUMNS {
        if let Ok(o) = Command::new("lsblk").args(["-J", "-o", columns]).output()
            && o.status.success() {
                return parse_lsblk(&String::from_utf8_lossy(&o.stdout), all);
            }
    }
    from_mount_roots()
}

/// Used and total bytes of the filesystem holding `path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub used: u64,
    pub total: u64,
    /// What an ordinary user can still write, which leaves out the blocks kept for root.
    pub free: u64,
}

impl Space {
    pub fn of(path: &Path) -> Option<Space> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: a valid C string and a statvfs to fill
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let unit = st.f_frsize as u64;
        let total = st.f_blocks as u64 * unit;
        (total > 0).then(|| Space { used: total.saturating_sub(st.f_bfree as u64 * unit), total, free: st.f_bavail as u64 * unit })
    }

    pub fn fraction(&self) -> f32 {
        (self.used as f64 / self.total as f64).clamp(0.0, 1.0) as f32
    }
}

/// Without `lsblk`: whatever sits in the usual mount folders.
fn from_mount_roots() -> Vec<Place> {
    let user = std::env::var("USER").unwrap_or_default();
    let roots = [PathBuf::from("/run/media").join(&user), PathBuf::from("/media").join(&user), PathBuf::from("/media")];
    let mut drives: Vec<Place> = roots
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flat_map(|rd| rd.flatten())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Place::new(name, p, icons::USB, PlaceGroup::Devices)
        })
        .collect();
    drives.sort_by_key(|d| d.name.to_lowercase());
    drives
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() { format!("{program} failed") } else { err })
    }
}

/// Mount a device; the folder it is mounted at.
pub fn mount(dev: &str) -> Result<PathBuf, String> {
    let said = run("udisksctl", &["mount", "-b", dev])?;
    // "Mounted /dev/sdb1 at /run/media/user/LABEL."
    said.rsplit_once(" at ")
        .map(|(_, p)| PathBuf::from(p.trim().trim_end_matches('.')))
        .ok_or(said)
}

pub fn unmount(dev: &str) -> Result<(), String> {
    run("udisksctl", &["unmount", "-b", dev]).map(drop).or_else(|e| run("umount", &[dev]).map(drop).map_err(|_| e))
}

/// Unmount, then power the drive off so it can be unplugged.
pub fn eject(dev: &str) -> Result<(), String> {
    unmount(dev).or_else(|e| if is_mounted(dev) { Err(e) } else { Ok(()) })?;
    // Not every device can be powered off (card readers, partitions of internal disks)
    let disk = run("lsblk", &["-no", "pkname", dev]).ok().filter(|s| !s.is_empty()).map(|s| format!("/dev/{}", s.lines().next().unwrap_or_default()));
    let _ = run("udisksctl", &["power-off", "-b", disk.as_deref().unwrap_or(dev)]);
    Ok(())
}

fn is_mounted(dev: &str) -> bool {
    run("lsblk", &["-no", "MOUNTPOINT", dev]).is_ok_and(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"blockdevices":[
      {"name":"nvme0n1","path":"/dev/nvme0n1","label":null,"size":"500G","mountpoint":null,"rm":false,"hotplug":false,"type":"disk","fstype":null,"children":[
        {"name":"nvme0n1p1","path":"/dev/nvme0n1p1","label":"boot","size":"1G","mountpoint":"/boot","rm":false,"hotplug":false,"type":"part","fstype":"vfat"},
        {"name":"nvme0n1p2","path":"/dev/nvme0n1p2","label":null,"size":"499G","mountpoint":"/","rm":false,"hotplug":false,"type":"part","fstype":"ext4"}]},
      {"name":"sda","path":"/dev/sda","label":null,"size":"16G","mountpoint":null,"rm":"1","hotplug":"1","type":"disk","fstype":null,"children":[
        {"name":"sda1","path":"/dev/sda1","label":"STICK","size":"8G","mountpoint":"/run/media/me/STICK","rm":"1","hotplug":"1","type":"part","fstype":"vfat"},
        {"name":"sda2","path":"/dev/sda2","label":null,"size":"8G","mountpoint":null,"rm":"1","hotplug":"1","type":"part","fstype":"ext4"}]},
      {"name":"sdb","path":"/dev/sdb","label":null,"size":"1T","mountpoint":null,"rm":false,"hotplug":false,"type":"disk","fstype":null,"children":[
        {"name":"sdb1","path":"/dev/sdb1","label":"data","size":"1T","mountpoint":null,"rm":false,"hotplug":false,"type":"part","fstype":"ext4"}]}]}"#;

    #[test]
    fn lists_every_partition_but_the_root() {
        let drives = parse_lsblk(SAMPLE, true);
        let mut seen: Vec<(&str, bool, bool)> = drives.iter().map(|p| {
            let d = p.drive.as_ref().unwrap();
            (d.dev.as_str(), d.mounted, d.system)
        }).collect();
        seen.sort();
        assert_eq!(seen, vec![
            ("/dev/nvme0n1p1", true, true),
            ("/dev/sda1", true, false),
            ("/dev/sda2", false, false),
            ("/dev/sdb1", false, false),
        ]);
        let stick = drives.iter().find(|p| p.name == "STICK").unwrap();
        assert_eq!(stick.path, PathBuf::from("/run/media/me/STICK"));
        // Internal disks come before the ones plugged in
        assert!(!drives[0].drive.as_ref().unwrap().removable);
    }

    #[test]
    fn lists_a_windows_partition_but_not_its_boot_ones() {
        let json = r#"{"blockdevices":[
          {"name":"nvme1n1","path":"/dev/nvme1n1","label":null,"size":"1T","mountpoint":null,"mountpoints":[null],"rm":false,"hotplug":false,"type":"disk","fstype":null,"parttype":null,"children":[
            {"name":"nvme1n1p1","path":"/dev/nvme1n1p1","label":"SYSTEM","size":"100M","mountpoint":null,"mountpoints":[null],"rm":false,"hotplug":false,"type":"part","fstype":"vfat","parttype":"c12a7328-f81f-11d2-ba4b-00a0c93ec93b"},
            {"name":"nvme1n1p2","path":"/dev/nvme1n1p2","label":null,"size":"16M","mountpoint":null,"mountpoints":[null],"rm":false,"hotplug":false,"type":"part","fstype":null,"parttype":"e3c9e316-0b5c-4db8-817d-f92df00215ae"},
            {"name":"nvme1n1p3","path":"/dev/nvme1n1p3","label":"Windows","size":"900G","mountpoint":null,"mountpoints":[null],"rm":false,"hotplug":false,"type":"part","fstype":"ntfs","parttype":"ebd0a0a2-b9e5-4433-87c0-68b6b72699c7"},
            {"name":"nvme1n1p4","path":"/dev/nvme1n1p4","label":"Recovery","size":"700M","mountpoint":null,"mountpoints":[null],"rm":false,"hotplug":false,"type":"part","fstype":"ntfs","parttype":"de94bba4-06d1-4d40-a16a-bfd50179d6ac"}]},
          {"name":"nvme0n1","path":"/dev/nvme0n1","label":null,"size":"500G","mountpoint":null,"mountpoints":[null],"rm":false,"hotplug":false,"type":"disk","fstype":null,"parttype":null,"children":[
            {"name":"nvme0n1p1","path":"/dev/nvme0n1p1","label":null,"size":"500G","mountpoint":"/home","mountpoints":["/home","/nix","/"],"rm":false,"hotplug":false,"type":"part","fstype":"btrfs","parttype":"0fc63daf-8483-4772-8e79-3d69d8477de4"}]}]}"#;
        let drives = parse_lsblk(json, true);
        assert!(parse_lsblk(json, false).is_empty());
        let names: Vec<&str> = drives.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Windows"]);
        let windows = drives[0].drive.as_ref().unwrap();
        assert!(!windows.mounted && !windows.removable);
        assert_eq!(drives[0].icon, icons::HARD_DISK);
    }

    #[test]
    fn without_all_lists_user_mounts_and_unmounted_removables_only() {
        let mut seen: Vec<String> = parse_lsblk(SAMPLE, false).iter().map(|p| p.drive.as_ref().unwrap().dev.clone()).collect();
        seen.sort();
        assert_eq!(seen, vec!["/dev/sda1", "/dev/sda2"]);
    }

    #[test]
    fn space_of_the_root() {
        let space = Space::of(Path::new("/")).unwrap();
        assert!(space.total > 0 && space.used <= space.total && space.free <= space.total);
    }
}
