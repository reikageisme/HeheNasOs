//! REST API. Every route except /api/auth/{status,setup,login,logout}
//! requires an admin session (the `AuthUser` extractor).
//!
//! CSRF: the session cookie is SameSite=Strict and mutating endpoints only
//! accept `application/json` bodies, which a cross-site form cannot send.

use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::{self, AuthUser},
    monitor, network,
    shares::Share,
    storage, system,
    util::{self, ApiError, ApiResult},
    web, Shared,
};

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/setup", post(auth_setup))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/password", post(auth_password))
        .route("/api/system/overview", get(sys_overview))
        .route("/api/system/history", get(sys_history))
        .route("/api/system/services", get(sys_services))
        .route("/api/system/services/{id}/{action}", post(sys_service_action))
        .route("/api/system/logs", get(sys_logs))
        .route("/api/system/power", post(sys_power))
        .route("/api/storage/disks", get(storage_disks))
        .route("/api/storage/mount", post(storage_mount))
        .route("/api/storage/unmount", post(storage_unmount))
        .route("/api/storage/format", post(storage_format))
        .route("/api/shares", get(shares_list).post(shares_upsert))
        .route("/api/shares/{name}", delete(shares_delete))
        .route("/api/smb-users", post(smb_user_set))
        .route("/api/smb-users/{name}", delete(smb_user_delete))
        .route("/api/network", get(net_info))
        .route("/api/network/hostname", post(net_hostname))
        .fallback(web::fallback)
        .with_state(state)
}

/// Run blocking work (hashing, mkfs, mount, file writes) off the event loop.
async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(ApiError::bad)
}

fn ok() -> Json<Value> {
    Json(json!({ "ok": true }))
}

// ---- auth -------------------------------------------------------------------

#[derive(Deserialize)]
struct Creds {
    username: String,
    password: String,
}

async fn auth_status(State(st): State<Shared>, headers: HeaderMap) -> Json<Value> {
    Json(json!({
        "setup_required": !st.auth.is_setup(),
        "user": auth::session_user(&st, &headers),
        "version": env!("CARGO_PKG_VERSION"),
        "hostname": network::hostname(),
    }))
}

async fn auth_setup(State(st): State<Shared>, Json(c): Json<Creds>) -> Result<Response, ApiError> {
    if st.auth.is_setup() {
        return Err(ApiError::new(StatusCode::CONFLICT, "Tài khoản quản trị đã được thiết lập"));
    }
    let Creds { username, password } = c;
    let (st2, u2) = (st.clone(), username.clone());
    blocking(move || st2.auth.setup(&u2, &password)).await?;
    crate::info!("admin account {username:?} created");
    Ok(session_response(&st, &username))
}

async fn auth_login(State(st): State<Shared>, Json(c): Json<Creds>) -> Result<Response, ApiError> {
    let Creds { username, password } = c;
    let (st2, u2) = (st.clone(), username.clone());
    let ok = tokio::task::spawn_blocking(move || st2.auth.verify(&u2, &password)).await.unwrap_or(false);
    if !ok {
        crate::info!("failed login for {username:?}");
        tokio::time::sleep(Duration::from_millis(1500)).await; // slow down guessing
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Sai tên đăng nhập hoặc mật khẩu"));
    }
    Ok(session_response(&st, &username))
}

fn session_response(st: &Shared, user: &str) -> Response {
    let token = st.auth.create_session(user);
    let cookie = format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        auth::COOKIE,
        st.cfg.session_ttl_secs
    );
    ([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true, "user": user }))).into_response()
}

async fn auth_logout(State(st): State<Shared>, headers: HeaderMap) -> Response {
    if let Some(t) = auth::token_from(&headers) {
        st.auth.destroy(&t);
    }
    let cookie = format!("{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0", auth::COOKIE);
    ([(header::SET_COOKIE, cookie)], ok()).into_response()
}

#[derive(Deserialize)]
struct PasswordChange {
    old_password: String,
    new_password: String,
}

async fn auth_password(
    AuthUser(user): AuthUser,
    State(st): State<Shared>,
    Json(r): Json<PasswordChange>,
) -> ApiResult<Value> {
    blocking(move || st.auth.change_password(&user, &r.old_password, &r.new_password)).await?;
    Ok(ok())
}

// ---- system -----------------------------------------------------------------

async fn sys_overview(_: AuthUser, State(st): State<Shared>) -> Json<monitor::Snapshot> {
    Json(st.monitor.snapshot())
}

async fn sys_history(_: AuthUser, State(st): State<Shared>) -> Json<Vec<monitor::Point>> {
    Json(st.monitor.history())
}

async fn sys_services(_: AuthUser, State(st): State<Shared>) -> ApiResult<Vec<system::ServiceStatus>> {
    blocking(move || Ok(system::services(&st.cfg))).await.map(Json)
}

async fn sys_service_action(
    _: AuthUser,
    State(st): State<Shared>,
    Path((id, action)): Path<(String, String)>,
) -> ApiResult<Value> {
    blocking(move || system::service_action(&st.cfg, &id, &action)).await?;
    Ok(ok())
}

async fn sys_logs(_: AuthUser, State(st): State<Shared>) -> ApiResult<Value> {
    let path = st.cfg.log_file.clone();
    let file = path.display().to_string();
    let lines = blocking(move || Ok(system::tail(&path, 300))).await?;
    Ok(Json(json!({ "file": file, "lines": lines })))
}

#[derive(Deserialize)]
struct Power {
    action: String,
}

async fn sys_power(AuthUser(user): AuthUser, State(st): State<Shared>, Json(p): Json<Power>) -> ApiResult<Value> {
    let cmd = match p.action.as_str() {
        "reboot" => "reboot",
        "poweroff" => "poweroff",
        _ => return Err(ApiError::bad("Thao tác không hợp lệ")),
    };
    crate::info!("{cmd} requested by {user:?}");
    let dry = st.cfg.dry_run;
    // Answer the request first, then act.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let _ = tokio::task::spawn_blocking(move || util::run(dry, cmd, &[])).await;
    });
    Ok(ok())
}

// ---- storage ----------------------------------------------------------------

#[derive(Deserialize)]
struct DeviceReq {
    device: String,
}

#[derive(Deserialize)]
struct FormatReq {
    device: String,
    #[serde(default)]
    label: String,
    confirm: String,
}

async fn storage_disks(_: AuthUser, State(st): State<Shared>) -> ApiResult<Vec<storage::Disk>> {
    blocking(move || Ok(storage::list(&st.cfg))).await.map(Json)
}

async fn storage_mount(_: AuthUser, State(st): State<Shared>, Json(r): Json<DeviceReq>) -> ApiResult<Value> {
    let mountpoint = blocking(move || {
        let _ops = st.ops.lock().unwrap();
        let mp = storage::mount(&st.cfg, &r.device)?;
        // Shares on this disk become reachable again.
        if let Err(e) = st.shares.lock().unwrap().apply(&st.cfg) {
            crate::info!("SMB reload: {e}");
        }
        Ok(mp)
    })
    .await?;
    Ok(Json(json!({ "ok": true, "mountpoint": mountpoint })))
}

async fn storage_unmount(_: AuthUser, State(st): State<Shared>, Json(r): Json<DeviceReq>) -> ApiResult<Value> {
    blocking(move || {
        let _ops = st.ops.lock().unwrap();
        let paths = st.shares.lock().unwrap().paths();
        storage::unmount(&st.cfg, &r.device, &paths)
    })
    .await?;
    Ok(ok())
}

async fn storage_format(AuthUser(user): AuthUser, State(st): State<Shared>, Json(r): Json<FormatReq>) -> ApiResult<Value> {
    crate::info!("format of {:?} requested by {user:?}", r.device);
    blocking(move || {
        let _ops = st.ops.lock().unwrap();
        storage::format(&st.cfg, &r.device, &r.label, &r.confirm)
    })
    .await?;
    Ok(ok())
}

// ---- shares -----------------------------------------------------------------

async fn shares_list(_: AuthUser, State(st): State<Shared>) -> Json<Value> {
    let s = st.shares.lock().unwrap();
    Json(json!({ "shares": s.shares(), "users": s.users(), "workgroup": s.workgroup() }))
}

async fn shares_upsert(_: AuthUser, State(st): State<Shared>, Json(share): Json<Share>) -> ApiResult<Value> {
    blocking(move || st.shares.lock().unwrap().upsert(&st.cfg, share)).await?;
    Ok(ok())
}

async fn shares_delete(_: AuthUser, State(st): State<Shared>, Path(name): Path<String>) -> ApiResult<Value> {
    blocking(move || st.shares.lock().unwrap().delete(&st.cfg, &name)).await?;
    Ok(ok())
}

#[derive(Deserialize)]
struct SmbUser {
    username: String,
    password: String,
}

async fn smb_user_set(_: AuthUser, State(st): State<Shared>, Json(u): Json<SmbUser>) -> ApiResult<Value> {
    blocking(move || st.shares.lock().unwrap().set_user(&st.cfg, &u.username, &u.password)).await?;
    Ok(ok())
}

async fn smb_user_delete(_: AuthUser, State(st): State<Shared>, Path(name): Path<String>) -> ApiResult<Value> {
    blocking(move || st.shares.lock().unwrap().delete_user(&st.cfg, &name)).await?;
    Ok(ok())
}

// ---- network ----------------------------------------------------------------

async fn net_info(_: AuthUser) -> ApiResult<network::NetInfo> {
    blocking(|| Ok(network::info())).await.map(Json)
}

#[derive(Deserialize)]
struct Hostname {
    hostname: String,
}

async fn net_hostname(_: AuthUser, State(st): State<Shared>, Json(h): Json<Hostname>) -> ApiResult<Value> {
    blocking(move || {
        network::set_hostname(st.cfg.dry_run, &h.hostname)?;
        st.shares.lock().unwrap().apply(&st.cfg) // NetBIOS name follows the hostname
    })
    .await?;
    Ok(ok())
}
