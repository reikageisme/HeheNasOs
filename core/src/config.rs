//! Daemon configuration (/etc/rustnas/rustnasd.toml). Read-only at runtime;
//! settings changed from the web UI live in `state_dir` (see settings.rs).

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Plain HTTP listener (redirects to HTTPS when enabled in settings).
    pub listen: String,
    /// HTTPS listener (self-signed certificate in state_dir/tls).
    pub https_listen: String,
    /// Persistent state (admin account, shares, mount table, settings...).
    pub state_dir: PathBuf,
    /// Data disks are mounted below this directory; shares must live here.
    pub mount_root: PathBuf,
    /// Disk holding the OS. Empty = detect from the device of "/".
    pub system_disk: String,
    /// Board id checked against update bundles ("rpi3b", "x86_64").
    pub board: String,

    // ---- privilege separation ----
    /// When started as root: fork a small root helper and run the web server
    /// as `share_user`. The helper only accepts a fixed set of validated ops.
    pub privsep: bool,
    pub helper_socket: PathBuf,

    // ---- files managed by the root helper ----
    pub smb_conf: PathBuf,
    pub smb_service: String,
    pub nfs_service: String,
    pub exports_file: PathBuf,
    pub hostname_file: PathBuf,
    pub hosts_file: PathBuf,
    pub interfaces_file: PathBuf,
    pub resolv_static: PathBuf,
    pub nft_file: PathBuf,
    pub net_iface: String,

    // ---- A/B updates (Raspberry Pi image) ----
    /// FAT boot partition holding slot.txt, kernels and recovery flags.
    pub boot_dir: PathBuf,
    pub slot_a_dev: String,
    pub slot_b_dev: String,

    /// Unix account that owns shared files (SMB "force user", NFS squash,
    /// and the account the web server runs as under privsep).
    pub share_user: String,
    pub share_uid: u32,
    pub share_gid: u32,
    pub session_ttl_secs: u64,
    /// Syslog file shown on the System page.
    pub log_file: PathBuf,
    /// Serve the dashboard from this directory instead of the embedded copy.
    pub web_root: Option<PathBuf>,
    /// Log privileged commands instead of running them (development).
    pub dry_run: bool,
}

impl Default for Config {
    fn default() -> Self {
        let etc = PathBuf::from("/var/lib/rustnas/etc");
        Self {
            listen: "0.0.0.0:8080".into(),
            https_listen: "0.0.0.0:8443".into(),
            state_dir: "/var/lib/rustnas".into(),
            mount_root: "/mnt".into(),
            system_disk: String::new(),
            board: "rpi3b".into(),
            privsep: true,
            helper_socket: "/run/rustnas/helper.sock".into(),
            smb_conf: "/var/lib/rustnas/ksmbd/ksmbd.conf".into(),
            smb_service: "/etc/init.d/S60ksmbd".into(),
            nfs_service: "/usr/libexec/rustnas/nfs-service".into(),
            exports_file: etc.join("exports"),
            hostname_file: etc.join("hostname"),
            hosts_file: etc.join("hosts"),
            interfaces_file: etc.join("interfaces"),
            resolv_static: etc.join("resolv.static"),
            nft_file: etc.join("firewall.nft"),
            net_iface: "eth0".into(),
            boot_dir: "/boot".into(),
            slot_a_dev: "mmcblk0p2".into(),
            slot_b_dev: "mmcblk0p3".into(),
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

    /// TCP port of a "host:port" listen string.
    pub fn port_of(listen: &str) -> u16 {
        listen.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(0)
    }
}
