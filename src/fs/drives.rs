//! Removable and extra drives: list them with `lsblk`, mount and eject them with `udisksctl`.
use std::path::PathBuf;
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
}

/// Where the desktop and the user put drives; other mount points (`/`, `/boot`, `/nix`...) are not listed.
const MOUNT_ROOTS: &[&str] = &["/run/media", "/media", "/mnt"];

/// Filesystems that are not something to browse.
const NOT_BROWSABLE: &[&str] = &["swap", "LVM2_member", "crypto_LUKS", "linux_raid_member", "zfs_member", "isw_raid_member"];

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

fn walk(node: &Value, inherited_removable: bool, out: &mut Vec<Place>) {
    let removable = inherited_removable || truthy(&node["rm"]) || truthy(&node["hotplug"]);
    let kind = text(node, "type").unwrap_or_default();
    let fstype = text(node, "fstype");
    let mount = text(node, "mountpoint").map(PathBuf::from);
    let has_children = node.get("children").and_then(Value::as_array).is_some_and(|c| !c.is_empty());

    let browsable = fstype.is_some_and(|f| !NOT_BROWSABLE.contains(&f)) && !matches!(kind, "loop") && !has_children;
    let listed = match &mount {
        Some(m) => MOUNT_ROOTS.iter().any(|r| m.starts_with(r)),
        None => removable && browsable,
    };
    if browsable && listed && let Some(dev) = text(node, "path") {
        let label = text(node, "label").map(str::to_owned).or_else(|| {
            mount.as_ref().and_then(|m| m.file_name()).map(|n| n.to_string_lossy().into_owned())
        });
        let name = label.unwrap_or_else(|| trf("{} Volume", &[&text(node, "size").unwrap_or("Drive")]));
        let mut place = Place::new(name, mount.clone().unwrap_or_default(), icons::USB, PlaceGroup::Devices);
        place.drive = Some(Drive { dev: dev.to_owned(), mounted: mount.is_some(), removable });
        out.push(place);
    }
    for child in node.get("children").and_then(Value::as_array).into_iter().flatten() {
        walk(child, removable, out);
    }
}

pub fn parse_lsblk(json: &str) -> Vec<Place> {
    let mut out = Vec::new();
    if let Ok(root) = serde_json::from_str::<Value>(json) {
        for node in root["blockdevices"].as_array().into_iter().flatten() {
            walk(node, false, &mut out);
        }
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Drives to show, or nothing when `lsblk` is missing.
pub fn list() -> Vec<Place> {
    let output = Command::new("lsblk").args(["-J", "-o", "NAME,PATH,LABEL,SIZE,MOUNTPOINT,RM,HOTPLUG,TYPE,FSTYPE"]).output();
    match output {
        Ok(o) if o.status.success() => parse_lsblk(&String::from_utf8_lossy(&o.stdout)),
        _ => from_mount_roots(),
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
    fn lists_user_mounts_and_unmounted_removables_only() {
        let drives = parse_lsblk(SAMPLE);
        let mut seen: Vec<(&str, bool)> = drives.iter().map(|p| {
            let d = p.drive.as_ref().unwrap();
            (d.dev.as_str(), d.mounted)
        }).collect();
        seen.sort();
        assert_eq!(seen, vec![("/dev/sda1", true), ("/dev/sda2", false)]);
        let stick = drives.iter().find(|p| p.name == "STICK").unwrap();
        assert_eq!(stick.path, PathBuf::from("/run/media/me/STICK"));
    }
}
