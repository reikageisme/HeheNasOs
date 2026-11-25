//! SMB shares and SMB users. The source of truth is state_dir/shares.toml;
//! the ksmbd (smb.conf-compatible) config is regenerated from it on change.
//!
//! ksmbd is the in-kernel SMB3 server: a few MB of RAM instead of the
//! 25–65 MB Samba needs, which matters on a 1 GB Raspberry Pi 3B.

use std::{
    fmt::Write as _,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{config::Config, network, storage, util};

const RESERVED: &[&str] = &["global", "homes", "printers", "print$", "ipc$"];
const MAX_SHARES: usize = 64;
const MAX_USERS: usize = 64;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Share {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub guest_ok: bool,
    /// Empty = every SMB user may connect.
    #[serde(default)]
    pub valid_users: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Data {
    workgroup: String,
    smb_users: Vec<String>,
    shares: Vec<Share>,
}

impl Default for Data {
    fn default() -> Self {
        Self { workgroup: "WORKGROUP".into(), smb_users: Vec::new(), shares: Vec::new() }
    }
}

pub struct ShareStore {
    file: PathBuf,
    data: Data,
}

impl ShareStore {
    pub fn load(cfg: &Config) -> Result<Self, String> {
        let file = cfg.state_dir.join("shares.toml");
        let data = match fs::read_to_string(&file) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", file.display()))?,
            Err(_) => Data::default(),
        };
        Ok(Self { file, data })
    }

    pub fn shares(&self) -> &[Share] {
        &self.data.shares
    }

    pub fn users(&self) -> &[String] {
        &self.data.smb_users
    }

    pub fn workgroup(&self) -> &str {
        &self.data.workgroup
    }

    pub fn paths(&self) -> Vec<String> {
        self.data.shares.iter().map(|s| s.path.clone()).collect()
    }

    /// Persist `next`, adopt it, then regenerate the SMB config.
    fn commit(&mut self, cfg: &Config, next: Data) -> Result<(), String> {
        let s = toml::to_string_pretty(&next).map_err(|e| e.to_string())?;
        util::write_atomic(&self.file, s.as_bytes(), 0o600)?;
        self.data = next;
        self.apply(cfg)
    }

    pub fn upsert(&mut self, cfg: &Config, mut s: Share) -> Result<(), String> {
        validate_share_name(&s.name)?;
        s.comment = s.comment.chars().filter(|c| !c.is_control()).take(80).collect::<String>().trim().to_string();
        s.path = validate_path(cfg, &s.path)?;
        s.valid_users.sort();
        s.valid_users.dedup();
        if let Some(u) = s.valid_users.iter().find(|u| !self.data.smb_users.contains(u)) {
            return Err(format!("Người dùng SMB '{u}' không tồn tại"));
        }
        if s.guest_ok {
            s.valid_users.clear();
        }
        prepare_dir(cfg, Path::new(&s.path))?;

        let mut next = self.data.clone();
        match next.shares.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&s.name)) {
            Some(existing) => *existing = s,
            None => {
                if next.shares.len() >= MAX_SHARES {
                    return Err("Đã đạt số lượng chia sẻ tối đa".into());
                }
                next.shares.push(s);
            }
        }
        next.shares.sort_by_key(|x| x.name.to_lowercase());
        self.commit(cfg, next)
    }

    pub fn delete(&mut self, cfg: &Config, name: &str) -> Result<(), String> {
        let mut next = self.data.clone();
        let before = next.shares.len();
        next.shares.retain(|s| !s.name.eq_ignore_ascii_case(name));
        if next.shares.len() == before {
            return Err(format!("Không có chia sẻ '{name}'"));
        }
        self.commit(cfg, next)
    }

    /// Add a user or change their password.
    pub fn set_user(&mut self, cfg: &Config, user: &str, password: &str) -> Result<(), String> {
        validate_smb_user(user)?;
        let n = password.chars().count();
        if !(6..=128).contains(&n) || password.chars().any(char::is_control) {
            return Err("Mật khẩu SMB cần 6–128 ký tự, không chứa ký tự điều khiển".into());
        }
        let exists = self.data.smb_users.iter().any(|u| u == user);
        if !exists && self.data.smb_users.len() >= MAX_USERS {
            return Err("Đã đạt số lượng người dùng tối đa".into());
        }
        // ksmbd-tools: -a add / -u update; works with both 3.4 and 3.5 CLIs.
        let flag = if exists { "-u" } else { "-a" };
        util::run(cfg.dry_run, "ksmbd.adduser", &[flag, user, "-p", password])?;
        if exists {
            return self.apply(cfg);
        }
        let mut next = self.data.clone();
        next.smb_users.push(user.into());
        next.smb_users.sort();
        self.commit(cfg, next)
    }

    pub fn delete_user(&mut self, cfg: &Config, user: &str) -> Result<(), String> {
        if !self.data.smb_users.iter().any(|u| u == user) {
            return Err(format!("Không có người dùng '{user}'"));
        }
        // Removing a user from a share's list could silently open it to
        // everyone, so refuse instead.
        let used: Vec<&str> = self
            .data
            .shares
            .iter()
            .filter(|s| s.valid_users.iter().any(|u| u == user))
            .map(|s| s.name.as_str())
            .collect();
        if !used.is_empty() {
            return Err(format!("'{user}' đang được gán cho chia sẻ: {} — hãy sửa chia sẻ trước", used.join(", ")));
        }
        util::run(cfg.dry_run, "ksmbd.adduser", &["-d", user])?;
        let mut next = self.data.clone();
        next.smb_users.retain(|u| u != user);
        self.commit(cfg, next)
    }

    pub fn render(&self, cfg: &Config, hostname: &str) -> String {
        let mut netbios: String =
            hostname.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(15).collect();
        if netbios.is_empty() {
            netbios = "RUSTNAS".into();
        }
        let any_guest = self.data.shares.iter().any(|s| s.guest_ok);
        let yn = |b: bool| if b { "yes" } else { "no" };

        let mut o = String::new();
        let _ = writeln!(o, "; Generated by rustnasd — manual edits will be overwritten.");
        let _ = writeln!(o, "[global]");
        let _ = writeln!(o, "\tnetbios name = {}", netbios.to_uppercase());
        let _ = writeln!(o, "\tworkgroup = {}", self.data.workgroup);
        let _ = writeln!(o, "\tserver string = RustNAS");
        let _ = writeln!(o, "\tmap to guest = {}", if any_guest { "bad user" } else { "never" });
        let _ = writeln!(o, "\tguest account = {}", cfg.share_user);
        for s in &self.data.shares {
            let _ = writeln!(o, "\n[{}]", s.name);
            let _ = writeln!(o, "\tpath = {}", s.path);
            if !s.comment.is_empty() {
                let _ = writeln!(o, "\tcomment = {}", s.comment);
            }
            let _ = writeln!(o, "\tread only = {}", yn(s.read_only));
            let _ = writeln!(o, "\tbrowseable = yes");
            let _ = writeln!(o, "\tguest ok = {}", yn(s.guest_ok));
            if !s.guest_ok && !s.valid_users.is_empty() {
                let _ = writeln!(o, "\tvalid users = {}", s.valid_users.join(" "));
            }
            let _ = writeln!(o, "\tforce user = {}", cfg.share_user);
            let _ = writeln!(o, "\tforce group = {}", cfg.share_user);
            let _ = writeln!(o, "\tcreate mask = 0664");
            let _ = writeln!(o, "\tdirectory mask = 0775");
        }
        o
    }

    /// Write the SMB config and ask the SMB server to reload it.
    pub fn apply(&self, cfg: &Config) -> Result<(), String> {
        let conf = self.render(cfg, &network::hostname());
        util::write_atomic(&cfg.smb_conf, conf.as_bytes(), 0o644)?;
        if !cfg.smb_service.is_empty() {
            util::run(cfg.dry_run, &cfg.smb_service, &["reload"])?;
        }
        Ok(())
    }
}

fn validate_share_name(n: &str) -> Result<(), String> {
    let ok = !n.is_empty()
        && n.len() <= 32
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !RESERVED.contains(&n.to_lowercase().as_str());
    if ok { Ok(()) } else { Err("Tên chia sẻ chỉ gồm chữ, số, _ và - (tối đa 32 ký tự)".into()) }
}

fn validate_smb_user(u: &str) -> Result<(), String> {
    let mut chars = u.chars();
    let ok = u.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok { Ok(()) } else { Err("Tên người dùng: chữ thường, số, _ và -, bắt đầu bằng chữ (tối đa 32)".into()) }
}

/// A share must be an absolute path inside a disk mounted below mount_root.
fn validate_path(cfg: &Config, p: &str) -> Result<String, String> {
    let p = p.trim();
    let p = if p.len() > 1 { p.trim_end_matches('/') } else { p };
    let path = Path::new(p);
    if !path.is_absolute()
        || p.chars().any(char::is_control)
        || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("Đường dẫn không hợp lệ".into());
    }
    let rel = path
        .strip_prefix(&cfg.mount_root)
        .map_err(|_| format!("Thư mục phải nằm trong {}", cfg.mount_root.display()))?;
    let top = rel.components().next().ok_or("Hãy chọn thư mục trên một ổ đã mount")?;
    let top = cfg.mount_root.join(top);
    if !cfg.dry_run && !storage::is_mountpoint(&top) {
        return Err(format!("{} chưa được mount", top.display()));
    }
    Ok(p.to_string())
}

/// Create the share directory if needed and hand root-owned dirs to the share
/// user so SMB clients can write (only that directory, never recursively).
fn prepare_dir(cfg: &Config, p: &Path) -> Result<(), String> {
    if cfg.dry_run {
        return Ok(());
    }
    use std::os::unix::fs::MetadataExt;
    fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let meta = fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if meta.uid() == 0 {
        std::os::unix::fs::chown(p, Some(cfg.share_uid), Some(cfg.share_gid))
            .map_err(|e| format!("chown {}: {e}", p.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ShareStore {
        ShareStore {
            file: PathBuf::from("/nonexistent/shares.toml"),
            data: Data {
                workgroup: "WORKGROUP".into(),
                smb_users: vec!["anh".into()],
                shares: vec![
                    Share {
                        name: "media".into(),
                        path: "/mnt/data/media".into(),
                        comment: "Phim".into(),
                        read_only: true,
                        guest_ok: true,
                        valid_users: vec![],
                    },
                    Share {
                        name: "docs".into(),
                        path: "/mnt/data/docs".into(),
                        comment: String::new(),
                        read_only: false,
                        guest_ok: false,
                        valid_users: vec!["anh".into()],
                    },
                ],
            },
        }
    }

    #[test]
    fn renders_conf() {
        let conf = store().render(&Config::default(), "rustnas");
        assert!(conf.contains("netbios name = RUSTNAS"));
        assert!(conf.contains("map to guest = bad user"));
        assert!(conf.contains("[media]\n\tpath = /mnt/data/media"));
        assert!(conf.contains("valid users = anh"));
        assert!(conf.contains("force user = nas"));
    }

    #[test]
    fn path_rules() {
        let cfg = Config { dry_run: true, ..Config::default() };
        assert!(validate_path(&cfg, "/mnt/data/x/").is_ok());
        assert!(validate_path(&cfg, "/mnt/data/../../etc").is_err());
        assert!(validate_path(&cfg, "/etc").is_err());
        assert!(validate_path(&cfg, "/mnt").is_err());
        assert!(validate_path(&cfg, "relative").is_err());
    }

    #[test]
    fn names() {
        assert!(validate_share_name("media-2").is_ok());
        assert!(validate_share_name("IPC$").is_err());
        assert!(validate_share_name("global").is_err());
        assert!(validate_smb_user("anh_1").is_ok());
        assert!(validate_smb_user("1anh").is_err());
        assert!(validate_smb_user("Anh").is_err());
    }
}
