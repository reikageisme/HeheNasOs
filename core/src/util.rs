//! Small shared helpers: logging, API errors, command execution, file writes.

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// Log to stderr; on the device stderr is piped into syslog (`logger`).
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { eprintln!("rustnasd: {}", format_args!($($arg)*)) };
}

pub struct ApiError {
    pub status: StatusCode,
    pub msg: String,
}

impl ApiError {
    pub fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        Self { status, msg: msg.into() }
    }
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

impl From<String> for ApiError {
    fn from(msg: String) -> Self {
        Self::bad(msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.msg }))).into_response()
    }
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// Read a small text file (sysfs/procfs), trimming whitespace and NULs.
pub fn read_trim(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p)
        .ok()
        .map(|s| s.trim_matches(|c: char| c.is_whitespace() || c == '\0').to_string())
}

/// Run a privileged command (no shell). In dry-run mode it is only logged.
pub fn run(dry_run: bool, prog: &str, args: &[&str]) -> Result<String, String> {
    if dry_run {
        crate::info!("[dry-run] {prog} {}", redact(args));
        return Ok(String::new());
    }
    let out = Command::new(prog)
        .args(args)
        .output()
        .map_err(|e| format!("Không chạy được {prog}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    crate::info!("{prog} {} failed: {err}", redact(args));
    Err(if err.is_empty() {
        format!("{prog} thất bại ({})", out.status)
    } else {
        format!("{prog}: {err}")
    })
}

/// Run a read-only helper (always executed, even in dry-run mode).
pub fn run_ro(prog: &str, args: &[&str]) -> Option<String> {
    Command::new(prog)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Hide the value following "-p" (passwords) when logging a command line.
fn redact(args: &[&str]) -> String {
    let mut out = Vec::with_capacity(args.len());
    let mut hide = false;
    for a in args {
        out.push(if hide { "***" } else { *a });
        hide = *a == "-p";
    }
    out.join(" ")
}

/// Write a file atomically (tmp + fsync + rename) so a power cut never leaves
/// a half-written config behind.
pub fn write_atomic(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let e = |err: std::io::Error| format!("{}: {err}", path.display());
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(e)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(e)?;
        f.set_permissions(fs::Permissions::from_mode(mode)).map_err(e)?;
        f.write_all(data).map_err(e)?;
        f.sync_all().map_err(e)?;
    }
    fs::rename(&tmp, path).map_err(e)
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::getrandom(&mut b).expect("getrandom");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn redacts_passwords() {
        assert_eq!(super::redact(&["-a", "bob", "-p", "secret"]), "-a bob -p ***");
    }
}
