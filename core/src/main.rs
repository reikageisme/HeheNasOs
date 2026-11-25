//! rustnasd — RustNAS management daemon.
//!
//! Started as root it forks: the parent stays root as a tiny helper that
//! performs a fixed set of validated operations (privileged.rs); the child
//! drops to the share user and runs the web server (HTTPS + HTTP redirect).

mod alerts;
mod api;
mod auth;
mod backup;
mod config;
mod files;
mod monitor;
mod netcfg;
mod network;
mod privileged;
mod shares;
mod storage;
mod system;
mod tls;
mod update;
mod util;
mod web;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::Extension;
use config::Config;

pub struct AppState {
    pub cfg: Config,
    pub auth: auth::AuthStore,
    pub monitor: monitor::Monitor,
    pub shares: Mutex<shares::ShareStore>,
    /// Serialises disk operations (mount / unmount / format / auto-mount).
    pub ops: Mutex<()>,
    pub settings: Mutex<tls::Settings>,
    pub alerts: Mutex<Vec<alerts::Alert>>,
    pub smart: Mutex<HashMap<String, storage::Smart>>,
    pub pending: Mutex<Option<netcfg::Pending>>,
    pub started: Instant,
}

pub type Shared = Arc<AppState>;

fn usage() -> ! {
    println!(
        "Usage:\n  rustnasd [--config PATH]\n  rustnasd make-update --version V --board B --kernel Image --rootfs rootfs.ext4.gz --out bundle.tar"
    );
    std::process::exit(0)
}

fn main() {
    let mut cfg_path = PathBuf::from("/etc/rustnas/rustnasd.toml");
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("make-update") {
        make_update(&args[1..]);
    }
    while !args.is_empty() {
        let arg = args.remove(0);
        match arg.as_str() {
            "-c" | "--config" if !args.is_empty() => cfg_path = args.remove(0).into(),
            "-V" | "--version" => {
                println!("rustnasd {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => usage(),
        }
    }

    let cfg = Config::load(&cfg_path).unwrap_or_else(|e| die(&e));
    // SAFETY: geteuid has no preconditions.
    let is_root = unsafe { libc::geteuid() } == 0;
    if is_root && !cfg.dry_run {
        if let Err(e) = prepare_state(&cfg) {
            die(&e);
        }
    }

    if is_root && cfg.privsep && !cfg.dry_run {
        let listener = privileged::bind(&cfg).unwrap_or_else(|e| die(&e));
        // Fork before any thread exists (tokio is started in the child only).
        // SAFETY: single-threaded at this point.
        match unsafe { libc::fork() } {
            -1 => die("fork failed"),
            0 => {
                drop(listener);
                privileged::drop_privileges(&cfg).unwrap_or_else(|e| die(&e));
                privileged::use_socket(cfg.helper_socket.clone());
                info!("web server running as uid {} (privilege-separated)", cfg.share_uid);
            }
            child => {
                std::thread::spawn(move || {
                    let mut status = 0;
                    // SAFETY: waiting for our own child.
                    unsafe { libc::waitpid(child, &mut status, 0) };
                    info!("web process exited ({status}); helper exiting too");
                    std::process::exit(1);
                });
                privileged::serve(cfg, listener);
            }
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(4)
        .build()
        .unwrap_or_else(|e| die(&format!("tokio runtime: {e}")));
    if let Err(e) = rt.block_on(run(cfg)) {
        die(&e);
    }
}

fn die(msg: &str) -> ! {
    eprintln!("rustnasd: {msg}");
    std::process::exit(1)
}

fn make_update(args: &[String]) -> ! {
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let (Some(v), Some(b), Some(k), Some(r), Some(o)) =
        (get("--version"), get("--board"), get("--kernel"), get("--rootfs"), get("--out"))
    else {
        usage()
    };
    match update::write_bundle(Path::new(&o), &v, &b, Path::new(&k), Path::new(&r)) {
        Ok(()) => {
            println!("{o}");
            std::process::exit(0)
        }
        Err(e) => die(&e),
    }
}

/// Root-only startup work: recovery flags, default files, ownership.
fn prepare_state(cfg: &Config) -> Result<(), String> {
    let state = &cfg.state_dir;
    std::fs::create_dir_all(state).map_err(|e| format!("{}: {e}", state.display()))?;

    // Recovery: files the user can drop on the FAT boot partition from any PC.
    let factory = cfg.boot_dir.join("rustnas-factory-reset.txt");
    if factory.exists() {
        info!("RECOVERY: factory reset requested");
        if let Ok(rd) = std::fs::read_dir(state) {
            for e in rd.filter_map(Result::ok) {
                let p = e.path();
                let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            }
        }
        let _ = std::fs::remove_file(&factory);
    }
    let reset_admin = cfg.boot_dir.join("rustnas-reset-admin.txt");
    if reset_admin.exists() {
        info!("RECOVERY: admin account reset — open the web UI to create a new one");
        let _ = std::fs::remove_file(state.join("auth.toml"));
        let _ = std::fs::remove_file(&reset_admin);
    }

    // Seed files the read-only rootfs links into /etc.
    let etc = state.join("etc");
    std::fs::create_dir_all(&etc).map_err(|e| e.to_string())?;
    if let Ok(rd) = std::fs::read_dir("/usr/share/rustnas/etc-default") {
        for e in rd.filter_map(Result::ok) {
            let dst = etc.join(e.file_name());
            if !dst.exists() {
                let _ = std::fs::copy(e.path(), dst);
            }
        }
    }
    std::fs::create_dir_all(cfg.smb_conf.parent().unwrap_or(state)).map_err(|e| e.to_string())?;

    // Everything else in state_dir belongs to the unprivileged web process.
    let (uid, gid) = (cfg.share_uid, cfg.share_gid);
    let _ = std::os::unix::fs::chown(state, Some(uid), Some(gid));
    if let Ok(rd) = std::fs::read_dir(state) {
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name();
            if name == "etc" || name == "ksmbd" || name == "lost+found" {
                continue;
            }
            chown_tree(&e.path(), uid, gid);
        }
    }
    Ok(())
}

fn chown_tree(p: &Path, uid: u32, gid: u32) {
    let _ = std::os::unix::fs::lchown(p, Some(uid), Some(gid));
    if p.is_dir() && !p.is_symlink() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.filter_map(Result::ok) {
                chown_tree(&e.path(), uid, gid);
            }
        }
    }
}

async fn run(cfg: Config) -> Result<(), String> {
    std::fs::create_dir_all(&cfg.state_dir).map_err(|e| format!("{}: {e}", cfg.state_dir.display()))?;
    if cfg.dry_run {
        info!("DRY-RUN mode: privileged commands are logged, not executed");
    }
    let auth = auth::AuthStore::load(&cfg.state_dir, cfg.session_ttl_secs)?;
    let shares = shares::ShareStore::load(&cfg)?;
    let settings = tls::Settings::load(&cfg);
    let state: Shared = Arc::new(AppState {
        auth,
        monitor: monitor::Monitor::new(),
        shares: Mutex::new(shares),
        ops: Mutex::new(()),
        settings: Mutex::new(settings.clone()),
        alerts: Mutex::new(Vec::new()),
        smart: Mutex::new(HashMap::new()),
        pending: Mutex::new(None),
        started: Instant::now(),
        cfg,
    });

    spawn_startup(state.clone());
    monitor::spawn_sampler(state.clone());
    spawn_housekeeping(state.clone());

    let app = api::router(state.clone());
    let cfg = &state.cfg;
    let http_app = if settings.https && settings.http_redirect {
        web::redirect_router(Config::port_of(&cfg.https_listen))
    } else {
        app.clone()
    };
    let http = tokio::net::TcpListener::bind(&cfg.listen).await.map_err(|e| format!("bind {}: {e}", cfg.listen))?;
    let http_srv = axum::serve(
        http,
        http_app.layer(Extension(web::Secure(false))).into_make_service_with_connect_info::<tls::Peer>(),
    )
    .with_graceful_shutdown(shutdown_signal());
    info!("RustNAS {} — http://{}", env!("CARGO_PKG_VERSION"), cfg.listen);

    if settings.https {
        let sc = tls::server_config(cfg)?;
        let tls_l = tls::listen(&cfg.https_listen, sc).await.map_err(|e| format!("bind {}: {e}", cfg.https_listen))?;
        info!("HTTPS on https://{}", cfg.https_listen);
        let https_srv = axum::serve(
            tls_l,
            app.layer(Extension(web::Secure(true))).into_make_service_with_connect_info::<tls::Peer>(),
        )
        .with_graceful_shutdown(shutdown_signal());
        let (a, b) = tokio::join!(http_srv, https_srv);
        a.and(b).map_err(|e| e.to_string())
    } else {
        http_srv.await.map_err(|e| e.to_string())
    }
}

/// Mount remembered disks, push SMB/NFS config, firewall, pending network.
/// USB disks can enumerate late, so mounting retries a few times.
fn spawn_startup(state: Shared) {
    tokio::spawn(async move {
        for (i, delay) in [1u64, 10, 30].into_iter().enumerate() {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            let st = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ops = st.ops.lock().unwrap();
                let restored = storage::restore_mounts(&st.cfg);
                if restored > 0 || i == 0 {
                    if let Err(e) = st.shares.lock().unwrap().apply(&st.cfg) {
                        info!("share config: {e}");
                    }
                }
                if i == 0 {
                    let fw = netcfg::load_fw(&st.cfg);
                    if let Err(e) = privileged::call(&st.cfg, privileged::Op::ApplyFirewall { fw }) {
                        info!("firewall: {e}");
                    }
                    let marker = st.cfg.state_dir.join(".apply-network");
                    if marker.exists() {
                        let net = netcfg::load_net(&st.cfg);
                        let _ = privileged::call(&st.cfg, privileged::Op::ApplyNetwork { net });
                        let _ = std::fs::remove_file(marker);
                    }
                }
            })
            .await;
        }
    });
}

/// Alerts every minute, SMART every 30 min, session sweep, trash expiry,
/// and confirming a freshly installed update once we have been healthy.
fn spawn_housekeeping(state: Shared) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(if tick == 0 { 5 } else { 60 })).await;
            let st = state.clone();
            let t = tick;
            let _ = tokio::task::spawn_blocking(move || {
                if t % 30 == 0 {
                    let disks = storage::list(&st.cfg);
                    for d in disks.iter().filter(|d| !d.name.starts_with("mmcblk")) {
                        let s = storage::smart(&st.cfg, &d.name);
                        st.smart.lock().unwrap().insert(d.name.clone(), s);
                    }
                    let names: Vec<String> = disks.into_iter().map(|d| d.name).collect();
                    st.smart.lock().unwrap().retain(|k, _| names.contains(k));
                }
                let list = alerts::compute(&st);
                *st.alerts.lock().unwrap() = list;
                if t % 5 == 0 {
                    st.auth.sweep();
                }
                if t % 1440 == 60 {
                    let n = files::trash_expire(&st.cfg);
                    if n > 0 {
                        info!("trash: removed {n} items older than 30 days");
                    }
                }
                if t == 2 && st.started.elapsed() >= Duration::from_secs(90) {
                    let _ = privileged::call(&st.cfg, privileged::Op::UpdateConfirm);
                }
            })
            .await;
            tick += 1;
        }
    });
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    info!("shutting down");
}
