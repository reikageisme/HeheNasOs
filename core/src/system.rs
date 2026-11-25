//! Services (status + control through the helper), logs and process memory.

use std::{fs, path::Path};

use serde::Serialize;

use crate::{
    config::Config,
    privileged::{self, Op},
    util::read_trim,
};

struct Def {
    id: &'static str,
    name: &'static str,
    /// Process names as shown in /proc/<pid>/comm (any of them).
    comms: &'static [&'static str],
    /// Name understood by the helper's Service op (None = view only).
    ctl: Option<&'static str>,
}

const SERVICES: &[Def] = &[
    Def { id: "hehenasd", name: "HeheNAS daemon", comms: &["hehenasd"], ctl: None },
    Def { id: "smb", name: "SMB server (ksmbd)", comms: &["ksmbd.mountd"], ctl: Some("smb") },
    Def { id: "nfs", name: "NFS server", comms: &["rpc.mountd", "nfsd"], ctl: Some("nfs") },
    Def { id: "network", name: "Mạng / DHCP", comms: &["udhcpc", "dhcpcd", "NetworkManager"], ctl: None },
    Def { id: "syslog", name: "System log", comms: &["syslogd", "systemd-journal"], ctl: Some("syslog") },
];

#[derive(Serialize)]
pub struct ServiceStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub running: bool,
    pub pids: Vec<u32>,
    pub rss_bytes: u64,
    pub controllable: bool,
}

pub fn services(cfg: &Config) -> Vec<ServiceStatus> {
    let procs = scan_procs();
    SERVICES
        .iter()
        .map(|d| {
            let mine: Vec<&(u32, String)> = procs.iter().filter(|(_, c)| d.comms.contains(&c.as_str())).collect();
            ServiceStatus {
                id: d.id,
                name: d.name,
                running: !mine.is_empty(),
                pids: mine.iter().map(|(p, _)| *p).collect(),
                rss_bytes: mine.iter().map(|(p, _)| rss(&p.to_string())).sum(),
                controllable: d.ctl.and_then(|c| privileged::service_script(cfg, c)).is_some(),
            }
        })
        .collect()
}

pub fn service_action(cfg: &Config, id: &str, action: &str) -> Result<(), String> {
    let def = SERVICES.iter().find(|d| d.id == id).ok_or("Dịch vụ không tồn tại")?;
    let ctl = def.ctl.ok_or("Dịch vụ này không điều khiển được từ giao diện web")?;
    if !matches!(action, "start" | "stop" | "restart") {
        return Err("Thao tác không hợp lệ".into());
    }
    privileged::call(cfg, Op::Service { name: ctl.into(), action: action.into() }).map(|_| ())
}

fn scan_procs() -> Vec<(u32, String)> {
    let Ok(rd) = fs::read_dir("/proc") else { return Vec::new() };
    rd.filter_map(Result::ok)
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            Some((pid, read_trim(e.path().join("comm"))?))
        })
        .collect()
}

/// Resident set size in bytes of /proc/<pid> ("self" works too).
pub fn rss(pid: &str) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

/// Does a syslog line belong to `source`?
fn matches_source(line: &str, source: &str) -> bool {
    let has = |tags: &[&str]| tags.iter().any(|t| line.contains(t));
    match source {
        "hehenasd" => has(&[" hehenasd", " hehenas:"]),
        "smb" => has(&["ksmbd"]),
        "nfs" => has(&["nfsd", "rpc.mountd", "rpcbind", "exportfs"]),
        "network" => has(&["udhcpc", "ifup", "eth0", "nft"]),
        "kernel" => has(&[" kern.", "kernel:"]),
        _ => true,
    }
}

pub fn tail(path: &Path, n: usize, source: &str, query: &str) -> Vec<String> {
    let s = fs::read_to_string(path).unwrap_or_default();
    let q = query.to_lowercase();
    let lines: Vec<&str> = s
        .lines()
        .filter(|l| matches_source(l, source) && (q.is_empty() || l.to_lowercase().contains(&q)))
        .collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_sources() {
        let l = "Nov 25 10:00:00 hehenas user.info hehenasd: listening";
        assert!(super::matches_source(l, "hehenasd"));
        assert!(!super::matches_source(l, "smb"));
        assert!(super::matches_source("Nov 25 hehenas kern.info kernel: ksmbd: running", "smb"));
        assert!(super::matches_source(l, "all"));
    }
}
