//! Daemon configuration (/etc/rustnas/rustnasd.toml).

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Address of the web UI / API, e.g. "0.0.0.0:8080".
    pub listen: String,
    /// Persistent state (admin account, shares, mount table, ksmbd db).
    pub state_dir: PathBuf,
    /// Data disks are mounted below this directory; shares must live here.
    pub mount_root: PathBuf,
    /// Block device holding the OS — never mounted/formatted from the UI.
    pub system_disk: String,
    /// Generated ksmbd/Samba configuration file.
    pub smb_conf: PathBuf,
    /// Init script used to reload/restart the SMB server ("" = none).
    pub smb_service: String,
    /// Unix account that owns shared files (SMB "force user").
    pub share_user: String,
    pub share_uid: u32,
    pub share_gid: u32,
    pub session_ttl_secs: u64,
    /// Log shown on the System page.
    pub log_file: PathBuf,
    /// Serve the dashboard from this directory instead of the embedded copy
    /// (handy while developing the frontend).
    pub web_root: Option<PathBuf>,
    /// Log privileged commands instead of running them (development).
    pub dry_run: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            state_dir: "/var/lib/rustnas".into(),
            mount_root: "/mnt".into(),
            system_disk: "mmcblk0".into(),
            smb_conf: "/var/lib/rustnas/ksmbd/ksmbd.conf".into(),
            smb_service: "/etc/init.d/S60ksmbd".into(),
            share_user: "nas".into(),
            share_uid: 1000,
            share_gid: 1000,
            session_ttl_secs: 8 * 3600,
            log_file: "/var/log/messages".into(),
            web_root: None,
            dry_run: false,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                crate::info!("{} not found, using defaults", path.display());
                Ok(Self::default())
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}
