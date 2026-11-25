//! rustnasd — RustNAS management daemon.
//!
//! One small process that serves the web dashboard and a JSON API, reads
//! system state from /proc and /sys, and drives the few privileged tools a NAS
//! needs (mount, mkfs.ext4, ksmbd). Commands are always executed directly with
//! validated arguments — never through a shell.

mod api;
mod auth;
mod config;
mod monitor;
mod network;
mod shares;
mod storage;
mod system;
mod util;
mod web;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use config::Config;

pub struct AppState {
    pub cfg: Config,
    pub auth: auth::AuthStore,
    pub monitor: monitor::Monitor,
    pub shares: Mutex<shares::ShareStore>,
    /// Serialises disk operations (mount / unmount / format / auto-mount).
    pub ops: Mutex<()>,
}

pub type Shared = Arc<AppState>;

fn main() {
    let mut cfg_path = PathBuf::from("/etc/rustnas/rustnasd.toml");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => match args.next() {
                Some(p) => cfg_path = p.into(),
                None => die("--config needs a path"),
            },
            "-V" | "--version" => {
                println!("rustnasd {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-h" | "--help" => {
                println!("Usage: rustnasd [--config PATH]\nDefault config: /etc/rustnas/rustnasd.toml");
                return;
            }
            other => die(&format!("unknown argument: {other}")),
        }
    }

    let cfg = Config::load(&cfg_path).unwrap_or_else(|e| die(&e));

    // A single-threaded runtime is plenty for a home NAS dashboard and keeps
    // RSS low. Slow work (hashing, mkfs, mount) goes to a small blocking pool.
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

async fn run(cfg: Config) -> Result<(), String> {
    std::fs::create_dir_all(&cfg.state_dir)
        .map_err(|e| format!("{}: {e}", cfg.state_dir.display()))?;
    if cfg.dry_run {
        info!("DRY-RUN mode: privileged commands are logged, not executed");
    }

    let auth = auth::AuthStore::load(&cfg.state_dir, cfg.session_ttl_secs)?;
    let shares = shares::ShareStore::load(&cfg)?;
    let state: Shared = Arc::new(AppState {
        cfg,
        auth,
        monitor: monitor::Monitor::new(),
        shares: Mutex::new(shares),
        ops: Mutex::new(()),
    });

    spawn_restore(state.clone());
    monitor::spawn_sampler(state.clone());
    {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(300)).await;
                st.auth.sweep();
            }
        });
    }

    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind(&state.cfg.listen)
        .await
        .map_err(|e| format!("bind {}: {e}", state.cfg.listen))?;
    info!("RustNAS {} listening on http://{}", env!("CARGO_PKG_VERSION"), state.cfg.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| e.to_string())
}

/// Re-mount remembered data disks and (re)write the SMB config. USB disks can
/// enumerate a few seconds after boot, so this retries a couple of times.
fn spawn_restore(state: Shared) {
    tokio::spawn(async move {
        for (i, delay) in [1u64, 10, 30].into_iter().enumerate() {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            let st = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ops = st.ops.lock().unwrap();
                let restored = storage::restore_mounts(&st.cfg);
                if restored > 0 || i == 0 {
                    if let Err(e) = st.shares.lock().unwrap().apply(&st.cfg) {
                        info!("SMB config: {e}");
                    }
                }
            })
            .await;
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
