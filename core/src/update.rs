//! A/B system updates for the SD-card image.
//!
//! Layout: p2 = slot A, p3 = slot B (read-only rootfs each), kernels
//! `Image-a` / `Image-b` and `cmdline-a.txt` / `cmdline-b.txt` on the FAT boot
//! partition, and `slot.txt` (included from config.txt) selecting the slot.
//!
//! An update bundle is a plain tar: `manifest.json` (first), `Image`,
//! `rootfs.ext4.gz`, each checked against the SHA-256 in the manifest. It is
//! written to the *inactive* slot, then `slot.txt` is switched and
//! `hehenas-try` records the attempt. Init (S00slot) counts boots; if the new
//! system does not confirm itself (hehenasd healthy for 90 s) within two
//! boots, it switches back to the old slot automatically.
//!
//! Every bundle must carry `manifest.sig` (second entry): an ed25519
//! signature of manifest.json made by the release workflow. The root helper
//! refuses unsigned or badly signed bundles, so neither a compromised web
//! process nor a compromised download source can install a system image.
//!
//! Over-the-air: hehenasd checks the latest GitHub release of `update_repo`,
//! downloads the bundle for its board and hands it to the same code path.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use ed25519_dalek::{Signature, VerifyingKey};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{config::Config, files, storage, util};

/// Public half of the release signing key (private key: GitHub secret
/// HEHENAS_SIGNING_KEY, used only by .github/workflows/release.yml).
const UPDATE_PUBKEY: &str = "ac7f5bcc231a9d14800468eb815056d86d81466475dac2626b6ec59eeada5d90";

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Check an ed25519 signature of the manifest against the trusted key.
pub fn verify_manifest(cfg: &Config, manifest: &[u8], sig: &[u8]) -> Result<(), String> {
    let hex_key = if cfg.update_pubkey.is_empty() { UPDATE_PUBKEY } else { cfg.update_pubkey.as_str() };
    let key: [u8; 32] = unhex(hex_key).and_then(|k| k.try_into().ok()).ok_or("Khóa xác minh cập nhật không hợp lệ")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "Khóa xác minh cập nhật không hợp lệ")?;
    let sig = Signature::from_slice(sig).map_err(|_| "Chữ ký gói cập nhật bị hỏng")?;
    key.verify_strict(manifest, &sig).map_err(|_| "Chữ ký gói cập nhật KHÔNG hợp lệ — từ chối cài".to_string())
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: String,
    board: String,
    kernel_sha256: String,
    rootfs_sha256: String,
}

#[derive(Serialize)]
pub struct Status {
    pub supported: bool,
    pub version: &'static str,
    pub board: String,
    pub current_slot: Option<String>,
    pub trying: Option<String>,
    pub slots: BTreeMap<String, String>,
}

fn current_slot() -> Option<String> {
    let cmdline = fs::read_to_string("/proc/cmdline").ok()?;
    cmdline
        .split_whitespace()
        .find_map(|w| w.strip_prefix("hehenas.slot="))
        .filter(|s| matches!(*s, "a" | "b"))
        .map(String::from)
}

fn other(slot: &str) -> &'static str {
    if slot == "a" { "b" } else { "a" }
}

fn slot_file(cfg: &Config) -> PathBuf {
    cfg.boot_dir.join("slot.txt")
}
fn try_file(cfg: &Config) -> PathBuf {
    cfg.boot_dir.join("hehenas-try")
}
fn versions_file(cfg: &Config) -> PathBuf {
    cfg.boot_dir.join("hehenas-slots.toml")
}

fn versions(cfg: &Config) -> BTreeMap<String, String> {
    fs::read_to_string(versions_file(cfg)).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

pub fn status(cfg: &Config) -> Status {
    let current = current_slot();
    Status {
        supported: slot_file(cfg).exists() && current.is_some(),
        version: env!("CARGO_PKG_VERSION"),
        board: cfg.board.clone(),
        trying: fs::read_to_string(try_file(cfg)).ok().and_then(|s| s.split_whitespace().next().map(String::from)),
        current_slot: current,
        slots: versions(cfg),
    }
}

/// Where a bundle of `size` bytes is stored before installing: a data disk
/// with room to spare, else the state partition (the rootfs is read-only
/// and RAM is precious).
pub fn bundle_path(cfg: &Config, size: u64) -> Result<PathBuf, String> {
    for m in storage::data_mounts(cfg) {
        if storage::fs_usage3(&m.to_string_lossy()).is_some_and(|(_, _, avail)| avail > size + (64 << 20)) {
            return Ok(m.join(storage::UPDATE_DIR).join("bundle.tar"));
        }
    }
    let st = state_bundle(cfg);
    if storage::fs_usage3(&cfg.state_dir.to_string_lossy()).is_some_and(|(_, _, avail)| avail > size + (16 << 20)) {
        return Ok(st);
    }
    Err("Không đủ chỗ trống để chứa gói cập nhật — hãy mount một ổ USB".into())
}

fn state_bundle(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("update").join("bundle.tar")
}

// ---- helper side ---------------------------------------------------------------

struct HashReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Minimal ustar reader: returns (name, size) of the next entry.
fn next_entry(r: &mut impl Read) -> io::Result<Option<(String, u64)>> {
    let mut h = [0u8; 512];
    r.read_exact(&mut h)?;
    if h.iter().all(|&b| b == 0) {
        return Ok(None);
    }
    let field = |a: usize, b: usize| {
        let f = &h[a..b];
        String::from_utf8_lossy(&f[..f.iter().position(|&c| c == 0).unwrap_or(f.len())]).into_owned()
    };
    let name = field(0, 100);
    let size = u64::from_str_radix(field(124, 136).trim(), 8)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "tar: kích thước không hợp lệ"))?;
    let name = name.trim_start_matches("./").to_string();
    Ok(Some((name, size)))
}

fn skip_padding(r: &mut impl Read, size: u64) -> io::Result<()> {
    let pad = (512 - size % 512) % 512;
    io::copy(&mut r.take(pad), &mut io::sink()).map(|_| ())
}

fn ioe(e: io::Error) -> String {
    e.to_string()
}

pub fn h_apply(cfg: &Config, bundle: &str) -> Result<String, String> {
    // The bundle must be exactly <data disk>/.hehenas-update/bundle.tar or
    // <state_dir>/update/bundle.tar.
    let p = if Path::new(bundle) == state_bundle(cfg) {
        state_bundle(cfg)
    } else {
        let p = files::normalize(cfg, bundle)?;
        let ok_name = p.file_name().is_some_and(|n| n == "bundle.tar")
            && p.parent().and_then(|d| d.file_name()).is_some_and(|n| n == storage::UPDATE_DIR);
        if !ok_name || storage::data_mount_of(cfg, &p).is_none() {
            return Err("Đường dẫn gói cập nhật không hợp lệ".into());
        }
        p
    };
    let st = status(cfg);
    let cur = st.current_slot.clone().filter(|_| st.supported).ok_or("Image này không hỗ trợ cập nhật A/B")?;
    let target = other(&cur);
    let dev = if target == "a" { &cfg.slot_a_dev } else { &cfg.slot_b_dev };
    if !storage::valid_devname(dev) || storage::read_mounts().iter().any(|m| m.dev == format!("/dev/{dev}")) {
        return Err("Phân vùng đích không hợp lệ hoặc đang được dùng".into());
    }
    let part_size = util::read_trim(format!("/sys/class/block/{dev}/size"))
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| s * 512)
        .ok_or("Không đọc được kích thước phân vùng")?;

    let mut r = io::BufReader::new(File::open(&p).map_err(ioe)?);
    let (name, size) = next_entry(&mut r).map_err(ioe)?.ok_or("Gói rỗng")?;
    if name != "manifest.json" || size > 65536 {
        return Err("Gói không hợp lệ: thiếu manifest.json ở đầu".into());
    }
    let mut mbuf = Vec::new();
    (&mut r).take(size).read_to_end(&mut mbuf).map_err(ioe)?;
    skip_padding(&mut r, size).map_err(ioe)?;
    // Signature first: nothing is written before it checks out.
    let mut pending = next_entry(&mut r).map_err(ioe)?;
    let sig_size = match &pending {
        Some((name, size)) if name == "manifest.sig" && *size <= 1024 => Some(*size),
        _ => None,
    };
    if let Some(size) = sig_size {
        let mut sig = Vec::new();
        (&mut r).take(size).read_to_end(&mut sig).map_err(ioe)?;
        skip_padding(&mut r, size).map_err(ioe)?;
        verify_manifest(cfg, &mbuf, &sig)?;
        pending = next_entry(&mut r).map_err(ioe)?;
    } else if cfg.allow_unsigned_updates {
        crate::info!("update: UNSIGNED bundle accepted (allow_unsigned_updates)");
    } else {
        return Err("Gói cập nhật chưa được ký — từ chối cài".into());
    }
    let m: Manifest = serde_json::from_slice(&mbuf).map_err(|e| format!("manifest: {e}"))?;
    if m.board != cfg.board {
        return Err(format!("Gói dành cho {}, máy này là {}", m.board, cfg.board));
    }

    let kernel_new = cfg.boot_dir.join(format!("Image-{target}.new"));
    let (mut kernel_ok, mut rootfs_ok) = (false, false);
    crate::info!("update {}: writing slot {target} ({dev})", m.version);
    while let Some((name, size)) = pending.take() {
        let mut hr = HashReader { inner: (&mut r).take(size), hasher: Sha256::new() };
        match name.as_str() {
            "Image" => {
                let mut f = File::create(&kernel_new).map_err(ioe)?;
                io::copy(&mut hr, &mut f).map_err(ioe)?;
                f.sync_all().map_err(ioe)?;
                kernel_ok = hex(&hr.hasher.finalize()) == m.kernel_sha256;
                if !kernel_ok {
                    let _ = fs::remove_file(&kernel_new);
                    return Err("Kernel sai checksum — gói bị hỏng".into());
                }
            }
            "rootfs.ext4.gz" => {
                let mut out = fs::OpenOptions::new().write(true).open(format!("/dev/{dev}")).map_err(ioe)?;
                let written = {
                    let mut gz = GzDecoder::new(&mut hr).take(part_size + 1);
                    io::copy(&mut gz, &mut out).map_err(ioe)?
                };
                // Drain whatever the decoder did not consume so the hash covers the whole entry.
                io::copy(&mut hr, &mut io::sink()).map_err(ioe)?;
                out.sync_all().map_err(ioe)?;
                if written > part_size {
                    return Err("Rootfs lớn hơn phân vùng".into());
                }
                rootfs_ok = hex(&hr.hasher.finalize()) == m.rootfs_sha256;
                if !rootfs_ok {
                    return Err("Rootfs sai checksum — gói bị hỏng, hệ thống hiện tại không bị ảnh hưởng".into());
                }
            }
            _ => {
                io::copy(&mut hr, &mut io::sink()).map_err(ioe)?;
            }
        }
        skip_padding(&mut r, size).map_err(ioe)?;
        pending = next_entry(&mut r).map_err(ioe)?;
    }
    if !(kernel_ok && rootfs_ok) {
        let _ = fs::remove_file(&kernel_new);
        return Err("Gói thiếu Image hoặc rootfs.ext4.gz".into());
    }

    fs::rename(&kernel_new, cfg.boot_dir.join(format!("Image-{target}"))).map_err(ioe)?;
    let mut vers = versions(cfg);
    vers.insert(target.into(), m.version.clone());
    let _ = util::write_atomic(&versions_file(cfg), toml::to_string(&vers).unwrap_or_default().as_bytes(), 0o644);
    util::write_atomic(&try_file(cfg), format!("{target} 0\n").as_bytes(), 0o644)?;
    switch_slot(cfg, target)?;
    let _ = fs::remove_file(&p);
    util::run(false, "sync", &[])?;
    crate::info!("update {} staged in slot {target}; reboot to try it", m.version);
    Ok(m.version)
}

// ---- over-the-air (web process) ----------------------------------------------

#[derive(Serialize, Clone, Default, Debug)]
pub struct Online {
    pub latest: Option<String>,
    pub available: bool,
    pub notes_url: Option<String>,
    #[serde(skip)]
    pub asset_url: Option<String>,
    pub asset_size: u64,
    pub checked_at: u64,
    pub error: Option<String>,
}

/// Background download/install job shown in the UI.
#[derive(Serialize, Clone, Default, Debug)]
pub struct Job {
    /// "idle", "downloading", "installing", "staged", "error"
    pub state: String,
    pub version: String,
    pub downloaded: u64,
    pub total: u64,
    pub error: Option<String>,
}

fn parse_ver(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim_start_matches('v').split(|c: char| c == '.' || c == '-').map(|x| x.parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Is `a` a newer version than `b`?
pub fn newer(a: &str, b: &str) -> bool {
    parse_ver(a) > parse_ver(b)
}

/// TLS certificate checks need a sane clock (the Pi has no RTC).
pub fn clock_ok() -> bool {
    util::now_secs() > 1_735_689_600 // 2025-01-01
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent(concat!("hehenasd/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Ask GitHub for the latest release and pick this board's bundle.
pub fn check(cfg: &Config) -> Online {
    let mut o = Online { checked_at: util::now_secs(), ..Default::default() };
    if !clock_ok() {
        o.error = Some("Đồng hồ hệ thống chưa đúng (đang chờ đồng bộ NTP)".into());
        return o;
    }
    let url = format!("https://api.github.com/repos/{}/releases/latest", cfg.update_repo);
    let res = agent().get(&url).set("Accept", "application/vnd.github+json").call();
    let v: serde_json::Value = match res.map_err(|e| e.to_string()).and_then(|r| {
        serde_json::from_reader(r.into_reader().take(4 << 20)).map_err(|e| e.to_string())
    }) {
        Ok(v) => v,
        Err(e) => {
            o.error = Some(format!("Không kiểm tra được bản mới: {e}"));
            return o;
        }
    };
    let tag = v.get("tag_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    o.notes_url = v.get("html_url").and_then(|x| x.as_str()).map(String::from);
    let prefix = format!("hehenas-{}-update-", cfg.board);
    let asset = v.get("assets").and_then(|a| a.as_array()).and_then(|a| {
        a.iter().find(|x| {
            x.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".tar"))
        })
    });
    if let Some(a) = asset {
        o.asset_url = a.get("browser_download_url").and_then(|x| x.as_str()).map(String::from);
        o.asset_size = a.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
    }
    o.available = !tag.is_empty() && o.asset_url.is_some() && newer(&tag, env!("CARGO_PKG_VERSION"));
    o.latest = (!tag.is_empty()).then(|| tag.trim_start_matches('v').to_string());
    o
}

/// Download the bundle (HTTPS, redirects followed) to `dest`, reporting progress.
pub fn download(url: &str, size: u64, dest: &Path, progress: &dyn Fn(u64)) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Chỉ tải cập nhật qua HTTPS".into());
    }
    let part = dest.with_extension("part");
    let resp = agent().get(url).call().map_err(|e| format!("Tải gói cập nhật thất bại: {e}"))?;
    let mut reader = resp.into_reader().take(size.max(1) + 1);
    let mut f = File::create(&part).map_err(ioe)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Mất kết nối khi tải: {e}"))?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(ioe)?;
        done += n as u64;
        progress(done);
    }
    f.sync_all().map_err(ioe)?;
    drop(f);
    if size > 0 && done != size {
        let _ = fs::remove_file(&part);
        return Err(format!("Tải thiếu dữ liệu ({done}/{size} byte)"));
    }
    fs::rename(&part, dest).map_err(ioe)
}

fn switch_slot(cfg: &Config, slot: &str) -> Result<(), String> {
    if !cfg.boot_dir.join(format!("Image-{slot}")).exists() {
        return Err(format!("Slot {slot} chưa có kernel"));
    }
    let content = format!("kernel=Image-{slot}\ncmdline=cmdline-{slot}.txt\n");
    util::write_atomic(&slot_file(cfg), content.as_bytes(), 0o644)
}

/// Mark the running slot as good (called once hehenasd has been healthy).
pub fn h_confirm(cfg: &Config) -> Result<String, String> {
    let Some(cur) = current_slot() else { return Ok(String::new()) };
    let trying = fs::read_to_string(try_file(cfg)).unwrap_or_default();
    if trying.split_whitespace().next() == Some(cur.as_str()) {
        fs::remove_file(try_file(cfg)).map_err(ioe)?;
        util::run(false, "sync", &[])?;
        crate::info!("update: slot {cur} confirmed");
        return Ok("confirmed".into());
    }
    Ok(String::new())
}

/// Boot the other slot next time.
pub fn h_rollback(cfg: &Config) -> Result<String, String> {
    let cur = current_slot().ok_or("Image này không hỗ trợ A/B")?;
    switch_slot(cfg, other(&cur))?;
    let _ = fs::remove_file(try_file(cfg));
    util::run(false, "sync", &[])?;
    Ok(other(&cur).into())
}

/// Write a bundle (used by tests and by scripts/make-update via `hehenasd make-update`).
pub fn write_bundle(out: &Path, version: &str, board: &str, kernel: &Path, rootfs_gz: &Path) -> Result<(), String> {
    fn sha(p: &Path) -> Result<String, String> {
        let mut h = Sha256::new();
        io::copy(&mut File::open(p).map_err(ioe)?, &mut h).map_err(ioe)?;
        Ok(hex(&h.finalize()))
    }
    let m = Manifest {
        version: version.into(),
        board: board.into(),
        kernel_sha256: sha(kernel)?,
        rootfs_sha256: sha(rootfs_gz)?,
    };
    let mut w = io::BufWriter::new(File::create(out).map_err(ioe)?);
    let mj = serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?;
    tar_entry(&mut w, "manifest.json", &mut &mj[..], mj.len() as u64)?;
    for (name, p) in [("Image", kernel), ("rootfs.ext4.gz", rootfs_gz)] {
        let len = fs::metadata(p).map_err(ioe)?.len();
        tar_entry(&mut w, name, &mut File::open(p).map_err(ioe)?, len)?;
    }
    w.write_all(&[0u8; 1024]).map_err(ioe)?;
    w.flush().map_err(ioe)
}

fn tar_entry(w: &mut impl Write, name: &str, data: &mut impl Read, len: u64) -> Result<(), String> {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{len:011o}\0").as_bytes());
    h[136..148].copy_from_slice(format!("{:011o}\0", util::now_secs()).as_bytes());
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    w.write_all(&h).map_err(ioe)?;
    let n = io::copy(&mut data.take(len), w).map_err(ioe)?;
    if n != len {
        return Err("tar: thiếu dữ liệu".into());
    }
    w.write_all(&vec![0u8; ((512 - len % 512) % 512) as usize]).map_err(ioe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_key_verifies_release_signature() {
        // Made with: openssl pkeyutl -sign -inkey hehenas-signing-key.pem -rawin
        let sig = unhex("c1a6b156669106985456b983c35da3cafe33ca881aa3bbb09c2f6ec201a971c81b86af31e4cadb7df1077d7426e07b360fd62ceffebfe9c0e41f955da35d6b09").unwrap();
        let cfg = Config::default();
        assert!(verify_manifest(&cfg, b"rustnas-signing-selftest", &sig).is_ok());
        assert!(verify_manifest(&cfg, b"hehenas-signing-selftesT", &sig).is_err());
        let mut bad = sig.clone();
        bad[0] ^= 1;
        assert!(verify_manifest(&cfg, b"rustnas-signing-selftest", &bad).is_err());
    }

    #[test]
    fn versions() {
        assert!(newer("v1.1.0", "1.0.9"));
        assert!(newer("1.10.0", "1.9.3"));
        assert!(!newer("1.1.0", "1.1.0"));
        assert!(!newer("v1.0.1", "1.1.0"));
    }

    #[test]
    fn tar_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hehenas-upd-{}", util::random_hex(4)));
        fs::create_dir_all(&dir).unwrap();
        let k = dir.join("Image");
        let r = dir.join("rootfs.ext4.gz");
        fs::write(&k, b"kernel-bytes").unwrap();
        fs::write(&r, vec![7u8; 1500]).unwrap();
        let out = dir.join("bundle.tar");
        write_bundle(&out, "9.9.9", "rpi3b", &k, &r).unwrap();

        let mut f = io::BufReader::new(File::open(&out).unwrap());
        let (n, s) = next_entry(&mut f).unwrap().unwrap();
        assert_eq!(n, "manifest.json");
        let mut m = Vec::new();
        (&mut f).take(s).read_to_end(&mut m).unwrap();
        skip_padding(&mut f, s).unwrap();
        let m: Manifest = serde_json::from_slice(&m).unwrap();
        assert_eq!(m.version, "9.9.9");
        let (n, s) = next_entry(&mut f).unwrap().unwrap();
        assert_eq!((n.as_str(), s), ("Image", 12));
        let mut hr = HashReader { inner: (&mut f).take(s), hasher: Sha256::new() };
        io::copy(&mut hr, &mut io::sink()).unwrap();
        assert_eq!(hex(&hr.hasher.finalize()), m.kernel_sha256);
        skip_padding(&mut f, s).unwrap();
        let (n, s) = next_entry(&mut f).unwrap().unwrap();
        assert_eq!((n.as_str(), s), ("rootfs.ext4.gz", 1500));
        let _ = fs::remove_dir_all(dir);
    }
}
