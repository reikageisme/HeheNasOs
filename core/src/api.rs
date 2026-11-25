//! REST API. Everything except /api/auth/{status,setup,login,logout} and
//! /api/network/confirm (token-protected) requires an admin session.
//! Mutating requests must also carry `X-HeheNAS: 1` (see web::csrf_guard).

use std::time::Duration;

use axum::{
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::{
    alerts,
    auth::{self, AuthUser},
    backup, files, monitor,
    netcfg::{self, FwConfig, NetConfig, Pending},
    network,
    privileged::{self, Op},
    shares::Share,
    storage, system, tls, update,
    util::{self, ApiError, ApiResult},
    web::{self, Secure},
    Shared,
};

pub fn router(state: Shared) -> Router {
    Router::new()
        // auth
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/setup", post(auth_setup))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/password", post(auth_password))
        // system
        .route("/api/alerts", get(get_alerts))
        .route("/api/system/overview", get(sys_overview))
        .route("/api/system/history", get(sys_history))
        .route("/api/system/services", get(sys_services))
        .route("/api/system/services/{id}/{action}", post(sys_service_action))
        .route("/api/system/logs", get(sys_logs))
        .route("/api/system/power", post(sys_power))
        .route("/api/system/backup", get(sys_backup))
        .route("/api/system/restore", post(sys_restore).layer(DefaultBodyLimit::max(8 << 20)))
        .route("/api/system/update", get(update_status).put(update_upload))
        .route("/api/system/update/rollback", post(update_rollback))
        .route("/api/system/update/check", post(update_check))
        .route("/api/system/update/install", post(update_install))
        .route("/api/system/update/settings", post(update_settings))
        .route("/api/settings", get(settings_get).post(settings_set))
        .route("/api/settings/regenerate-cert", post(settings_regen_cert))
        // storage
        .route("/api/storage/disks", get(storage_disks))
        .route("/api/storage/smart/{disk}", get(storage_smart))
        .route("/api/storage/mount", post(storage_mount))
        .route("/api/storage/unmount", post(storage_unmount))
        .route("/api/storage/format", post(storage_format))
        .route("/api/storage/initialize", post(storage_initialize))
        .route("/api/storage/fix-owner", post(storage_fix_owner))
        // files
        .route("/api/files", get(files_list))
        .route("/api/files/download", get(files_download))
        .route("/api/files/upload", axum::routing::put(files_upload))
        .route("/api/files/mkdir", post(files_mkdir))
        .route("/api/files/rename", post(files_rename))
        .route("/api/files/delete", post(files_delete))
        .route("/api/trash", get(trash_list))
        .route("/api/trash/restore", post(trash_restore))
        .route("/api/trash/purge", post(trash_purge))
        // shares
        .route("/api/shares", get(shares_list).post(shares_upsert))
        .route("/api/shares/{name}", delete(shares_delete))
        .route("/api/nfs", post(nfs_toggle))
        .route("/api/smb-users", post(smb_user_set))
        .route("/api/smb-users/{name}", delete(smb_user_delete))
        // network
        .route("/api/network", get(net_info))
        .route("/api/network/config", post(net_config))
        .route("/api/network/hostname", post(net_hostname))
        .route("/api/network/confirm", post(net_confirm))
        .route("/api/network/revert", post(net_revert))
        .route("/api/firewall", get(fw_get).post(fw_set))
        .fallback(web::fallback)
        .layer(middleware::from_fn(web::csrf_guard))
        .layer(middleware::from_fn(web::security_headers))
        .with_state(state)
}

/// Run blocking work (hashing, helper calls, file writes) off the event loop.
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

/// Exit after the response is sent; init (or the helper) restarts us.
fn schedule_restart(why: &str) {
    crate::info!("restarting: {why}");
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        std::process::exit(0);
    });
}

// ---- auth -------------------------------------------------------------------

#[derive(Deserialize)]
struct Creds {
    username: String,
    password: String,
}

async fn auth_status(State(st): State<Shared>, Extension(sec): Extension<Secure>, headers: HeaderMap) -> Json<Value> {
    Json(json!({
        "setup_required": !st.auth.is_setup(),
        "user": auth::session_user(&st, &headers),
        "version": env!("CARGO_PKG_VERSION"),
        "hostname": network::hostname(),
        "secure": sec.0,
        "demo": false,
    }))
}

fn session_response(st: &Shared, user: &str, secure: bool) -> Response {
    let token = st.auth.create_session(user);
    let cookie = format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        auth::COOKIE,
        st.cfg.session_ttl_secs,
        if secure { "; Secure" } else { "" }
    );
    ([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true, "user": user }))).into_response()
}

async fn auth_setup(
    State(st): State<Shared>,
    Extension(sec): Extension<Secure>,
    Json(c): Json<Creds>,
) -> Result<Response, ApiError> {
    if st.auth.is_setup() {
        return Err(ApiError::new(StatusCode::CONFLICT, "Tài khoản quản trị đã được thiết lập"));
    }
    let Creds { username, password } = c;
    let (st2, u2) = (st.clone(), username.clone());
    blocking(move || st2.auth.setup(&u2, &password)).await?;
    crate::info!("admin account {username:?} created");
    Ok(session_response(&st, &username, sec.0))
}

async fn auth_login(
    State(st): State<Shared>,
    Extension(sec): Extension<Secure>,
    ConnectInfo(tls::Peer(peer)): ConnectInfo<tls::Peer>,
    Json(c): Json<Creds>,
) -> Result<Response, ApiError> {
    let ip = peer.ip();
    if let Some(secs) = st.auth.locked_for(ip) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            format!("Sai quá nhiều lần. Thử lại sau {} phút.", secs.div_ceil(60)),
        ));
    }
    let Creds { username, password } = c;
    let (st2, u2) = (st.clone(), username.clone());
    let ok = tokio::task::spawn_blocking(move || st2.auth.verify(&u2, &password)).await.unwrap_or(false);
    if !ok {
        st.auth.record_failure(ip);
        crate::info!("failed login for {username:?} from {ip}");
        tokio::time::sleep(Duration::from_millis(1000)).await;
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Sai tên đăng nhập hoặc mật khẩu"));
    }
    st.auth.record_success(ip);
    crate::info!("login {username:?} from {ip}");
    Ok(session_response(&st, &username, sec.0))
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

async fn auth_password(AuthUser(user): AuthUser, State(st): State<Shared>, Json(r): Json<PasswordChange>) -> ApiResult<Value> {
    blocking(move || st.auth.change_password(&user, &r.old_password, &r.new_password)).await?;
    Ok(ok())
}

// ---- system -----------------------------------------------------------------

async fn get_alerts(_: AuthUser, State(st): State<Shared>) -> Json<Vec<alerts::Alert>> {
    Json(st.alerts.lock().unwrap().clone())
}

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

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default)]
    source: String,
    #[serde(default)]
    q: String,
}

async fn sys_logs(_: AuthUser, State(st): State<Shared>, Query(q): Query<LogQuery>) -> ApiResult<Value> {
    let path = st.cfg.log_file.clone();
    let file = path.display().to_string();
    let lines = blocking(move || Ok(system::tail(&path, 400, &q.source, &q.q))).await?;
    Ok(Json(json!({ "file": file, "lines": lines })))
}

#[derive(Deserialize)]
struct Power {
    action: String,
}

async fn sys_power(AuthUser(user): AuthUser, State(st): State<Shared>, Json(p): Json<Power>) -> ApiResult<Value> {
    if !matches!(p.action.as_str(), "reboot" | "poweroff") {
        return Err(ApiError::bad("Thao tác không hợp lệ"));
    }
    crate::info!("{} requested by {user:?}", p.action);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let _ = tokio::task::spawn_blocking(move || privileged::call(&st.cfg, Op::Power { action: p.action })).await;
    });
    Ok(ok())
}

#[derive(Deserialize)]
struct BackupQuery {
    #[serde(default)]
    secrets: Option<u8>,
}

async fn sys_backup(_: AuthUser, State(st): State<Shared>, Query(q): Query<BackupQuery>) -> Result<Response, ApiError> {
    let secrets = q.secrets == Some(1);
    let b = blocking(move || backup::create(&st.cfg, secrets)).await?;
    let name = format!("hehenas-backup-{}-{}.json", b.hostname, b.created);
    let body = serde_json::to_vec_pretty(&b).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        body,
    )
        .into_response())
}

async fn sys_restore(AuthUser(user): AuthUser, State(st): State<Shared>, Json(b): Json<backup::Backup>) -> ApiResult<Value> {
    crate::info!("restore requested by {user:?}");
    blocking(move || backup::restore(&st.cfg, &b)).await?;
    schedule_restart("configuration restored");
    Ok(ok())
}

// ---- updates ----------------------------------------------------------------

async fn update_status(_: AuthUser, State(st): State<Shared>) -> Json<Value> {
    let s = st.settings.lock().unwrap().clone();
    let mut v = serde_json::to_value(update::status(&st.cfg)).unwrap_or_default();
    v["online"] = serde_json::to_value(st.online.lock().unwrap().clone()).unwrap_or_default();
    v["job"] = serde_json::to_value(st.job.lock().unwrap().clone()).unwrap_or_default();
    v["auto_update"] = json!(s.auto_update);
    v["update_hour"] = json!(s.update_hour);
    v["tz_offset"] = json!(s.tz_offset);
    v["clock_ok"] = json!(update::clock_ok());
    Json(v)
}

async fn update_check(_: AuthUser, State(st): State<Shared>) -> ApiResult<update::Online> {
    let st2 = st.clone();
    let o = blocking(move || Ok(update::check(&st2.cfg))).await?;
    *st.online.lock().unwrap() = o.clone();
    Ok(Json(o))
}

/// Download + verify + stage the latest release in the background.
async fn update_install(AuthUser(user): AuthUser, State(st): State<Shared>) -> ApiResult<Value> {
    {
        let o = st.online.lock().unwrap();
        if !o.available {
            return Err(ApiError::bad("Chưa có bản mới — bấm \"Kiểm tra\" trước"));
        }
    }
    let busy = matches!(st.job.lock().unwrap().state.as_str(), "downloading" | "installing");
    if busy {
        return Err(ApiError::new(StatusCode::CONFLICT, "Đang cập nhật"));
    }
    crate::info!("OTA update requested by {user:?}");
    tokio::task::spawn_blocking(move || {
        if let Err(e) = ota_install(&st) {
            crate::info!("OTA update failed: {e}");
        }
    });
    Ok(ok())
}

#[derive(Deserialize)]
struct UpdateSettings {
    auto_update: String,
    update_hour: u8,
}

async fn update_settings(_: AuthUser, State(st): State<Shared>, Json(u): Json<UpdateSettings>) -> ApiResult<Value> {
    if !matches!(u.auto_update.as_str(), "off" | "notify" | "install") || u.update_hour > 23 {
        return Err(ApiError::bad("Cài đặt không hợp lệ"));
    }
    let mut s = st.settings.lock().unwrap().clone();
    s.auto_update = u.auto_update;
    s.update_hour = u.update_hour;
    let (st2, s2) = (st.clone(), s.clone());
    blocking(move || s2.save(&st2.cfg)).await?;
    *st.settings.lock().unwrap() = s;
    Ok(ok())
}

/// Shared by the "Update now" button and automatic updates (blocking).
pub fn ota_install(st: &Shared) -> Result<String, String> {
    let o = st.online.lock().unwrap().clone();
    let (Some(url), Some(version)) = (o.asset_url.clone(), o.latest.clone()) else {
        return Err("Chưa có bản mới".into());
    };
    if !o.available {
        return Err("Đang chạy bản mới nhất".into());
    }
    {
        let mut j = st.job.lock().unwrap();
        if matches!(j.state.as_str(), "downloading" | "installing") {
            return Err("Đang cập nhật".into());
        }
        *j = update::Job { state: "downloading".into(), version: version.clone(), total: o.asset_size, ..Default::default() };
    }
    let res = (|| {
        let path = update::bundle_path(&st.cfg, o.asset_size)?;
        let dir = path.parent().unwrap_or(&path).to_path_buf();
        if path.starts_with(&st.cfg.mount_root) {
            privileged::call(&st.cfg, Op::PrepareDir { path: dir.to_string_lossy().into_owned() })?;
        } else {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        crate::info!("OTA: downloading {version} ({} bytes)", o.asset_size);
        let st2 = st.clone();
        update::download(&url, o.asset_size, &path, &|n| st2.job.lock().unwrap().downloaded = n)?;
        st.job.lock().unwrap().state = "installing".into();
        let r = privileged::call(&st.cfg, Op::UpdateApply { bundle: path.to_string_lossy().into_owned() });
        let _ = std::fs::remove_file(&path);
        r
    })();
    let mut j = st.job.lock().unwrap();
    match &res {
        Ok(_) => {
            j.state = "staged".into();
            crate::info!("OTA: {version} staged — reboot to switch");
        }
        Err(e) => {
            j.state = "error".into();
            j.error = Some(e.clone());
        }
    }
    res
}

/// Stream the uploaded bundle to a data disk, then let the helper verify and
/// write it to the inactive slot.
async fn update_upload(
    AuthUser(user): AuthUser,
    State(st): State<Shared>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Value> {
    let cfg = st.cfg.clone();
    let len = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300 << 20);
    let path = blocking(move || {
        let p = update::bundle_path(&cfg, len)?;
        let dir = p.parent().unwrap_or(&p).to_path_buf();
        if p.starts_with(&cfg.mount_root) {
            privileged::call(&cfg, Op::PrepareDir { path: dir.to_string_lossy().into_owned() })?;
        } else {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        Ok(p)
    })
    .await?;
    crate::info!("update upload by {user:?} -> {}", path.display());
    stream_to_file(body, &path, &path).await?;
    let (cfg, bundle) = (st.cfg.clone(), path.to_string_lossy().into_owned());
    let version = blocking(move || {
        let r = privileged::call(&cfg, Op::UpdateApply { bundle: bundle.clone() });
        let _ = std::fs::remove_file(&bundle);
        r
    })
    .await?;
    Ok(Json(json!({ "ok": true, "version": version })))
}

async fn update_rollback(_: AuthUser, State(st): State<Shared>) -> ApiResult<Value> {
    let slot = blocking(move || privileged::call(&st.cfg, Op::UpdateRollback)).await?;
    Ok(Json(json!({ "ok": true, "slot": slot })))
}

// ---- settings ---------------------------------------------------------------

async fn settings_get(_: AuthUser, State(st): State<Shared>) -> Json<Value> {
    let s = st.settings.lock().unwrap().clone();
    Json(json!({
        "https": s.https,
        "http_redirect": s.http_redirect,
        "http_port": crate::config::Config::port_of(&st.cfg.listen),
        "https_port": crate::config::Config::port_of(&st.cfg.https_listen),
        "fingerprint": tls::fingerprint(&st.cfg),
    }))
}

#[derive(Deserialize)]
struct WebSettings {
    https: bool,
    http_redirect: bool,
}

async fn settings_set(_: AuthUser, State(st): State<Shared>, Json(w): Json<WebSettings>) -> ApiResult<Value> {
    let mut s = st.settings.lock().unwrap().clone();
    s.https = w.https;
    s.http_redirect = w.http_redirect;
    let st2 = st.clone();
    let s2 = s.clone();
    blocking(move || s2.save(&st2.cfg)).await?;
    *st.settings.lock().unwrap() = s;
    schedule_restart("settings changed");
    Ok(ok())
}

async fn settings_regen_cert(_: AuthUser, State(st): State<Shared>) -> ApiResult<Value> {
    blocking(move || tls::generate(&st.cfg)).await?;
    schedule_restart("new TLS certificate");
    Ok(ok())
}

// ---- storage ----------------------------------------------------------------

#[derive(Deserialize)]
struct DeviceReq {
    device: String,
}

#[derive(Deserialize)]
struct WipeReq {
    device: String,
    #[serde(default)]
    label: String,
    confirm: String,
}

#[derive(Deserialize)]
struct PathReq {
    path: String,
}

async fn storage_disks(_: AuthUser, State(st): State<Shared>) -> ApiResult<Value> {
    let st2 = st.clone();
    let disks = blocking(move || Ok(storage::list(&st2.cfg))).await?;
    let smart = st.smart.lock().unwrap().clone();
    Ok(Json(json!({ "disks": disks, "smart": smart })))
}

async fn storage_smart(_: AuthUser, State(st): State<Shared>, Path(disk): Path<String>) -> ApiResult<storage::Smart> {
    let st2 = st.clone();
    let d = disk.clone();
    let s = blocking(move || Ok(storage::smart(&st2.cfg, &d))).await?;
    st.smart.lock().unwrap().insert(disk, s.clone());
    Ok(Json(s))
}

async fn storage_mount(_: AuthUser, State(st): State<Shared>, Json(r): Json<DeviceReq>) -> ApiResult<Value> {
    let mountpoint = blocking(move || {
        let _ops = st.ops.lock().unwrap();
        let mp = storage::mount(&st.cfg, &r.device)?;
        if let Err(e) = st.shares.lock().unwrap().apply(&st.cfg) {
            crate::info!("share reload: {e}");
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

async fn storage_format(AuthUser(user): AuthUser, State(st): State<Shared>, Json(r): Json<WipeReq>) -> ApiResult<Value> {
    crate::info!("format of {:?} requested by {user:?}", r.device);
    blocking(move || {
        let _ops = st.ops.lock().unwrap();
        storage::format(&st.cfg, &r.device, &r.label, &r.confirm)
    })
    .await?;
    Ok(ok())
}

async fn storage_initialize(AuthUser(user): AuthUser, State(st): State<Shared>, Json(r): Json<WipeReq>) -> ApiResult<Value> {
    crate::info!("initialise of {:?} requested by {user:?}", r.device);
    blocking(move || {
        let _ops = st.ops.lock().unwrap();
        storage::initialize(&st.cfg, &r.device, &r.label, &r.confirm)
    })
    .await?;
    Ok(ok())
}

async fn storage_fix_owner(_: AuthUser, State(st): State<Shared>, Json(r): Json<PathReq>) -> ApiResult<Value> {
    blocking(move || {
        let _ops = st.ops.lock().unwrap();
        storage::fix_owner(&st.cfg, &r.path)
    })
    .await?;
    Ok(ok())
}

// ---- files ------------------------------------------------------------------

#[derive(Deserialize)]
struct FilesQuery {
    #[serde(default)]
    path: String,
}

async fn files_list(_: AuthUser, State(st): State<Shared>, Query(q): Query<FilesQuery>) -> ApiResult<Value> {
    blocking(move || {
        if q.path.is_empty() || q.path == "/" || std::path::Path::new(&q.path) == st.cfg.mount_root {
            return Ok(json!({ "roots": files::roots(&st.cfg) }));
        }
        serde_json::to_value(files::list(&st.cfg, &q.path)?).map_err(|e| e.to_string())
    })
    .await
    .map(Json)
}

/// RFC 5987 encoding for Content-Disposition filename*.
fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }
        })
        .collect()
}

async fn files_download(_: AuthUser, State(st): State<Shared>, Query(q): Query<FilesQuery>) -> Result<Response, ApiError> {
    let cfg = st.cfg.clone();
    let (real, _) = blocking(move || files::resolve(&cfg, &q.path)).await?;
    let file = tokio::fs::File::open(&real).await.map_err(|e| ApiError::bad(e.to_string()))?;
    let meta = file.metadata().await.map_err(|e| ApiError::bad(e.to_string()))?;
    if meta.is_dir() {
        return Err(ApiError::bad("Không tải được cả thư mục"));
    }
    let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' }).collect();
    let body = Body::from_stream(ReaderStream::with_capacity(file, 64 * 1024));
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, meta.len().to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", pct_encode(&name))),
        ],
        body,
    )
        .into_response())
}

/// Write a request body to `part`, then rename it to `target` (never a
/// half-written file under the real name).
async fn stream_to_file(body: Body, part: &std::path::Path, target: &std::path::Path) -> Result<u64, ApiError> {
    let err = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::PermissionDenied => ApiError::bad("Không có quyền ghi (thử 'Sửa quyền' ở trang Ổ đĩa)"),
        _ => ApiError::bad(e.to_string()),
    };
    let mut f = tokio::fs::File::create(part).await.map_err(err)?;
    let mut stream = body.into_data_stream();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let res = match chunk {
            Ok(c) => {
                total += c.len() as u64;
                f.write_all(&c).await.map_err(err)
            }
            Err(_) => Err(ApiError::bad("Tải lên bị gián đoạn")),
        };
        if let Err(e) = res {
            drop(f);
            let _ = tokio::fs::remove_file(part).await;
            return Err(e);
        }
    }
    f.sync_all().await.map_err(err)?;
    drop(f);
    if part != target {
        tokio::fs::rename(part, target).await.map_err(err)?;
    }
    Ok(total)
}

#[derive(Deserialize)]
struct UploadQuery {
    dir: String,
    name: String,
    #[serde(default)]
    overwrite: Option<u8>,
}

async fn files_upload(_: AuthUser, State(st): State<Shared>, Query(q): Query<UploadQuery>, body: Body) -> ApiResult<Value> {
    let cfg = st.cfg.clone();
    let (target, part) = blocking(move || files::upload_target(&cfg, &q.dir, &q.name, q.overwrite == Some(1))).await?;
    let size = stream_to_file(body, &part, &target).await?;
    Ok(Json(json!({ "ok": true, "size": size })))
}

#[derive(Deserialize)]
struct NameReq {
    path: String,
    name: String,
}

async fn files_mkdir(_: AuthUser, State(st): State<Shared>, Json(r): Json<NameReq>) -> ApiResult<Value> {
    blocking(move || files::mkdir(&st.cfg, &r.path, &r.name)).await?;
    Ok(ok())
}

async fn files_rename(_: AuthUser, State(st): State<Shared>, Json(r): Json<NameReq>) -> ApiResult<Value> {
    blocking(move || files::rename(&st.cfg, &r.path, &r.name)).await?;
    Ok(ok())
}

#[derive(Deserialize)]
struct PathsReq {
    paths: Vec<String>,
}

async fn files_delete(_: AuthUser, State(st): State<Shared>, Json(r): Json<PathsReq>) -> ApiResult<Value> {
    if r.paths.len() > 1000 {
        return Err(ApiError::bad("Quá nhiều mục"));
    }
    let n = blocking(move || files::delete(&st.cfg, &r.paths)).await?;
    Ok(Json(json!({ "ok": true, "moved": n })))
}

async fn trash_list(_: AuthUser, State(st): State<Shared>) -> ApiResult<Vec<files::TrashItem>> {
    blocking(move || Ok(files::trash_list(&st.cfg))).await.map(Json)
}

#[derive(Deserialize)]
struct TrashReq {
    #[serde(default)]
    mount: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

async fn trash_restore(_: AuthUser, State(st): State<Shared>, Json(r): Json<TrashReq>) -> ApiResult<Value> {
    let (Some(m), Some(id)) = (r.mount, r.id) else { return Err(ApiError::bad("Thiếu mount/id")) };
    let path = blocking(move || files::trash_restore(&st.cfg, &m, &id)).await?;
    Ok(Json(json!({ "ok": true, "path": path })))
}

async fn trash_purge(_: AuthUser, State(st): State<Shared>, Json(r): Json<TrashReq>) -> ApiResult<Value> {
    let n = blocking(move || files::trash_purge(&st.cfg, r.mount.as_deref(), r.id.as_deref())).await?;
    Ok(Json(json!({ "ok": true, "removed": n })))
}

// ---- shares -----------------------------------------------------------------

async fn shares_list(_: AuthUser, State(st): State<Shared>) -> Json<Value> {
    let nfs_available = privileged::service_script(&st.cfg, "nfs").is_some();
    let s = st.shares.lock().unwrap();
    Json(json!({
        "shares": s.shares(),
        "users": s.users(),
        "workgroup": s.workgroup(),
        "nfs_enabled": s.nfs_enabled(),
        "nfs_available": nfs_available,
    }))
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
struct Toggle {
    enabled: bool,
}

async fn nfs_toggle(_: AuthUser, State(st): State<Shared>, Json(t): Json<Toggle>) -> ApiResult<Value> {
    blocking(move || st.shares.lock().unwrap().set_nfs_enabled(&st.cfg, t.enabled)).await?;
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

// ---- network & firewall (with automatic rollback) ------------------------------------

async fn net_info(_: AuthUser, State(st): State<Shared>) -> ApiResult<Value> {
    let st2 = st.clone();
    let (info, cfgn) = blocking(move || Ok((network::info(), netcfg::load_net(&st2.cfg)))).await?;
    let pending = st.pending.lock().unwrap().clone();
    Ok(Json(json!({ "info": info, "config": cfgn, "pending": pending, "rollback_secs": netcfg::ROLLBACK_SECS })))
}

/// Apply a network or firewall change and arm the rollback timer.
async fn begin_change(st: Shared, net: Option<NetConfig>, fw: Option<FwConfig>) -> Result<Pending, ApiError> {
    if st.pending.lock().unwrap().is_some() {
        return Err(ApiError::new(StatusCode::CONFLICT, "Đang có một thay đổi mạng chờ xác nhận"));
    }
    let kind = if net.is_some() { "network" } else { "firewall" };
    let st2 = st.clone();
    let (net2, fw2) = (net.clone(), fw.clone());
    let (prev_net, prev_fw) = blocking(move || {
        let (pn, pf) = (netcfg::load_net(&st2.cfg), netcfg::load_fw(&st2.cfg));
        if let Some(n) = net2 {
            privileged::call(&st2.cfg, Op::ApplyNetwork { net: n })?;
        }
        if let Some(f) = fw2 {
            privileged::call(&st2.cfg, Op::ApplyFirewall { fw: f })?;
        }
        Ok((pn, pf))
    })
    .await?;
    let p = Pending {
        kind: kind.into(),
        token: util::random_hex(16),
        deadline: util::now_secs() + netcfg::ROLLBACK_SECS,
        prev_net: net.as_ref().map(|_| prev_net),
        prev_fw: fw.as_ref().map(|_| prev_fw),
        new_net: net,
        new_fw: fw,
    };
    *st.pending.lock().unwrap() = Some(p.clone());
    let (st3, token) = (st.clone(), p.token.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(netcfg::ROLLBACK_SECS)).await;
        let expired = st3.pending.lock().unwrap().as_ref().is_some_and(|p| p.token == token);
        if expired {
            crate::info!("network change not confirmed — rolling back");
            revert(st3).await;
        }
    });
    Ok(p)
}

async fn revert(st: Shared) {
    let Some(p) = st.pending.lock().unwrap().take() else { return };
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(n) = p.prev_net {
            let _ = privileged::call(&st.cfg, Op::ApplyNetwork { net: n });
        }
        if let Some(f) = p.prev_fw {
            let _ = privileged::call(&st.cfg, Op::ApplyFirewall { fw: f });
        }
    })
    .await;
}

async fn net_config(_: AuthUser, State(st): State<Shared>, Json(n): Json<NetConfig>) -> ApiResult<Value> {
    let n = netcfg::validate_net(&n).map_err(ApiError::bad)?;
    let p = begin_change(st, Some(n.clone()), None).await?;
    Ok(Json(json!({ "ok": true, "token": p.token, "deadline": p.deadline, "config": n })))
}

async fn fw_get(_: AuthUser, State(st): State<Shared>) -> Json<FwConfig> {
    Json(netcfg::load_fw(&st.cfg))
}

async fn fw_set(_: AuthUser, State(st): State<Shared>, Json(f): Json<FwConfig>) -> ApiResult<Value> {
    let f = netcfg::validate_fw(&f).map_err(ApiError::bad)?;
    let p = begin_change(st, None, Some(f)).await?;
    Ok(Json(json!({ "ok": true, "token": p.token, "deadline": p.deadline })))
}

#[derive(Deserialize)]
struct ConfirmReq {
    token: String,
}

/// No session needed: after an IP change the browser is on a new origin and
/// has no cookie there. The random one-time token proves it is the admin.
async fn net_confirm(State(st): State<Shared>, Json(r): Json<ConfirmReq>) -> ApiResult<Value> {
    let p = {
        let mut g = st.pending.lock().unwrap();
        match g.as_ref() {
            Some(p) if !r.token.is_empty() && p.token == r.token => g.take(),
            _ => None,
        }
    };
    let Some(p) = p else { return Err(ApiError::bad("Mã xác nhận không đúng hoặc đã hết hạn")) };
    blocking(move || {
        if let Some(n) = &p.new_net {
            netcfg::save_net(&st.cfg, n)?;
        }
        if let Some(f) = &p.new_fw {
            netcfg::save_fw(&st.cfg, f)?;
        }
        Ok(())
    })
    .await?;
    crate::info!("{} change confirmed", p.kind);
    Ok(ok())
}

async fn net_revert(_: AuthUser, State(st): State<Shared>) -> ApiResult<Value> {
    revert(st).await;
    Ok(ok())
}

#[derive(Deserialize)]
struct Hostname {
    hostname: String,
}

async fn net_hostname(_: AuthUser, State(st): State<Shared>, Json(h): Json<Hostname>) -> ApiResult<Value> {
    blocking(move || {
        netcfg::validate_hostname(&h.hostname)?;
        privileged::call(&st.cfg, Op::SetHostname { name: h.hostname })?;
        st.shares.lock().unwrap().apply(&st.cfg) // NetBIOS name follows the hostname
    })
    .await?;
    Ok(ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc5987() {
        assert_eq!(super::pct_encode("phim 1.mkv"), "phim%201.mkv");
        assert_eq!(super::pct_encode("ả"), "%E1%BA%A3");
    }
}
