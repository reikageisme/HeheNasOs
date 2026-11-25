//! Admin authentication: a single admin account (argon2id hash on disk) and
//! in-memory sessions carried in an HttpOnly, SameSite=Strict cookie.

use std::{
    collections::HashMap,
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::{
    util::{self, ApiError},
    AppState, Shared,
};

pub const COOKIE: &str = "rustnas_session";
const MAX_SESSIONS: usize = 32;

#[derive(Serialize, Deserialize, Default, Clone)]
struct AuthFile {
    admin: Option<Admin>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Admin {
    username: String,
    password_hash: String,
}

struct Session {
    user: String,
    expires: Instant,
}

pub struct AuthStore {
    path: PathBuf,
    file: Mutex<AuthFile>,
    sessions: Mutex<HashMap<String, Session>>,
    /// Failed logins per client IP: (count, first failure).
    fails: Mutex<HashMap<IpAddr, (u32, Instant)>>,
    ttl: Duration,
}

const MAX_FAILS: u32 = 5;
const FAIL_WINDOW: Duration = Duration::from_secs(15 * 60);
const LOCKOUT: Duration = Duration::from_secs(5 * 60);

impl AuthStore {
    pub fn load(state_dir: &Path, ttl_secs: u64) -> Result<Self, String> {
        let path = state_dir.join("auth.toml");
        let file = match fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(_) => AuthFile::default(),
        };
        Ok(Self {
            path,
            file: Mutex::new(file),
            sessions: Mutex::new(HashMap::new()),
            fails: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs.max(60)),
        })
    }

    pub fn is_setup(&self) -> bool {
        self.file.lock().unwrap().admin.is_some()
    }

    /// First-run: create the admin account. Fails once an admin exists.
    pub fn setup(&self, username: &str, password: &str) -> Result<(), String> {
        validate_username(username)?;
        validate_password(password)?;
        let password_hash = hash_password(password)?;
        let mut f = self.file.lock().unwrap();
        if f.admin.is_some() {
            return Err("Tài khoản quản trị đã tồn tại".into());
        }
        let next = AuthFile { admin: Some(Admin { username: username.into(), password_hash }) };
        self.save(&next)?;
        *f = next;
        Ok(())
    }

    /// CPU-heavy (argon2): call from a blocking thread.
    pub fn verify(&self, username: &str, password: &str) -> bool {
        let admin = self.file.lock().unwrap().admin.clone();
        let Some(admin) = admin else { return false };
        // Always run the hash so a wrong username costs the same time.
        let ok = verify_password(password, &admin.password_hash);
        ok && admin.username == username
    }

    /// Changes the password and logs out every session.
    pub fn change_password(&self, username: &str, old: &str, new: &str) -> Result<(), String> {
        if !self.verify(username, old) {
            return Err("Mật khẩu hiện tại không đúng".into());
        }
        validate_password(new)?;
        let password_hash = hash_password(new)?;
        {
            let mut f = self.file.lock().unwrap();
            let next = AuthFile { admin: Some(Admin { username: username.into(), password_hash }) };
            self.save(&next)?;
            *f = next;
        }
        self.sessions.lock().unwrap().clear();
        Ok(())
    }

    pub fn create_session(&self, user: &str) -> String {
        let token = util::random_hex(32);
        let now = Instant::now();
        let mut s = self.sessions.lock().unwrap();
        s.retain(|_, v| v.expires > now);
        if s.len() >= MAX_SESSIONS {
            let oldest = s.iter().min_by_key(|(_, v)| v.expires).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                s.remove(&k);
            }
        }
        s.insert(token.clone(), Session { user: user.into(), expires: now + self.ttl });
        token
    }

    /// Returns the session's user and extends it (sliding expiry).
    pub fn check(&self, token: &str) -> Option<String> {
        let now = Instant::now();
        let mut s = self.sessions.lock().unwrap();
        let valid = matches!(s.get(token), Some(sess) if sess.expires > now);
        if !valid {
            s.remove(token);
            return None;
        }
        let sess = s.get_mut(token)?;
        sess.expires = now + self.ttl;
        Some(sess.user.clone())
    }

    /// Seconds left if this IP is locked out after too many failures.
    pub fn locked_for(&self, ip: IpAddr) -> Option<u64> {
        let f = self.fails.lock().unwrap();
        let (n, first) = f.get(&ip)?;
        let until = *first + FAIL_WINDOW.max(LOCKOUT);
        (*n >= MAX_FAILS && Instant::now() < until).then(|| until.duration_since(Instant::now()).as_secs() + 1)
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut f = self.fails.lock().unwrap();
        f.retain(|_, (_, first)| now.duration_since(*first) < FAIL_WINDOW.max(LOCKOUT));
        if f.len() > 4096 {
            f.clear(); // bounded memory under a spray of addresses
        }
        let e = f.entry(ip).or_insert((0, now));
        e.0 += 1;
    }

    pub fn record_success(&self, ip: IpAddr) {
        self.fails.lock().unwrap().remove(&ip);
    }

    pub fn destroy(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }

    pub fn sweep(&self) {
        let now = Instant::now();
        self.sessions.lock().unwrap().retain(|_, v| v.expires > now);
    }

    fn save(&self, f: &AuthFile) -> Result<(), String> {
        let s = toml::to_string(f).map_err(|e| e.to_string())?;
        util::write_atomic(&self.path, s.as_bytes(), 0o600)
    }
}

/// 8 MiB / 3 passes: roughly 0.3–0.5 s on a Cortex-A53 and gentle on 1 GB RAM.
fn argon() -> Argon2<'static> {
    let params = Params::new(8 * 1024, 3, 1, None).expect("argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_password(pw: &str) -> Result<String, String> {
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|e| e.to_string())?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| e.to_string())?;
    argon()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| argon().verify_password(pw.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

fn validate_username(u: &str) -> Result<(), String> {
    let ok = !u.is_empty()
        && u.len() <= 32
        && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok { Ok(()) } else { Err("Tên đăng nhập chỉ gồm chữ, số, _ - . (tối đa 32 ký tự)".into()) }
}

fn validate_password(p: &str) -> Result<(), String> {
    let n = p.chars().count();
    if !(8..=128).contains(&n) {
        return Err("Mật khẩu cần từ 8 đến 128 ký tự".into());
    }
    if p.chars().any(char::is_control) {
        return Err("Mật khẩu chứa ký tự không hợp lệ".into());
    }
    Ok(())
}

pub fn token_from(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix("rustnas_session="))
        .map(str::to_string)
}

pub fn session_user(st: &AppState, headers: &HeaderMap) -> Option<String> {
    token_from(headers).and_then(|t| st.auth.check(&t))
}

/// Extractor: rejects the request with 401 unless a valid session cookie is sent.
pub struct AuthUser(pub String);

impl FromRequestParts<Shared> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &Shared) -> Result<Self, Self::Rejection> {
        session_user(st, &parts.headers).map(AuthUser).ok_or_else(|| {
            ApiError::new(StatusCode::UNAUTHORIZED, "Chưa đăng nhập hoặc phiên đã hết hạn")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip() {
        let h = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn lockout() {
        let dir = std::env::temp_dir();
        let a = AuthStore::load(&dir.join("nonexistent-rustnas"), 60).unwrap();
        let ip: IpAddr = "192.168.1.9".parse().unwrap();
        for _ in 0..MAX_FAILS {
            assert!(a.locked_for(ip).is_none());
            a.record_failure(ip);
        }
        assert!(a.locked_for(ip).is_some());
        a.record_success(ip);
        assert!(a.locked_for(ip).is_none());
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "a=1; rustnas_session=abc; b=2".parse().unwrap());
        assert_eq!(token_from(&h).as_deref(), Some("abc"));
    }
}
