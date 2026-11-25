//! Privilege separation.
//!
//! When rustnasd starts as root it forks: the parent becomes a tiny root
//! helper listening on a Unix socket, the child drops to `share_user` and
//! runs the web server. The web process can only ask for the operations in
//! [`Op`]; the helper re-validates every argument and builds all config files
//! itself from structured data (no free-form file contents, no shell), so a
//! bug in the web layer cannot be turned into arbitrary root commands.
//!
//! Without root (development) or with `privsep = false`, ops run in-process.

use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    netcfg::{self, FwConfig, NetConfig},
    shares::{self, Share},
    storage, update, util,
};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Blkid,
    Smart { disk: String },
    Mount { dev: String, mountpoint: String, fstype: String },
    Umount { mountpoint: String },
    Mkfs { dev: String, label: String },
    Initialize { disk: String, label: String },
    PrepareDir { path: String },
    FixOwner { path: String },
    SmbUser { action: String, user: String, password: Option<String> },
    ApplySmb { workgroup: String, shares: Vec<Share> },
    ApplyNfs { enabled: bool, shares: Vec<Share> },
    ApplyNetwork { net: NetConfig },
    ApplyFirewall { fw: FwConfig },
    SetHostname { name: String },
    Service { name: String, action: String },
    Power { action: String },
    ReadSmbDb,
    WriteSmbDb { content: String },
    UpdateApply { bundle: String },
    UpdateConfirm,
    UpdateRollback,
}

#[derive(Serialize, Deserialize)]
struct Reply {
    ok: bool,
    out: String,
}

static SOCKET: OnceLock<PathBuf> = OnceLock::new();
/// Serialises mutating operations inside the helper.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Called in the unprivileged child: route ops to the helper socket.
pub fn use_socket(path: PathBuf) {
    let _ = SOCKET.set(path);
}

/// Run an operation (blocking; call from a blocking thread).
pub fn call(cfg: &Config, op: Op) -> Result<String, String> {
    match SOCKET.get() {
        Some(path) => remote(path, &op),
        None => exec(cfg, op),
    }
}

fn remote(path: &Path, op: &Op) -> Result<String, String> {
    let mut s = UnixStream::connect(path).map_err(|e| format!("helper: {e}"))?;
    let req = serde_json::to_vec(op).map_err(|e| e.to_string())?;
    s.write_all(&req).map_err(|e| format!("helper: {e}"))?;
    s.shutdown(std::net::Shutdown::Write).map_err(|e| format!("helper: {e}"))?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).map_err(|e| format!("helper: {e}"))?;
    let r: Reply = serde_json::from_str(&buf).map_err(|e| format!("helper reply: {e}"))?;
    if r.ok { Ok(r.out) } else { Err(r.out) }
}

fn is_mutating(op: &Op) -> bool {
    !matches!(op, Op::Blkid | Op::Smart { .. } | Op::ReadSmbDb)
}

/// Execute an op with full validation. Runs as root inside the helper.
pub fn exec(cfg: &Config, op: Op) -> Result<String, String> {
    let _guard = if is_mutating(&op) { Some(WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner())) } else { None };
    match op {
        Op::Blkid => Ok(storage::h_blkid()),
        Op::Smart { disk } => storage::h_smart(cfg, &disk),
        Op::Mount { dev, mountpoint, fstype } => storage::h_mount(cfg, &dev, &mountpoint, &fstype),
        Op::Umount { mountpoint } => storage::h_umount(cfg, &mountpoint),
        Op::Mkfs { dev, label } => storage::h_mkfs(cfg, &dev, &label),
        Op::Initialize { disk, label } => storage::h_initialize(cfg, &disk, &label),
        Op::PrepareDir { path } => storage::h_prepare_dir(cfg, &path),
        Op::FixOwner { path } => storage::h_fix_owner(cfg, &path),
        Op::SmbUser { action, user, password } => shares::h_smb_user(cfg, &action, &user, password.as_deref()),
        Op::ApplySmb { workgroup, shares } => shares::h_apply_smb(cfg, &workgroup, &shares),
        Op::ApplyNfs { enabled, shares } => shares::h_apply_nfs(cfg, enabled, &shares),
        Op::ApplyNetwork { net } => netcfg::h_apply_network(cfg, &net),
        Op::ApplyFirewall { fw } => netcfg::h_apply_firewall(cfg, &fw),
        Op::SetHostname { name } => netcfg::h_set_hostname(cfg, &name),
        Op::Service { name, action } => h_service(cfg, &name, &action),
        Op::Power { action } => h_power(cfg, &action),
        Op::ReadSmbDb => shares::h_read_smb_db(cfg),
        Op::WriteSmbDb { content } => shares::h_write_smb_db(cfg, &content),
        Op::UpdateApply { bundle } => update::h_apply(cfg, &bundle),
        Op::UpdateConfirm => update::h_confirm(cfg),
        Op::UpdateRollback => update::h_rollback(cfg),
    }
}

/// Services the UI may control, mapped to fixed scripts from the config.
pub fn service_script(cfg: &Config, name: &str) -> Option<String> {
    let s = match name {
        "smb" => cfg.smb_service.clone(),
        "nfs" => cfg.nfs_service.clone(),
        "syslog" => "/etc/init.d/S01syslogd".into(),
        _ => return None,
    };
    (!s.is_empty() && (cfg.dry_run || Path::new(&s).exists())).then_some(s)
}

fn h_service(cfg: &Config, name: &str, action: &str) -> Result<String, String> {
    let script = service_script(cfg, name).ok_or("Dịch vụ không điều khiển được")?;
    if !matches!(action, "start" | "stop" | "restart" | "reload") {
        return Err("Thao tác không hợp lệ".into());
    }
    crate::info!("service {name}: {action}");
    util::run(cfg.dry_run, &script, &[action])
}

fn h_power(cfg: &Config, action: &str) -> Result<String, String> {
    let cmd = match action {
        "reboot" => "reboot",
        "poweroff" => "poweroff",
        _ => return Err("Thao tác không hợp lệ".into()),
    };
    util::run(cfg.dry_run, "sync", &[])?;
    util::run(cfg.dry_run, cmd, &[])
}

// ---- helper process -------------------------------------------------------------

fn peer_uid(s: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: u32::MAX, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred/len are valid for writes of the sizes given.
    let rc = unsafe {
        libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0).then_some(cred.uid)
}

fn handle(cfg: &Config, mut s: UnixStream, allowed_uid: u32) {
    match peer_uid(&s) {
        Some(uid) if uid == allowed_uid || uid == 0 => {}
        other => {
            crate::info!("helper: rejected connection from uid {other:?}");
            return;
        }
    }
    let mut buf = Vec::new();
    // Requests are small; the cap stops a runaway client from eating RAM.
    if (&mut s).take(1 << 20).read_to_end(&mut buf).is_err() {
        return;
    }
    let reply = match serde_json::from_slice::<Op>(&buf) {
        Ok(op) => match exec(cfg, op) {
            Ok(out) => Reply { ok: true, out },
            Err(out) => Reply { ok: false, out },
        },
        Err(e) => Reply { ok: false, out: format!("yêu cầu không hợp lệ: {e}") },
    };
    let _ = s.write_all(&serde_json::to_vec(&reply).unwrap_or_default());
}

/// Bind the helper socket, owned by the unprivileged user, mode 0600.
pub fn bind(cfg: &Config) -> Result<UnixListener, String> {
    let path = &cfg.helper_socket;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::os::unix::fs::chown(path, Some(cfg.share_uid), Some(cfg.share_gid)).map_err(|e| e.to_string())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    Ok(l)
}

/// Root helper main loop (never returns). One thread per request.
pub fn serve(cfg: Config, listener: UnixListener) -> ! {
    let cfg = std::sync::Arc::new(cfg);
    for conn in listener.incoming() {
        let Ok(s) = conn else { continue };
        let cfg = cfg.clone();
        let uid = cfg.share_uid;
        std::thread::spawn(move || handle(&cfg, s, uid));
    }
    std::process::exit(1)
}

/// Drop from root to share_user (called in the forked child).
pub fn drop_privileges(cfg: &Config) -> Result<(), String> {
    let (uid, gid) = (cfg.share_uid, cfg.share_gid);
    // SAFETY: plain libc calls with valid arguments; order matters (groups, gid, uid).
    unsafe {
        let groups = [gid];
        if libc::setgroups(1, groups.as_ptr()) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
            return Err("không hạ được quyền".into());
        }
        if libc::setuid(0) == 0 {
            return Err("vẫn lấy lại được quyền root".into());
        }
        // Die with the helper so init restarts both together.
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    Ok(())
}
