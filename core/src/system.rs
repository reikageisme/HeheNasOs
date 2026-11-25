//! Services (status + start/stop via init scripts), logs and process memory.

use std::{fs, path::Path};

use serde::Serialize;

use crate::{
    config::Config,
    util::{self, read_trim},
};

struct Def {
    id: &'static str,
    name: &'static str,
    /// Process names as shown in /proc/<pid>/comm (any of them).
    comms: &'static [&'static str],
    /// Control script (start/stop/restart). "smb" uses `smb_service` from the
    /// config; a script that does not exist on this system is view-only.
    script: Option<&'static str>,
}

const SERVICES: &[Def] = &[
    Def { id: "rustnasd", name: "RustNAS daemon", comms: &["rustnasd"], script: None },
    Def { id: "smb", name: "SMB server (ksmbd)", comms: &["ksmbd.mountd"], script: Some("") },
    Def { id: "network", name: "Mạng / DHCP", comms: &["udhcpc", "dhcpcd", "NetworkManager"], script: None },
    Def { id: "syslog", name: "System log", comms: &["syslogd", "systemd-journal"], script: Some("/etc/init.d/S01syslogd") },
];

fn script_for(cfg: &Config, d: &Def) -> Option<String> {
    let s = d.script?;
    let s = if d.id == "smb" { cfg.smb_service.as_str() } else { s };
    (!s.is_empty() && (cfg.dry_run || Path::new(s).exists())).then(|| s.to_string())
}

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
            let mine: Vec<&(u32, String)> =
                procs.iter().filter(|(_, c)| d.comms.contains(&c.as_str())).collect();
            ServiceStatus {
                id: d.id,
                name: d.name,
                running: !mine.is_empty(),
                pids: mine.iter().map(|(p, _)| *p).collect(),
                rss_bytes: mine.iter().map(|(p, _)| rss(&p.to_string())).sum(),
                controllable: script_for(cfg, d).is_some(),
            }
        })
        .collect()
}

pub fn service_action(cfg: &Config, id: &str, action: &str) -> Result<(), String> {
    let def = SERVICES.iter().find(|d| d.id == id).ok_or("Dịch vụ không tồn tại")?;
    let script = script_for(cfg, def).ok_or("Dịch vụ này không điều khiển được từ giao diện web")?;
    if !matches!(action, "start" | "stop" | "restart") {
        return Err("Thao tác không hợp lệ".into());
    }
    crate::info!("service {id}: {action}");
    util::run(cfg.dry_run, &script, &[action]).map(|_| ())
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

pub fn tail(path: &Path, n: usize) -> Vec<String> {
    let s = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}
