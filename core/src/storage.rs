//! Block devices: discovery (/sys/block), mount/unmount, format, initialise
//! (new GPT partition table), SMART health, and re-mounting after reboot.
//!
//! Functions prefixed `h_` run inside the root helper and re-validate every
//! argument themselves; the rest run in the unprivileged web process.

use std::{
    collections::HashMap,
    ffi::CString,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    privileged::{self, Op},
    util,
};

const SKIP_PREFIXES: &[&str] = &["loop", "ram", "zram", "fd", "sr", "mtdblock", "nbd", "dm-"];
pub const TRASH_DIR: &str = ".rustnas-trash";
pub const UPDATE_DIR: &str = ".rustnas-update";

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
    /// Holds the OS — never mounted, formatted or initialised from the UI.
    pub system: bool,
    pub partitions: Vec<Partition>,
}

pub struct MountInfo {
    pub dev: String,
    pub mountpoint: String,
}

type BlkInfo = HashMap<String, HashMap<String, String>>;

// ---- discovery ----------------------------------------------------------------

/// Decode a Linux dev_t (glibc/musl encoding).
fn dev_major_minor(dev: u64) -> (u64, u64) {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major, minor)
}

/// Whole-disk name holding "/" (e.g. "mmcblk0"), or the configured override.
pub fn system_disk(cfg: &Config) -> Option<String> {
    if !cfg.system_disk.is_empty() {
        return Some(cfg.system_disk.clone());
    }
    let (maj, min) = dev_major_minor(fs::metadata("/").ok()?.dev());
    let p = fs::canonicalize(format!("/sys/dev/block/{maj}:{min}")).ok()?;
    let disk = if p.join("partition").exists() { p.parent()?.to_path_buf() } else { p };
    disk.file_name().map(|n| n.to_string_lossy().into_owned())
}

pub fn valid_devname(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

pub fn read_mounts() -> Vec<MountInfo> {
    parse_mounts(&fs::read_to_string("/proc/self/mounts").unwrap_or_default())
}

pub fn list(cfg: &Config) -> Vec<Disk> {
    let mounts = read_mounts();
    let blk = privileged::call(cfg, Op::Blkid).map(|s| parse_blkid(&s)).unwrap_or_default();
    let sys = system_disk(cfg);
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
        let mut pnames = partition_names(&name);
        pnames.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
        let mut partitions: Vec<Partition> =
            pnames.iter().map(|p| partition(p, &base.join(p), &mounts, &blk)).collect();
        // A disk formatted without a partition table ("superfloppy").
        if partitions.is_empty() && blk.get(&format!("/dev/{name}")).is_some_and(|m| m.contains_key("TYPE")) {
            partitions.push(partition(&name, &base, &mounts, &blk));
        }
        let system = sys.as_deref() == Some(name.as_str())
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

fn partition_names(disk: &str) -> Vec<String> {
    let base = Path::new("/sys/block").join(disk);
    fs::read_dir(&base)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(disk) && base.join(n).join("partition").exists())
                .collect()
        })
        .unwrap_or_default()
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

/// (total, used, available) bytes of a mounted filesystem.
// statvfs field types are u64 on 64-bit Linux but u32 on some 32-bit targets.
#[allow(clippy::useless_conversion)]
pub fn fs_usage3(path: &str) -> Option<(u64, u64, u64)> {
    let c = CString::new(path).ok()?;
    // SAFETY: statvfs fills the zeroed struct; c is a valid NUL-terminated path.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = u64::from(s.f_frsize);
    let total = u64::from(s.f_blocks) * frsize;
    let free = u64::from(s.f_bfree) * frsize;
    let avail = u64::from(s.f_bavail) * frsize;
    Some((total, total.saturating_sub(free), avail))
}

fn fs_usage(path: &str) -> Option<(u64, u64)> {
    fs_usage3(path).map(|(t, u, _)| (t, u))
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
    read_mounts().iter().any(|m| Path::new(&m.mountpoint) == p)
}

/// Mounted data filesystems directly below mount_root.
pub fn data_mounts(cfg: &Config) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = read_mounts()
        .into_iter()
        .map(|m| PathBuf::from(m.mountpoint))
        .filter(|p| p.parent() == Some(cfg.mount_root.as_path()))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The data mount that contains `p` (p must already be normalised).
pub fn data_mount_of(cfg: &Config, p: &Path) -> Option<PathBuf> {
    let rel = p.strip_prefix(&cfg.mount_root).ok()?;
    let top = cfg.mount_root.join(rel.components().next()?);
    is_mountpoint(&top).then_some(top)
}

fn find(cfg: &Config, name: &str) -> Result<(Disk, Partition), String> {
    if !valid_devname(name) {
        return Err("Tên thiết bị không hợp lệ".into());
    }
    let disks = list(cfg);
    for d in &disks {
        let found = d.partitions.iter().find(|p| p.name == name).cloned();
        if let Some(p) = found {
            if d.system {
                return Err("Không thể thao tác trên thẻ nhớ hệ thống".into());
            }
            return Ok((d.clone(), p));
        }
    }
    let present: Vec<String> = disks
        .iter()
        .filter(|d| !d.system)
        .flat_map(|d| d.partitions.iter().map(|p| p.name.clone()).chain(std::iter::once(d.name.clone())))
        .collect();
    Err(format!(
        "Không tìm thấy phân vùng {name} — ổ có thể vừa bị ngắt/cắm lại (nguồn USB yếu?). Đang có: {}. Bấm \"Làm mới\".",
        if present.is_empty() { "không có ổ dữ liệu nào".to_string() } else { present.join(", ") }
    ))
}

/// `blkid` output, completed with filesystem types read straight from the
/// superblocks. BusyBox blkid may be built without TYPE output, and the UI
/// needs the type to offer "Mount" instead of "Format".
pub fn h_blkid() -> String {
    let mut map = parse_blkid(&util::run_ro("blkid", &[]).unwrap_or_default());
    if let Ok(rd) = fs::read_dir("/sys/class/block") {
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if SKIP_PREFIXES.iter().any(|p| name.starts_with(p)) || !valid_devname(&name) {
                continue;
            }
            let dev = format!("/dev/{name}");
            let has_type = map.get(&dev).is_some_and(|kv| kv.contains_key("TYPE"));
            if !has_type {
                if let Some(t) = probe_fs_type(&dev) {
                    map.entry(dev).or_default().insert("TYPE".into(), t.into());
                }
            }
        }
    }
    let mut lines: Vec<String> = map
        .into_iter()
        .filter(|(_, kv)| !kv.is_empty())
        .map(|(dev, kv)| {
            let mut kv: Vec<(String, String)> = kv.into_iter().collect();
            kv.sort();
            let fields: Vec<String> = kv.iter().map(|(k, v)| format!("{k}=\"{}\"", v.replace('"', ""))).collect();
            format!("{dev}: {}", fields.join(" "))
        })
        .collect();
    lines.sort();
    lines.join("\n")
}

/// Identify a filesystem from its first 4 KiB.
fn probe_fs_type(dev: &str) -> Option<&'static str> {
    use std::io::Read;
    let mut b = [0u8; 4096];
    fs::File::open(dev).ok()?.read_exact(&mut b).ok()?;
    fs_type_from_superblock(&b)
}

fn fs_type_from_superblock(b: &[u8; 4096]) -> Option<&'static str> {
    if b[1080..1082] == [0x53, 0xEF] {
        Some("ext4") // ext2/3/4 share the magic; the ext4 driver mounts all three
    } else if &b[3..11] == b"NTFS    " {
        Some("ntfs")
    } else if &b[3..11] == b"EXFAT   " {
        Some("exfat")
    } else if &b[82..90] == b"FAT32   " || &b[54..62] == b"FAT16   " || &b[54..62] == b"FAT12   " {
        Some("vfat")
    } else if &b[4086..4096] == b"SWAPSPACE2" {
        Some("swap")
    } else {
        None
    }
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(32).collect()
}

fn dir_in_use(p: &Path) -> bool {
    is_mountpoint(p) || fs::read_dir(p).map(|mut d| d.next().is_some()).unwrap_or(false)
}

// ---- web-side operations (go through the helper) -------------------------------------

pub fn mount(cfg: &Config, name: &str) -> Result<String, String> {
    let (_disk, part) = find(cfg, name)?;
    if let Some(mp) = &part.mountpoint {
        return Err(format!("{name} đã được mount tại {mp}"));
    }
    let fstype = part.fstype.clone().ok_or("Phân vùng chưa có filesystem — hãy format trước")?;
    let base = part.label.as_deref().map(sanitize).filter(|s| !s.is_empty()).unwrap_or_else(|| name.to_string());
    let mut mp = cfg.mount_root.join(&base);
    if dir_in_use(&mp) {
        mp = cfg.mount_root.join(format!("{base}-{name}"));
    }
    if dir_in_use(&mp) {
        return Err(format!("{} đang được dùng", mp.display()));
    }
    let mps = mp.to_string_lossy().into_owned();
    privileged::call(cfg, Op::Mount { dev: name.into(), mountpoint: mps.clone(), fstype })?;
    if let Some(uuid) = &part.uuid {
        remember(cfg, uuid, &mps);
    }
    Ok(mps)
}

pub fn unmount(cfg: &Config, name: &str, share_paths: &[String]) -> Result<(), String> {
    let (_disk, part) = find(cfg, name)?;
    let mp = part.mountpoint.ok_or_else(|| format!("{name} chưa được mount"))?;
    if let Some(s) = share_paths.iter().find(|s| Path::new(s).starts_with(&mp)) {
        return Err(format!("Chia sẻ {s} đang nằm trên ổ này — hãy xóa chia sẻ trước"));
    }
    privileged::call(cfg, Op::Umount { mountpoint: mp.clone() })?;
    forget(cfg, &mp);
    Ok(())
}

pub fn format(cfg: &Config, name: &str, label: &str, confirm: &str) -> Result<(), String> {
    if confirm != name {
        return Err("Tên xác nhận không khớp — chưa format".into());
    }
    validate_label(label)?;
    let (_disk, part) = find(cfg, name)?;
    if part.mountpoint.is_some() {
        return Err("Hãy tháo (unmount) phân vùng trước khi format".into());
    }
    privileged::call(cfg, Op::Mkfs { dev: name.into(), label: label.trim().into() }).map(|_| ())
}

/// Wipe a whole disk: new GPT table with one ext4 partition.
pub fn initialize(cfg: &Config, disk: &str, label: &str, confirm: &str) -> Result<(), String> {
    if confirm != disk {
        return Err("Tên xác nhận không khớp — chưa khởi tạo".into());
    }
    validate_label(label)?;
    let d = list(cfg).into_iter().find(|d| d.name == disk).ok_or("Không tìm thấy ổ")?;
    if d.system {
        return Err("Không thể khởi tạo thẻ nhớ hệ thống".into());
    }
    if let Some(p) = d.partitions.iter().find(|p| p.mountpoint.is_some()) {
        return Err(format!("Hãy tháo {} trước", p.name));
    }
    privileged::call(cfg, Op::Initialize { disk: disk.into(), label: label.trim().into() }).map(|_| ())
}

pub fn fix_owner(cfg: &Config, mountpoint: &str) -> Result<(), String> {
    privileged::call(cfg, Op::FixOwner { path: mountpoint.into() }).map(|_| ())
}

fn validate_label(label: &str) -> Result<(), String> {
    let label = label.trim();
    if label.len() > 16 || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Nhãn chỉ gồm chữ, số, - và _ (tối đa 16 ký tự)".into());
    }
    Ok(())
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

fn remember(cfg: &Config, uuid: &str, mp: &str) {
    let mut t = load_table(cfg);
    t.mount.retain(|e| e.uuid != uuid && e.mountpoint != mp);
    t.mount.push(MountEntry { uuid: uuid.into(), mountpoint: mp.into() });
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
        if is_mountpoint(Path::new(&e.mountpoint)) {
            continue;
        }
        let found = disks
            .iter()
            .filter(|d| !d.system)
            .flat_map(|d| d.partitions.iter())
            .find(|p| p.uuid.as_deref() == Some(e.uuid.as_str()) && p.mountpoint.is_none());
        let Some(p) = found else { continue };
        let Some(fstype) = p.fstype.clone() else { continue };
        let op = Op::Mount { dev: p.name.clone(), mountpoint: e.mountpoint.clone(), fstype };
        match privileged::call(cfg, op) {
            Ok(_) => n += 1,
            Err(err) => crate::info!("auto-mount {} failed: {err}", e.mountpoint),
        }
    }
    n
}

// ---- SMART -------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, Default)]
pub struct Smart {
    pub disk: String,
    pub available: bool,
    pub passed: Option<bool>,
    pub temperature_c: Option<i64>,
    pub power_on_hours: Option<i64>,
    pub reallocated: Option<i64>,
    pub pending: Option<i64>,
    pub uncorrectable: Option<i64>,
    pub percentage_used: Option<i64>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub message: Option<String>,
}

pub fn smart(cfg: &Config, disk: &str) -> Smart {
    match privileged::call(cfg, Op::Smart { disk: disk.into() }) {
        Ok(json) => parse_smart(disk, &json),
        Err(e) => Smart { disk: disk.into(), message: Some(e), ..Default::default() },
    }
}

pub fn parse_smart(disk: &str, json: &str) -> Smart {
    let mut s = Smart { disk: disk.into(), ..Default::default() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        s.message = Some("Ổ không hỗ trợ SMART (hoặc hộp USB không chuyển tiếp lệnh)".into());
        return s;
    };
    s.passed = v.pointer("/smart_status/passed").and_then(|x| x.as_bool());
    s.available = s.passed.is_some();
    s.temperature_c = v.pointer("/temperature/current").and_then(|x| x.as_i64());
    s.power_on_hours = v.pointer("/power_on_time/hours").and_then(|x| x.as_i64());
    s.model = v.get("model_name").and_then(|x| x.as_str()).map(String::from);
    s.serial = v.get("serial_number").and_then(|x| x.as_str()).map(String::from);
    s.percentage_used = v.pointer("/nvme_smart_health_information_log/percentage_used").and_then(|x| x.as_i64());
    if let Some(table) = v.pointer("/ata_smart_attributes/table").and_then(|x| x.as_array()) {
        let raw = |id: i64| {
            table
                .iter()
                .find(|a| a.get("id").and_then(|x| x.as_i64()) == Some(id))
                .and_then(|a| a.pointer("/raw/value").and_then(|x| x.as_i64()))
        };
        s.reallocated = raw(5);
        s.pending = raw(197);
        s.uncorrectable = raw(198);
    }
    if !s.available {
        s.message = v
            .pointer("/smartctl/messages/0/string")
            .and_then(|x| x.as_str())
            .map(String::from)
            .or(Some("Không đọc được SMART".into()));
    }
    s
}

// ---- helper-side implementations (run as root) ---------------------------------------

fn h_check_data_dev(cfg: &Config, name: &str) -> Result<(), String> {
    if !valid_devname(name) || !Path::new("/sys/class/block").join(name).exists() {
        return Err("Thiết bị không hợp lệ".into());
    }
    if let Some(sys) = system_disk(cfg) {
        if name.starts_with(&sys) {
            return Err("Từ chối: đây là ổ hệ thống".into());
        }
    }
    Ok(())
}

fn h_is_dev_mounted(name: &str) -> bool {
    let dev = format!("/dev/{name}");
    read_mounts().iter().any(|m| m.dev == dev || m.dev.starts_with(&dev))
}

/// mountpoint must be exactly mount_root/<one safe component>.
fn h_check_mountpoint(cfg: &Config, mp: &str) -> Result<PathBuf, String> {
    let p = Path::new(mp);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if p.parent() != Some(cfg.mount_root.as_path()) || name.is_empty() || sanitize(name) != name {
        return Err("Điểm mount không hợp lệ".into());
    }
    Ok(p.to_path_buf())
}

pub fn h_mount(cfg: &Config, dev: &str, mountpoint: &str, fstype: &str) -> Result<String, String> {
    h_check_data_dev(cfg, dev)?;
    let mp = h_check_mountpoint(cfg, mountpoint)?;
    if h_is_dev_mounted(dev) || is_mountpoint(&mp) {
        return Err("Đã được mount".into());
    }
    let owner = format!("uid={},gid={},umask=002", cfg.share_uid, cfg.share_gid);
    let (t, opts) = match fstype {
        "ext4" | "ext3" | "ext2" => ("ext4", "noatime,nodev,nosuid".to_string()),
        "vfat" => ("vfat", format!("noatime,nodev,nosuid,noexec,{owner},utf8")),
        "exfat" => ("exfat", format!("noatime,nodev,nosuid,noexec,{owner}")),
        "ntfs" => ("ntfs3", format!("noatime,nodev,nosuid,noexec,{owner}")),
        other => return Err(format!("Chưa hỗ trợ filesystem {other}")),
    };
    let devpath = format!("/dev/{dev}");
    if t == "ext4" && !cfg.dry_run {
        // Repair what can be repaired automatically after a power cut.
        let (code, _) = util::run_status("e2fsck", &["-p", &devpath])?;
        if code >= 4 {
            return Err(format!("Filesystem trên {dev} có lỗi cần sửa thủ công (e2fsck mã {code})"));
        }
    }
    if !cfg.dry_run {
        fs::create_dir_all(&mp).map_err(|e| format!("{}: {e}", mp.display()))?;
    }
    let mps = mp.to_string_lossy().into_owned();
    util::run(cfg.dry_run, "mount", &["-t", t, "-o", &opts, &devpath, &mps])?;
    crate::info!("mounted {devpath} at {mps}");
    Ok(mps)
}

pub fn h_umount(cfg: &Config, mountpoint: &str) -> Result<String, String> {
    let mp = h_check_mountpoint(cfg, mountpoint)?;
    if !cfg.dry_run && !is_mountpoint(&mp) {
        return Err("Chưa được mount".into());
    }
    let mps = mp.to_string_lossy().into_owned();
    util::run(cfg.dry_run, "sync", &[])?;
    util::run(cfg.dry_run, "umount", &[&mps])?;
    if !cfg.dry_run {
        let _ = fs::remove_dir(&mp);
    }
    crate::info!("unmounted {mps}");
    Ok(String::new())
}

fn mkfs_args<'a>(cfg_owner: &'a str, label: &'a str, dev: &'a str) -> Vec<&'a str> {
    let mut args = vec!["-F", "-q", "-m", "1", "-E", cfg_owner];
    if !label.is_empty() {
        args.extend(["-L", label]);
    }
    args.push(dev);
    args
}

pub fn h_mkfs(cfg: &Config, dev: &str, label: &str) -> Result<String, String> {
    h_check_data_dev(cfg, dev)?;
    validate_label(label)?;
    if h_is_dev_mounted(dev) {
        return Err("Phân vùng đang được mount".into());
    }
    let devpath = format!("/dev/{dev}");
    // The share owner owns the new filesystem root, so SMB/web users can write.
    let ext = format!("lazy_itable_init=1,lazy_journal_init=1,root_owner={}:{}", cfg.share_uid, cfg.share_gid);
    crate::info!("formatting {devpath} as ext4");
    util::run(cfg.dry_run, "mkfs.ext4", &mkfs_args(&ext, label, &devpath)).map(|_| String::new())
}

pub fn h_initialize(cfg: &Config, disk: &str, label: &str) -> Result<String, String> {
    h_check_data_dev(cfg, disk)?;
    validate_label(label)?;
    if !Path::new("/sys/block").join(disk).exists() {
        return Err("Chỉ khởi tạo được cả ổ (không phải phân vùng)".into());
    }
    if h_is_dev_mounted(disk) {
        return Err("Ổ đang có phân vùng được mount".into());
    }
    let devpath = format!("/dev/{disk}");
    crate::info!("initialising {devpath}: new GPT + ext4");
    util::run(cfg.dry_run, "wipefs", &["-a", &devpath])?;
    util::run_input(cfg.dry_run, "sfdisk", &["--wipe", "always", &devpath], "label: gpt\n,,L\n")?;
    let part = if disk.ends_with(|c: char| c.is_ascii_digit()) { format!("{disk}p1") } else { format!("{disk}1") };
    if !cfg.dry_run {
        for _ in 0..20 {
            if Path::new("/dev").join(&part).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
    h_mkfs(cfg, &part, label)?;
    Ok(part)
}

/// Create a share directory and hand root-owned dirs to the share user.
pub fn h_prepare_dir(cfg: &Config, path: &str) -> Result<String, String> {
    let p = crate::files::normalize(cfg, path)?;
    data_mount_of(cfg, &p).ok_or("Thư mục phải nằm trên ổ dữ liệu đã mount")?;
    if cfg.dry_run {
        return Ok(String::new());
    }
    fs::create_dir_all(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let real = fs::canonicalize(&p).map_err(|e| e.to_string())?;
    data_mount_of(cfg, &real).ok_or("Đường dẫn trỏ ra ngoài ổ dữ liệu")?;
    let meta = fs::symlink_metadata(&real).map_err(|e| e.to_string())?;
    if meta.uid() == 0 {
        std::os::unix::fs::chown(&real, Some(cfg.share_uid), Some(cfg.share_gid)).map_err(|e| e.to_string())?;
    }
    Ok(String::new())
}

/// chown -R a whole data disk to the share user (for disks brought from another PC).
pub fn h_fix_owner(cfg: &Config, path: &str) -> Result<String, String> {
    let p = h_check_mountpoint(cfg, path)?;
    if !cfg.dry_run && !is_mountpoint(&p) {
        return Err("Chưa được mount".into());
    }
    let owner = format!("{}:{}", cfg.share_uid, cfg.share_gid);
    let ps = p.to_string_lossy().into_owned();
    // -h: change symlinks themselves, never follow them out of the disk.
    util::run(cfg.dry_run, "chown", &["-R", "-h", &owner, &ps])?;
    util::run(cfg.dry_run, "chmod", &["-R", "u+rwX,g+rwX", &ps]).map(|_| String::new())
}

pub fn h_smart(cfg: &Config, disk: &str) -> Result<String, String> {
    if !valid_devname(disk) || !Path::new("/sys/block").join(disk).exists() {
        return Err("Ổ không hợp lệ".into());
    }
    if cfg.dry_run {
        return Ok(String::new());
    }
    let dev = format!("/dev/{disk}");
    // smartctl's exit code is a bitmask; judge by the JSON instead.
    let (_, out) = util::run_status("smartctl", &["-j", "-a", &dev])?;
    let needs_sat = serde_json::from_str::<serde_json::Value>(&out)
        .map(|v| v.pointer("/smart_status/passed").is_none())
        .unwrap_or(true);
    if needs_sat {
        // Most USB-SATA bridges need the SAT pass-through.
        if let Ok((_, sat)) = util::run_status("smartctl", &["-j", "-a", "-d", "sat", &dev]) {
            if serde_json::from_str::<serde_json::Value>(&sat).is_ok_and(|v| v.pointer("/smart_status/passed").is_some()) {
                return Ok(sat);
            }
        }
    }
    Ok(out)
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

    #[test]
    fn superblock_types() {
        let mut b = [0u8; 4096];
        assert_eq!(fs_type_from_superblock(&b), None);
        b[1080] = 0x53;
        b[1081] = 0xEF;
        assert_eq!(fs_type_from_superblock(&b), Some("ext4"));
        let mut b = [0u8; 4096];
        b[3..11].copy_from_slice(b"NTFS    ");
        assert_eq!(fs_type_from_superblock(&b), Some("ntfs"));
        let mut b = [0u8; 4096];
        b[3..11].copy_from_slice(b"EXFAT   ");
        assert_eq!(fs_type_from_superblock(&b), Some("exfat"));
        let mut b = [0u8; 4096];
        b[82..90].copy_from_slice(b"FAT32   ");
        assert_eq!(fs_type_from_superblock(&b), Some("vfat"));
    }

    #[test]
    fn devt_decoding() {
        assert_eq!(dev_major_minor((179 << 8) | 2), (179, 2));
        assert_eq!(dev_major_minor((8 << 8) | 17), (8, 17));
    }

    #[test]
    fn mountpoint_rules() {
        let cfg = Config::default();
        assert!(h_check_mountpoint(&cfg, "/mnt/data").is_ok());
        assert!(h_check_mountpoint(&cfg, "/mnt/../etc").is_err());
        assert!(h_check_mountpoint(&cfg, "/mnt/a/b").is_err());
        assert!(h_check_mountpoint(&cfg, "/etc").is_err());
        assert!(h_check_mountpoint(&cfg, "/mnt/a b").is_err());
    }

    #[test]
    fn smart_json() {
        let j = r#"{"smart_status":{"passed":true},"temperature":{"current":38},"power_on_time":{"hours":1234},
            "ata_smart_attributes":{"table":[{"id":5,"raw":{"value":0}},{"id":197,"raw":{"value":2}}]}}"#;
        let s = parse_smart("sda", j);
        assert_eq!(s.passed, Some(true));
        assert_eq!(s.temperature_c, Some(38));
        assert_eq!(s.pending, Some(2));
        assert!(!parse_smart("sda", "").available);
    }
}
