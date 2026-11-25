//! SMB (ksmbd) and NFS shares with per-user read/write access, plus SMB users.
//!
//! The web process keeps the source of truth in state_dir/shares.toml and
//! sends structured data to the root helper, which validates it and renders
//! ksmbd.conf / exports itself.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    files, netcfg,
    privileged::{self, Op},
    storage, util,
};

const RESERVED: &[&str] = &["global", "homes", "printers", "print$", "ipc$"];
const MAX_SHARES: usize = 64;
const MAX_USERS: usize = 64;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShareUser {
    pub name: String,
    /// "rw" or "ro"
    pub access: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Share {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub comment: String,
    /// Default for users not listed (and for guests).
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub guest_ok: bool,
    /// Empty = every SMB user, with the share's default access.
    #[serde(default)]
    pub users: Vec<ShareUser>,
    #[serde(default)]
    pub nfs: bool,
    /// Space/comma separated IPv4 addresses or CIDRs allowed over NFS.
    #[serde(default)]
    pub nfs_clients: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Data {
    workgroup: String,
    nfs_enabled: bool,
    smb_users: Vec<String>,
    shares: Vec<Share>,
}

impl Default for Data {
    fn default() -> Self {
        Self { workgroup: "WORKGROUP".into(), nfs_enabled: false, smb_users: Vec::new(), shares: Vec::new() }
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
    pub fn nfs_enabled(&self) -> bool {
        self.data.nfs_enabled
    }
    pub fn paths(&self) -> Vec<String> {
        self.data.shares.iter().map(|s| s.path.clone()).collect()
    }

    fn commit(&mut self, cfg: &Config, next: Data) -> Result<(), String> {
        let s = toml::to_string_pretty(&next).map_err(|e| e.to_string())?;
        util::write_atomic(&self.file, s.as_bytes(), 0o600)?;
        self.data = next;
        self.apply(cfg)
    }

    pub fn upsert(&mut self, cfg: &Config, mut s: Share) -> Result<(), String> {
        validate_share(&mut s)?;
        if let Some(u) = s.users.iter().find(|u| !self.data.smb_users.contains(&u.name)) {
            return Err(format!("Người dùng SMB '{}' không tồn tại", u.name));
        }
        if !cfg.dry_run && storage::data_mount_of(cfg, Path::new(&s.path)).is_none() {
            return Err("Thư mục phải nằm trên ổ dữ liệu đã mount".into());
        }
        privileged::call(cfg, Op::PrepareDir { path: s.path.clone() })?;

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

    pub fn set_nfs_enabled(&mut self, cfg: &Config, enabled: bool) -> Result<(), String> {
        let mut next = self.data.clone();
        next.nfs_enabled = enabled;
        self.commit(cfg, next)
    }

    pub fn set_user(&mut self, cfg: &Config, user: &str, password: &str) -> Result<(), String> {
        validate_smb_user(user)?;
        validate_smb_password(password)?;
        let exists = self.data.smb_users.iter().any(|u| u == user);
        if !exists && self.data.smb_users.len() >= MAX_USERS {
            return Err("Đã đạt số lượng người dùng tối đa".into());
        }
        let action = if exists { "update" } else { "add" };
        privileged::call(cfg, Op::SmbUser { action: action.into(), user: user.into(), password: Some(password.into()) })?;
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
        // Dropping the last listed user would silently open a share to everyone.
        let used: Vec<&str> = self
            .data
            .shares
            .iter()
            .filter(|s| s.users.iter().any(|u| u.name == user))
            .map(|s| s.name.as_str())
            .collect();
        if !used.is_empty() {
            return Err(format!("'{user}' đang được gán cho chia sẻ: {} — hãy sửa chia sẻ trước", used.join(", ")));
        }
        privileged::call(cfg, Op::SmbUser { action: "delete".into(), user: user.into(), password: None })?;
        let mut next = self.data.clone();
        next.smb_users.retain(|u| u != user);
        self.commit(cfg, next)
    }

    /// Push the configuration to the helper (SMB + NFS).
    pub fn apply(&self, cfg: &Config) -> Result<(), String> {
        let smb = privileged::call(
            cfg,
            Op::ApplySmb { workgroup: self.data.workgroup.clone(), shares: self.data.shares.clone() },
        );
        let nfs = privileged::call(cfg, Op::ApplyNfs { enabled: self.data.nfs_enabled, shares: self.data.shares.clone() });
        smb?;
        nfs.map(|_| ())
    }

    pub fn raw(&self) -> String {
        toml::to_string_pretty(&self.data).unwrap_or_default()
    }
}

// ---- validation (shared by the web process and the helper) ---------------------------

pub fn validate_share(s: &mut Share) -> Result<(), String> {
    validate_share_name(&s.name)?;
    s.comment = s.comment.chars().filter(|c| !c.is_control()).take(80).collect::<String>().trim().to_string();
    if s.comment.contains(['[', ']']) {
        return Err("Mô tả không được chứa [ ]".into());
    }
    s.path = files::normalize_str(&s.path)?;
    for u in &s.users {
        validate_smb_user(&u.name)?;
        if !matches!(u.access.as_str(), "rw" | "ro") {
            return Err("Quyền phải là rw hoặc ro".into());
        }
    }
    s.users.sort_by(|a, b| a.name.cmp(&b.name));
    s.users.dedup_by(|a, b| a.name == b.name);
    if s.guest_ok {
        s.users.clear();
    }
    s.nfs_clients = netcfg::normalize_cidrs(&s.nfs_clients)?.join(" ");
    if s.nfs && s.nfs_clients.is_empty() {
        return Err("Hãy nhập mạng được phép truy cập NFS, ví dụ 192.168.1.0/24".into());
    }
    Ok(())
}

fn validate_share_name(n: &str) -> Result<(), String> {
    let ok = !n.is_empty()
        && n.len() <= 32
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !RESERVED.contains(&n.to_lowercase().as_str());
    if ok { Ok(()) } else { Err("Tên chia sẻ chỉ gồm chữ, số, _ và - (tối đa 32 ký tự)".into()) }
}

pub fn validate_smb_user(u: &str) -> Result<(), String> {
    let mut chars = u.chars();
    let ok = u.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok { Ok(()) } else { Err("Tên người dùng: chữ thường, số, _ và -, bắt đầu bằng chữ (tối đa 32)".into()) }
}

fn validate_smb_password(p: &str) -> Result<(), String> {
    let n = p.chars().count();
    if !(8..=128).contains(&n) || p.chars().any(char::is_control) {
        return Err("Mật khẩu SMB cần 8–128 ký tự, không chứa ký tự điều khiển".into());
    }
    Ok(())
}

// ---- rendering (pure; used by the helper) --------------------------------------------

pub fn render_smb(cfg: &Config, workgroup: &str, shares: &[Share], hostname: &str) -> String {
    let mut netbios: String = hostname.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(15).collect();
    if netbios.is_empty() {
        netbios = "HEHENAS".into();
    }
    let wg: String = workgroup.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(15).collect();
    let any_guest = shares.iter().any(|s| s.guest_ok);
    let yn = |b: bool| if b { "yes" } else { "no" };

    let mut o = String::new();
    let _ = writeln!(o, "; Generated by hehenasd — manual edits will be overwritten.");
    let _ = writeln!(o, "[global]");
    let _ = writeln!(o, "\tnetbios name = {}", netbios.to_uppercase());
    let _ = writeln!(o, "\tworkgroup = {}", if wg.is_empty() { "WORKGROUP".into() } else { wg.to_uppercase() });
    let _ = writeln!(o, "\tserver string = HeheNAS");
    let _ = writeln!(o, "\tserver min protocol = SMB2_10");
    let _ = writeln!(o, "\trestrict anonymous = {}", if any_guest { 0 } else { 2 });
    let _ = writeln!(o, "\tmap to guest = {}", if any_guest { "bad user" } else { "never" });
    let _ = writeln!(o, "\tguest account = {}", cfg.share_user);
    for s in shares {
        let _ = writeln!(o, "\n[{}]", s.name);
        let _ = writeln!(o, "\tpath = {}", s.path);
        if !s.comment.is_empty() {
            let _ = writeln!(o, "\tcomment = {}", s.comment);
        }
        let _ = writeln!(o, "\tread only = {}", yn(s.read_only));
        let _ = writeln!(o, "\tbrowseable = yes");
        let _ = writeln!(o, "\tguest ok = {}", yn(s.guest_ok));
        if !s.guest_ok && !s.users.is_empty() {
            let names: Vec<&str> = s.users.iter().map(|u| u.name.as_str()).collect();
            let _ = writeln!(o, "\tvalid users = {}", names.join(" "));
            let pick = |acc: &str| -> Vec<&str> {
                s.users.iter().filter(|u| u.access == acc).map(|u| u.name.as_str()).collect()
            };
            if s.read_only {
                let rw = pick("rw");
                if !rw.is_empty() {
                    let _ = writeln!(o, "\twrite list = {}", rw.join(" "));
                }
            } else {
                let ro = pick("ro");
                if !ro.is_empty() {
                    let _ = writeln!(o, "\tread list = {}", ro.join(" "));
                }
            }
        }
        let _ = writeln!(o, "\tforce user = {}", cfg.share_user);
        let _ = writeln!(o, "\tforce group = {}", cfg.share_user);
        let _ = writeln!(o, "\tcreate mask = 0664");
        let _ = writeln!(o, "\tdirectory mask = 0775");
        let _ = writeln!(o, "\thide dot files = yes");
        let _ = writeln!(o, "\tveto files = /{}/{}/", storage::TRASH_DIR, storage::UPDATE_DIR);
    }
    o
}

pub fn render_exports(cfg: &Config, shares: &[Share]) -> String {
    let mut o = String::from("# Generated by hehenasd\n");
    for s in shares.iter().filter(|s| s.nfs && !s.nfs_clients.is_empty()) {
        let mode = if s.read_only { "ro" } else { "rw" };
        let opts = format!(
            "{mode},sync,no_subtree_check,all_squash,anonuid={},anongid={},sec=sys",
            cfg.share_uid, cfg.share_gid
        );
        let clients: Vec<String> = s.nfs_clients.split_whitespace().map(|c| format!("{c}({opts})")).collect();
        let _ = writeln!(o, "\"{}\" {}", s.path, clients.join(" "));
    }
    o
}

/// Only shares whose data disk is mounted make it into the config.
fn h_live_shares(cfg: &Config, shares: &[Share]) -> Result<Vec<Share>, String> {
    let mut out = Vec::new();
    for s in shares {
        let mut s = s.clone();
        validate_share(&mut s)?;
        let p = files::normalize(cfg, &s.path)?;
        if cfg.dry_run || storage::data_mount_of(cfg, &p).is_some() {
            out.push(s);
        } else {
            crate::info!("share {} skipped: disk not mounted", s.name);
        }
    }
    Ok(out)
}

pub fn h_apply_smb(cfg: &Config, workgroup: &str, shares: &[Share]) -> Result<String, String> {
    let live = h_live_shares(cfg, shares)?;
    let hostname = crate::network::hostname();
    util::write_atomic(&cfg.smb_conf, render_smb(cfg, workgroup, &live, &hostname).as_bytes(), 0o644)?;
    if !cfg.smb_service.is_empty() {
        util::run(cfg.dry_run, &cfg.smb_service, &["reload"])?;
    }
    Ok(String::new())
}

pub fn h_apply_nfs(cfg: &Config, enabled: bool, shares: &[Share]) -> Result<String, String> {
    let live = h_live_shares(cfg, shares)?;
    util::write_atomic(&cfg.exports_file, render_exports(cfg, &live).as_bytes(), 0o644)?;
    let Some(script) = privileged::service_script(cfg, "nfs") else {
        return if enabled { Err("Image này không có NFS server".into()) } else { Ok(String::new()) };
    };
    if enabled {
        util::run(cfg.dry_run, &script, &["start"])?;
        util::run(cfg.dry_run, "exportfs", &["-ra"]).map(|_| String::new())
    } else {
        util::run(cfg.dry_run, &script, &["stop"]).map(|_| String::new())
    }
}

pub fn h_smb_user(cfg: &Config, action: &str, user: &str, password: Option<&str>) -> Result<String, String> {
    validate_smb_user(user)?;
    match (action, password) {
        // ksmbd-tools: -a add / -u update / -d delete (3.4 and 3.5 CLIs).
        ("add" | "update", Some(pw)) => {
            validate_smb_password(pw)?;
            let flag = if action == "add" { "-a" } else { "-u" };
            util::run(cfg.dry_run, "ksmbd.adduser", &[flag, user, "-p", pw])
        }
        ("delete", None) => util::run(cfg.dry_run, "ksmbd.adduser", &["-d", user]),
        _ => Err("Thao tác không hợp lệ".into()),
    }
}

fn smb_db_path(cfg: &Config) -> PathBuf {
    cfg.smb_conf.with_file_name("ksmbdpwd.db")
}

pub fn h_read_smb_db(cfg: &Config) -> Result<String, String> {
    Ok(fs::read_to_string(smb_db_path(cfg)).unwrap_or_default())
}

pub fn h_write_smb_db(cfg: &Config, content: &str) -> Result<String, String> {
    // Format: one "user:base64hash" per line — refuse anything else.
    for line in content.lines().filter(|l| !l.is_empty()) {
        let (u, h) = line.split_once(':').ok_or("DB người dùng SMB không hợp lệ")?;
        validate_smb_user(u)?;
        if h.is_empty() || !h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')) {
            return Err("DB người dùng SMB không hợp lệ".into());
        }
    }
    if cfg.dry_run {
        return Ok(String::new());
    }
    util::write_atomic(&smb_db_path(cfg), content.as_bytes(), 0o600).map(|_| String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(name: &str, ro: bool, users: &[(&str, &str)]) -> Share {
        Share {
            name: name.into(),
            path: format!("/mnt/data/{name}"),
            comment: String::new(),
            read_only: ro,
            guest_ok: false,
            users: users.iter().map(|(n, a)| ShareUser { name: (*n).into(), access: (*a).into() }).collect(),
            nfs: false,
            nfs_clients: String::new(),
        }
    }

    #[test]
    fn renders_acl() {
        let cfg = Config::default();
        let conf = render_smb(&cfg, "WORKGROUP", &[share("docs", false, &[("anh", "rw"), ("em", "ro")])], "hehenas");
        assert!(conf.contains("netbios name = HEHENAS"));
        assert!(conf.contains("valid users = anh em"));
        assert!(conf.contains("read list = em"));
        assert!(conf.contains("restrict anonymous = 2"));
        assert!(conf.contains("veto files = /.hehenas-trash/.hehenas-update/"));
        let conf = render_smb(&cfg, "WG", &[share("media", true, &[("anh", "rw")])], "x");
        assert!(conf.contains("write list = anh"));
    }

    #[test]
    fn renders_exports() {
        let cfg = Config::default();
        let mut s = share("media", true, &[]);
        s.nfs = true;
        s.nfs_clients = "192.168.1.0/24 10.0.0.5".into();
        let e = render_exports(&cfg, &[s]);
        assert!(e.contains("\"/mnt/data/media\" 192.168.1.0/24(ro,sync"));
        assert!(e.contains("10.0.0.5(ro,"));
    }

    #[test]
    fn validation() {
        let mut s = share("ok-1", false, &[]);
        assert!(validate_share(&mut s).is_ok());
        let mut s = share("bad", false, &[]);
        s.path = "/mnt/data/../../etc".into();
        assert!(validate_share(&mut s).is_err());
        let mut s = share("bad", false, &[]);
        s.nfs = true;
        assert!(validate_share(&mut s).is_err());
        assert!(validate_share_name("IPC$").is_err());
        assert!(validate_smb_user("1anh").is_err());
    }
}
