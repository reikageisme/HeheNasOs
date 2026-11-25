//! Web file manager with a per-disk trash (`<disk>/.hehenas-trash`).
//!
//! Every path is confined to a mounted data disk below `mount_root`:
//! normalised lexically (no `..`), then canonicalised and checked again so a
//! symlink on the disk cannot point the file manager at the rest of the OS.

use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    storage::{self, TRASH_DIR, UPDATE_DIR},
    util,
};

const TRASH_DAYS: u64 = 30;

/// Absolute, `..`-free, without trailing slash.
pub fn normalize_str(p: &str) -> Result<String, String> {
    let p = p.trim();
    if !p.starts_with('/') || p.chars().any(char::is_control) || p.len() > 4096 {
        return Err("Đường dẫn không hợp lệ".into());
    }
    let mut out = PathBuf::from("/");
    for c in Path::new(p).components() {
        match c {
            Component::RootDir => {}
            Component::Normal(x) => out.push(x),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err("Đường dẫn không được chứa . hoặc ..".into())
            }
        }
    }
    Ok(out.to_string_lossy().into_owned())
}

/// Normalised path that lies strictly below mount_root.
pub fn normalize(cfg: &Config, p: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(normalize_str(p)?);
    match p.strip_prefix(&cfg.mount_root) {
        Ok(rel) if rel.components().next().is_some() => Ok(p),
        _ => Err(format!("Đường dẫn phải nằm trong {}", cfg.mount_root.display())),
    }
}

pub fn valid_name(n: &str) -> Result<(), String> {
    if n.is_empty()
        || n.len() > 255
        || n == "."
        || n == ".."
        || n.contains('/')
        || n.chars().any(char::is_control)
        || n.starts_with(".hehenas-")
    {
        return Err("Tên file không hợp lệ".into());
    }
    Ok(())
}

/// Resolve an existing path: returns (real path, data mount it lives on).
pub fn resolve(cfg: &Config, p: &str) -> Result<(PathBuf, PathBuf), String> {
    let p = normalize(cfg, p)?;
    let mount = storage::data_mount_of(cfg, &p).ok_or("Ổ dữ liệu chưa được mount")?;
    let real = fs::canonicalize(&p).map_err(|_| "Không tìm thấy file/thư mục".to_string())?;
    if !real.starts_with(&mount) {
        return Err("Đường dẫn trỏ ra ngoài ổ dữ liệu".into());
    }
    let rel = real.strip_prefix(&mount).unwrap_or(Path::new(""));
    if rel.components().next().is_some_and(|c| {
        let s = c.as_os_str();
        s == TRASH_DIR || s == UPDATE_DIR
    }) {
        return Err("Thư mục hệ thống của HeheNAS".into());
    }
    Ok((real, mount))
}

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub symlink: bool,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Serialize)]
pub struct Root {
    pub name: String,
    pub path: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

#[derive(Serialize)]
pub struct Listing {
    pub path: String,
    pub mount: String,
    pub entries: Vec<Entry>,
}

fn mtime(m: &fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn roots(cfg: &Config) -> Vec<Root> {
    storage::data_mounts(cfg)
        .into_iter()
        .map(|p| {
            let ps = p.to_string_lossy().into_owned();
            let (total, used, avail) = storage::fs_usage3(&ps).unwrap_or((0, 0, 0));
            Root { name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), path: ps, total, used, avail }
        })
        .collect()
}

pub fn list(cfg: &Config, p: &str) -> Result<Listing, String> {
    let (real, mount) = resolve(cfg, p)?;
    let at_root = real == mount;
    let mut entries = Vec::new();
    for e in fs::read_dir(&real).map_err(|e| e.to_string())?.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().into_owned();
        if at_root && (name == TRASH_DIR || name == UPDATE_DIR || name == "lost+found") {
            continue;
        }
        let Ok(lm) = e.path().symlink_metadata() else { continue };
        let symlink = lm.file_type().is_symlink();
        let m = if symlink { fs::metadata(e.path()).unwrap_or(lm) } else { lm };
        entries.push(Entry { name, dir: m.is_dir(), symlink, size: if m.is_dir() { 0 } else { m.len() }, mtime: mtime(&m) });
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(Listing { path: real.to_string_lossy().into_owned(), mount: mount.to_string_lossy().into_owned(), entries })
}

pub fn mkdir(cfg: &Config, parent: &str, name: &str) -> Result<(), String> {
    valid_name(name)?;
    let (real, _) = resolve(cfg, parent)?;
    fs::create_dir(real.join(name)).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => "Đã tồn tại".to_string(),
        std::io::ErrorKind::PermissionDenied => "Không có quyền ghi (thử 'Sửa quyền' ở trang Ổ đĩa)".to_string(),
        _ => e.to_string(),
    })
}

pub fn rename(cfg: &Config, path: &str, new_name: &str) -> Result<(), String> {
    valid_name(new_name)?;
    let (real, mount) = resolve(cfg, path)?;
    if real == mount {
        return Err("Không đổi tên được thư mục gốc của ổ".into());
    }
    let target = real.with_file_name(new_name);
    if target.symlink_metadata().is_ok() {
        return Err("Tên mới đã tồn tại".into());
    }
    fs::rename(&real, &target).map_err(|e| e.to_string())
}

/// Path the upload is written to; refuses to overwrite unless asked.
pub fn upload_target(cfg: &Config, dir: &str, name: &str, overwrite: bool) -> Result<(PathBuf, PathBuf), String> {
    valid_name(name)?;
    let (real, _) = resolve(cfg, dir)?;
    if !real.is_dir() {
        return Err("Đích không phải thư mục".into());
    }
    let target = real.join(name);
    if !overwrite && target.symlink_metadata().is_ok() {
        return Err(format!("'{name}' đã tồn tại"));
    }
    if target.symlink_metadata().is_ok_and(|m| m.is_dir() || m.file_type().is_symlink()) {
        return Err("Không ghi đè được thư mục/symlink".into());
    }
    let part = real.join(format!(".{name}.hehenas-part"));
    Ok((target, part))
}

// ---- trash ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct TrashInfo {
    original: String,
    deleted_at: u64,
}

#[derive(Serialize)]
pub struct TrashItem {
    pub id: String,
    pub mount: String,
    pub name: String,
    pub original: String,
    pub deleted_at: u64,
    pub dir: bool,
    pub size: u64,
}

fn trash_dirs(mount: &Path) -> (PathBuf, PathBuf) {
    let t = mount.join(TRASH_DIR);
    (t.join("files"), t.join("info"))
}

/// Move paths into their disk's trash (same filesystem: a cheap rename).
pub fn delete(cfg: &Config, paths: &[String]) -> Result<usize, String> {
    let mut n = 0;
    for p in paths {
        let (real, mount) = resolve(cfg, p)?;
        if real == mount {
            return Err("Không xóa được thư mục gốc của ổ".into());
        }
        let (files, info) = trash_dirs(&mount);
        fs::create_dir_all(&files).map_err(|e| e.to_string())?;
        fs::create_dir_all(&info).map_err(|e| e.to_string())?;
        let id = format!("{}-{}", util::now_secs(), util::random_hex(4));
        let meta = TrashInfo { original: real.to_string_lossy().into_owned(), deleted_at: util::now_secs() };
        util::write_atomic(&info.join(format!("{id}.json")), &serde_json::to_vec(&meta).unwrap_or_default(), 0o644)?;
        fs::rename(&real, files.join(&id)).map_err(|e| format!("{}: {e}", real.display()))?;
        n += 1;
    }
    Ok(n)
}

pub fn trash_list(cfg: &Config) -> Vec<TrashItem> {
    let mut out = Vec::new();
    for mount in storage::data_mounts(cfg) {
        let (files, info) = trash_dirs(&mount);
        let Ok(rd) = fs::read_dir(&info) else { continue };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json") else { continue };
            let Some(meta) = fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<TrashInfo>(&b).ok()) else {
                continue;
            };
            let Ok(m) = files.join(id).symlink_metadata() else { continue };
            out.push(TrashItem {
                id: id.into(),
                mount: mount.to_string_lossy().into_owned(),
                name: Path::new(&meta.original).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                original: meta.original,
                deleted_at: meta.deleted_at,
                dir: m.is_dir(),
                size: if m.is_dir() { 0 } else { m.len() },
            });
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.deleted_at));
    out
}

fn trash_item(cfg: &Config, mount: &str, id: &str) -> Result<(PathBuf, PathBuf, TrashInfo), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Mục không hợp lệ".into());
    }
    let mount = PathBuf::from(normalize_str(mount)?);
    if !storage::data_mounts(cfg).contains(&mount) {
        return Err("Ổ không hợp lệ".into());
    }
    let (files, info) = trash_dirs(&mount);
    let info_path = info.join(format!("{id}.json"));
    let meta: TrashInfo = fs::read(&info_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Không tìm thấy trong thùng rác")?;
    Ok((files.join(id), info_path, meta))
}

pub fn trash_restore(cfg: &Config, mount: &str, id: &str) -> Result<String, String> {
    let (item, info_path, meta) = trash_item(cfg, mount, id)?;
    let orig = PathBuf::from(normalize_str(&meta.original)?);
    // The original must still be on the same disk.
    let disk = storage::data_mount_of(cfg, &orig).ok_or("Ổ gốc không còn mount")?;
    if disk.as_path() != Path::new(&normalize_str(mount)?) {
        return Err("Ổ gốc không khớp".into());
    }
    let parent = orig.parent().ok_or("Đường dẫn gốc không hợp lệ")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut target = orig.clone();
    if target.symlink_metadata().is_ok() {
        let name = orig.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        target = orig.with_file_name(format!("{name} (khôi phục {})", util::now_secs()));
    }
    fs::rename(&item, &target).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(info_path);
    Ok(target.to_string_lossy().into_owned())
}

/// Permanently delete one item (id) or everything (id = None).
pub fn trash_purge(cfg: &Config, mount: Option<&str>, id: Option<&str>) -> Result<usize, String> {
    if let (Some(m), Some(id)) = (mount, id) {
        let (item, info_path, _) = trash_item(cfg, m, id)?;
        remove_any(&item)?;
        let _ = fs::remove_file(info_path);
        return Ok(1);
    }
    let mut n = 0;
    for it in trash_list(cfg) {
        if let Ok((item, info_path, _)) = trash_item(cfg, &it.mount, &it.id) {
            if remove_any(&item).is_ok() {
                let _ = fs::remove_file(info_path);
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Drop trash items older than 30 days.
pub fn trash_expire(cfg: &Config) -> usize {
    let cutoff = util::now_secs().saturating_sub(TRASH_DAYS * 86400);
    let mut n = 0;
    for it in trash_list(cfg).into_iter().filter(|it| it.deleted_at < cutoff) {
        if trash_purge(cfg, Some(&it.mount), Some(&it.id)).is_ok() {
            n += 1;
        }
    }
    n
}

fn remove_any(p: &Path) -> Result<(), String> {
    let m = p.symlink_metadata().map_err(|e| e.to_string())?;
    if m.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalising() {
        assert_eq!(normalize_str("/mnt/data//x/").unwrap(), "/mnt/data/x");
        assert!(normalize_str("/mnt/../etc").is_err());
        assert!(normalize_str("mnt/x").is_err());
        let cfg = Config::default();
        assert!(normalize(&cfg, "/mnt").is_err());
        assert!(normalize(&cfg, "/etc/passwd").is_err());
        assert!(normalize(&cfg, "/mnt/data/a").is_ok());
    }

    #[test]
    fn names() {
        assert!(valid_name("phim 2024.mkv").is_ok());
        for bad in ["", ".", "..", "a/b", ".hehenas-trash", "x\n"] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }
}
