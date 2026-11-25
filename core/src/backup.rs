//! Configuration backup / restore as a single JSON file.
//!
//! Without `secrets` the backup holds no password hashes, so it is safe to
//! store anywhere; with secrets it also carries the admin hash and the SMB
//! password database (treat it like a password).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    netcfg,
    privileged::{self, Op},
    util,
};

const FORMAT: &str = "hehenas-backup-1";
const PLAIN: &[&str] = &["shares.toml", "mounts.toml", "network.toml", "firewall.toml", "settings.toml"];
const SECRET: &[&str] = &["auth.toml"];

#[derive(Serialize, Deserialize)]
pub struct Backup {
    pub format: String,
    pub version: String,
    pub created: u64,
    pub hostname: String,
    pub secrets: bool,
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub smb_db: Option<String>,
}

pub fn create(cfg: &Config, secrets: bool) -> Result<Backup, String> {
    let mut files = BTreeMap::new();
    let names: Vec<&str> = PLAIN.iter().chain(if secrets { SECRET } else { &[] }).copied().collect();
    for name in names {
        if let Ok(s) = std::fs::read_to_string(cfg.state_dir.join(name)) {
            files.insert(name.to_string(), s);
        }
    }
    let smb_db = if secrets { Some(privileged::call(cfg, Op::ReadSmbDb)?) } else { None };
    Ok(Backup {
        format: FORMAT.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        created: util::now_secs(),
        hostname: crate::network::hostname(),
        secrets,
        files,
        smb_db,
    })
}

/// Validate everything first, then write. The caller restarts the daemon so
/// every subsystem re-reads its state.
pub fn restore(cfg: &Config, b: &Backup) -> Result<(), String> {
    if b.format != FORMAT {
        return Err("Không phải file sao lưu HeheNAS".into());
    }
    for (name, content) in &b.files {
        if !PLAIN.contains(&name.as_str()) && !SECRET.contains(&name.as_str()) {
            return Err(format!("File lạ trong bản sao lưu: {name}"));
        }
        toml::from_str::<toml::Value>(content).map_err(|e| format!("{name}: {e}"))?;
    }
    if let Some(n) = b.files.get("network.toml") {
        netcfg::validate_net(&toml::from_str(n).map_err(|e| format!("network.toml: {e}"))?)?;
    }
    if let Some(f) = b.files.get("firewall.toml") {
        netcfg::validate_fw(&toml::from_str(f).map_err(|e| format!("firewall.toml: {e}"))?)?;
    }
    for (name, content) in &b.files {
        util::write_atomic(&cfg.state_dir.join(name), content.as_bytes(), 0o600)?;
    }
    if let Some(db) = &b.smb_db {
        privileged::call(cfg, Op::WriteSmbDb { content: db.clone() })?;
    }
    if netcfg::validate_hostname(&b.hostname).is_ok() {
        let _ = privileged::call(cfg, Op::SetHostname { name: b.hostname.clone() });
    }
    // hehenasd applies network.toml at its next start (it restarts right
    // after a restore) when this marker exists.
    let _ = std::fs::write(cfg.state_dir.join(".apply-network"), b"1");
    crate::info!("configuration restored from backup of {} ({})", b.hostname, b.version);
    Ok(())
}
