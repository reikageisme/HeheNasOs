//! Block devices: discovery from /sys/block, mounting data disks below
//! `mount_root`, formatting to ext4, and re-mounting them after a reboot.

use std::{
    collections::HashMap,
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{config::Config, util};

const SKIP_PREFIXES: &[&str] = &["loop", "ram", "zram", "fd", "sr", "mtdblock", "nbd"];

#[derive(Serialize, Clone, Debug)]
pub struct Partition {
    pub name: String,
    pub dev: String,
    pub size_bytes: u64,
    pub fstype: Option<String>,
    pub label: Option<String>,
    pub uuid: Option<String>,
    pub mountpoint: Option<String>,
    pub total_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Disk {
    pub name: String,
    pub dev: String,
    pub model: String,
    pub size_bytes: u64,
    pub transport: String,
    pub rotational: bool,
    pub removable: bool,
    /// Holds the OS (SD card) — read-only in the UI.
    pub system: bool,
    pub partitions: Vec<Partition>,
}

pub struct MountInfo {
    pub dev: String,
    pub mountpoint: String,
}

type BlkInfo = HashMap<String, HashMap<String, String>>;

pub fn list(cfg: &Config) -> Vec<Disk> {
    let mounts = parse_mounts(&fs::read_to_string("/proc/self/mounts").unwrap_or_default());
    let blk = util::run_ro("blkid", &[]).map(|s| parse_blkid(&s)).unwrap_or_default();
    let mut names: Vec<String> = match fs::read_dir("/sys/block") {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !SKIP_PREFIXES.iter().any(|p| n.starts_with(p)))
            .collect(),
        Err(_) => return Vec::new(),
    };
    names.sort();

    let mut disks = Vec::new();
    for name in names {
        let base = Path::new("/sys/block").join(&name);
        let size_bytes = sectors(&base);
        if size_bytes == 0 {
            continue; // empty card reader slot etc.
        }
        let mut pnames: Vec<String> = fs::read_dir(&base)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with(name.as_str()) && base.join(n).join("partition").exists())
                    .collect()
            })
            .unwrap_or_default();
        pnames.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
        let mut partitions: Vec<Partition> =
            pnames.iter().map(|p| partition(p, &base.join(p), &mounts, &blk)).collect();
        // A disk formatted without a partition table ("superfloppy").
        if partitions.is_empty() && blk.contains_key(&format!("/dev/{name}")) {
            partitions.push(partition(&name, &base, &mounts, &blk));
        }

        let system = name == cfg.system_disk
            || partitions.iter().any(|p| {
                p.mountpoint.as_deref().is_some_and(|m| {
                    m == "/" || m.starts_with("/boot") || Path::new(m) == cfg.state_dir.as_path()
                })
            });
        let model = ["device/model", "device/name"]
            .iter()
            .find_map(|f| util::read_trim(base.join(f)).filter(|s| !s.is_empty()))
            .unwrap_or_default();
        let vendor = util::read_trim(base.join("device/vendor")).unwrap_or_default();
        let model = if vendor.is_empty() || model.starts_with(&vendor) { model } else { format!("{vendor} {model}") };
        let real = fs::canonicalize(&base).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let transport = if name.starts_with("mmcblk") {
            "sd"
        } else if real.contains("/usb") {
            "usb"
        } else if name.starts_with("nvme") {
            "nvme"
        } else {
            "sata"
        };
        disks.push(Disk {
            dev: format!("/dev/{name}"),
            model,
            size_bytes,
            transport: transport.into(),
            rotational: util::read_trim(base.join("queue/rotational")).as_deref() == Some("1"),
            removable: util::read_trim(base.join("removable")).as_deref() == Some("1"),
            system,
            partitions,
            name,
        });
    }
    disks
}

fn sectors(sys: &Path) -> u64 {
    util::read_trim(sys.join("size")).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) * 512
}

fn partition(name: &str, sys: &Path, mounts: &[MountInfo], blk: &BlkInfo) -> Partition {
    let dev = format!("/dev/{name}");
    let info = blk.get(&dev);
    let get = |k: &str| info.and_then(|m| m.get(k)).cloned().filter(|s| !s.is_empty());
    let mountpoint = mounts.iter().find(|m| m.dev == dev).map(|m| m.mountpoint.clone());
    let usage = mountpoint.as_deref().and_then(fs_usage);
    Partition {
        name: name.into(),
        size_bytes: sectors(sys),
        fstype: get("TYPE"),
        label: get("LABEL"),
        uuid: get("UUID"),
        total_bytes: usage.map(|u| u.0),
        used_bytes: usage.map(|u| u.1),
        mountpoint,
        dev,
    }
}

/// (total, used) bytes of a mounted filesystem.
// statvfs field types are u64 on 64-bit Linux but u32 on some 32-bit targets.
#[allow(clippy::useless_conversion)]
fn fs_usage(path: &str) -> Option<(u64, u64)> {
    let c = CString::new(path).ok()?;
    // SAFETY: statvfs fills the zeroed struct; c is a valid NUL-terminated path.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = u64::from(s.f_frsize);
    let total = u64::from(s.f_blocks) * frsize;
    let free = u64::from(s.f_bfree) * frsize;
    Some((total, total.saturating_sub(free)))
}

pub fn parse_mounts(s: &str) -> Vec<MountInfo> {
    s.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some(MountInfo { dev: unescape(f.next()?), mountpoint: unescape(f.next()?) })
        })
        .collect()
}

/// /proc/mounts escapes space, tab, newline and backslash as \ooo octal.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses `blkid` output: `/dev/sda1: LABEL="data" UUID="..." TYPE="ext4"`.
pub fn parse_blkid(out: &str) -> BlkInfo {
    let mut map = HashMap::new();
    for line in out.lines() {
        let Some((dev, mut rest)) = line.split_once(": ") else { continue };
        let mut kv = HashMap::new();
        while let Some(eq) = rest.find("=\"") {
            let key = rest[..eq].trim().to_string();
            let after = &rest[eq + 2..];
            let Some(end) = after.find('"') else { break };
            kv.insert(key, after[..end].to_string());
            rest = &after[end + 1..];
        }
        map.insert(dev.trim().to_string(), kv);
    }
    map
}

pub fn is_mountpoint(p: &Path) -> bool {
    parse_mounts(&fs::read_to_string("/proc/self/mounts").unwrap_or_default())
        .iter()
        .any(|m| Path::new(&m.mountpoint) == p)
}

fn find(cfg: &Config, name: &str) -> Result<(Disk, Partition), String> {
    let valid = !name.is_empty()
        && name.len() <= 32
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if !valid {
        return Err("Tên thiết bị không hợp lệ".into());
    }
    for d in list(cfg) {
        let found = d.partitions.iter().find(|p| p.name == name).cloned();
        if let Some(p) = found {
            if d.system {
                return Err("Không thể thao tác trên thẻ nhớ hệ thống".into());
            }
            return Ok((d, p));
        }
    }
    Err(format!("Không tìm thấy phân vùng {name}"))
}

fn mount_opts(cfg: &Config, fstype: &str) -> Result<(&'static str, String), String> {
    // Non-POSIX filesystems get the share owner so SMB users can write.
    let owner = format!("uid={},gid={},umask=002", cfg.share_uid, cfg.share_gid);
    Ok(match fstype {
        "ext4" | "ext3" | "ext2" => ("ext4", "noatime".into()),
        "vfat" => ("vfat", format!("noatime,{owner},utf8")),
        "exfat" => ("exfat", format!("noatime,{owner}")),
        "ntfs" => ("ntfs3", format!("noatime,{owner}")),
        other => return Err(format!("Chưa hỗ trợ filesystem {other}")),
    })
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(32).collect()
}

fn dir_in_use(p: &Path) -> bool {
    is_mountpoint(p) || fs::read_dir(p).map(|mut d| d.next().is_some()).unwrap_or(false)
}

fn do_mount(cfg: &Config, part: &Partition, mp: &Path) -> Result<(), String> {
    let fstype = part.fstype.as_deref().ok_or("Phân vùng chưa có filesystem — hãy format trước")?;
    let (t, opts) = mount_opts(cfg, fstype)?;
    if !cfg.dry_run {
        fs::create_dir_all(mp).map_err(|e| format!("{}: {e}", mp.display()))?;
    }
    let mps = mp.to_string_lossy().into_owned();
    util::run(cfg.dry_run, "mount", &["-t", t, "-o", opts.as_str(), part.dev.as_str(), mps.as_str()])?;
    crate::info!("mounted {} at {mps}", part.dev);
    Ok(())
}

/// Mounts a data partition at /mnt/<label> and remembers it for next boot.
pub fn mount(cfg: &Config, name: &str) -> Result<String, String> {
    let (_disk, part) = find(cfg, name)?;
    if let Some(mp) = &part.mountpoint {
        return Err(format!("{name} đã được mount tại {mp}"));
    }
    let base = part.label.as_deref().map(sanitize).filter(|s| !s.is_empty()).unwrap_or_else(|| name.to_string());
    let mut mp = cfg.mount_root.join(&base);
    if dir_in_use(&mp) {
        mp = cfg.mount_root.join(format!("{base}-{name}"));
    }
    if dir_in_use(&mp) {
        return Err(format!("{} đang được dùng", mp.display()));
    }
    do_mount(cfg, &part, &mp)?;
    if let Some(uuid) = &part.uuid {
        remember(cfg, uuid, &mp);
    }
    Ok(mp.to_string_lossy().into_owned())
}

pub fn unmount(cfg: &Config, name: &str, share_paths: &[String]) -> Result<(), String> {
    let (_disk, part) = find(cfg, name)?;
    let mp = part.mountpoint.ok_or_else(|| format!("{name} chưa được mount"))?;
    let mpp = Path::new(&mp);
    if !mpp.starts_with(&cfg.mount_root) {
        return Err(format!("Chỉ tháo được ổ mount trong {}", cfg.mount_root.display()));
    }
    if let Some(s) = share_paths.iter().find(|s| Path::new(s).starts_with(mpp)) {
        return Err(format!("Chia sẻ {s} đang nằm trên ổ này — hãy xóa chia sẻ trước"));
    }
    util::run(cfg.dry_run, "sync", &[])?;
    util::run(cfg.dry_run, "umount", &[mp.as_str()])?;
    if !cfg.dry_run {
        let _ = fs::remove_dir(mpp);
    }
    forget(cfg, &mp);
    crate::info!("unmounted {mp}");
    Ok(())
}

pub fn format(cfg: &Config, name: &str, label: &str, confirm: &str) -> Result<(), String> {
    if confirm != name {
        return Err("Tên xác nhận không khớp — chưa format".into());
    }
    let label = label.trim();
    if label.len() > 16 || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Nhãn chỉ gồm chữ, số, - và _ (tối đa 16 ký tự)".into());
    }
    let (_disk, part) = find(cfg, name)?;
    if part.mountpoint.is_some() {
        return Err("Hãy tháo (unmount) phân vùng trước khi format".into());
    }
    let mut args = vec!["-F", "-q", "-m", "1", "-E", "lazy_itable_init=1,lazy_journal_init=1"];
    if !label.is_empty() {
        args.extend(["-L", label]);
    }
    args.push(part.dev.as_str());
    crate::info!("formatting {} as ext4", part.dev);
    util::run(cfg.dry_run, "mkfs.ext4", &args).map(|_| ())
}

// ---- remembered mounts (state_dir/mounts.toml) ------------------------------

#[derive(Serialize, Deserialize, Default)]
struct MountTable {
    #[serde(default)]
    mount: Vec<MountEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct MountEntry {
    uuid: String,
    mountpoint: String,
}

fn table_path(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("mounts.toml")
}

fn load_table(cfg: &Config) -> MountTable {
    fs::read_to_string(table_path(cfg)).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

fn save_table(cfg: &Config, t: &MountTable) {
    let res = toml::to_string(t)
        .map_err(|e| e.to_string())
        .and_then(|s| util::write_atomic(&table_path(cfg), s.as_bytes(), 0o600));
    if let Err(e) = res {
        crate::info!("saving mount table: {e}");
    }
}

fn remember(cfg: &Config, uuid: &str, mp: &Path) {
    let mut t = load_table(cfg);
    let mps = mp.to_string_lossy().into_owned();
    t.mount.retain(|e| e.uuid != uuid && e.mountpoint != mps);
    t.mount.push(MountEntry { uuid: uuid.into(), mountpoint: mps });
    save_table(cfg, &t);
}

fn forget(cfg: &Config, mp: &str) {
    let mut t = load_table(cfg);
    t.mount.retain(|e| e.mountpoint != mp);
    save_table(cfg, &t);
}

/// Re-mount remembered disks (by filesystem UUID). Returns how many were mounted.
pub fn restore_mounts(cfg: &Config) -> usize {
    let t = load_table(cfg);
    if t.mount.is_empty() {
        return 0;
    }
    let disks = list(cfg);
    let mut n = 0;
    for e in &t.mount {
        let mp = Path::new(&e.mountpoint);
        if !mp.starts_with(&cfg.mount_root) || is_mountpoint(mp) {
            continue;
        }
        let found = disks
            .iter()
            .filter(|d| !d.system)
            .flat_map(|d| d.partitions.iter())
            .find(|p| p.uuid.as_deref() == Some(e.uuid.as_str()) && p.mountpoint.is_none());
        if let Some(p) = found {
            match do_mount(cfg, p, mp) {
                Ok(()) => n += 1,
                Err(err) => crate::info!("auto-mount {} failed: {err}", e.mountpoint),
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blkid_lines() {
        let m = parse_blkid("/dev/sda1: LABEL=\"My Disk\" UUID=\"abcd-1234\" TYPE=\"ext4\"\n/dev/mmcblk0p1: TYPE=\"vfat\"\n");
        assert_eq!(m["/dev/sda1"]["LABEL"], "My Disk");
        assert_eq!(m["/dev/sda1"]["TYPE"], "ext4");
        assert_eq!(m["/dev/mmcblk0p1"]["TYPE"], "vfat");
    }

    #[test]
    fn mounts_unescape() {
        let m = parse_mounts("/dev/sda1 /mnt/my\\040disk ext4 rw 0 0\n");
        assert_eq!(m[0].mountpoint, "/mnt/my disk");
        assert_eq!(unescape("a\\134b"), "a\\b");
        assert_eq!(unescape("end\\04"), "end\\04");
    }
}
