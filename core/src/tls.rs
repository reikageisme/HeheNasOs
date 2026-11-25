//! HTTPS: self-signed certificate (generated on first start, kept in
//! state_dir/tls) and a TLS listener for `axum::serve`.

use std::{io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use axum::{extract::connect_info::Connected, serve::IncomingStream};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Semaphore},
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};

use crate::{config::Config, util};

/// Runtime settings changed from the UI (state_dir/settings.toml).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub https: bool,
    pub http_redirect: bool,
    /// "off", "notify" (check only) or "install" (download, stage, reboot).
    pub auto_update: String,
    /// Local hour (0-23) at which a staged update reboots the NAS.
    pub update_hour: u8,
    /// Hours east of UTC for update_hour (Vietnam = 7).
    pub tz_offset: i8,
}

impl Default for Settings {
    fn default() -> Self {
        Self { https: true, http_redirect: true, auto_update: "install".into(), update_hour: 3, tz_offset: 7 }
    }
}

impl Settings {
    pub fn load(cfg: &Config) -> Self {
        std::fs::read_to_string(cfg.state_dir.join("settings.toml"))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, cfg: &Config) -> Result<(), String> {
        let s = toml::to_string(self).map_err(|e| e.to_string())?;
        util::write_atomic(&cfg.state_dir.join("settings.toml"), s.as_bytes(), 0o600)
    }
}

fn paths(cfg: &Config) -> (PathBuf, PathBuf) {
    let d = cfg.state_dir.join("tls");
    (d.join("cert.pem"), d.join("key.pem"))
}

/// Create a new self-signed certificate for the current hostname and IPs.
pub fn generate(cfg: &Config) -> Result<(), String> {
    let host = crate::network::hostname();
    let mut names = vec![host.clone(), format!("{host}.local"), "hehenas".into(), "localhost".into()];
    for i in crate::network::info().interfaces {
        names.extend(i.ipv4.iter().filter_map(|a| a.split('/').next()).map(String::from));
    }
    names.sort();
    names.dedup();
    let ck = rcgen::generate_simple_self_signed(names).map_err(|e| format!("tạo chứng chỉ: {e}"))?;
    let (cert, key) = paths(cfg);
    util::write_atomic(&key, ck.key_pair.serialize_pem().as_bytes(), 0o600)?;
    util::write_atomic(&cert, ck.cert.pem().as_bytes(), 0o644)?;
    crate::info!("generated self-signed TLS certificate");
    Ok(())
}

pub fn server_config(cfg: &Config) -> Result<Arc<rustls::ServerConfig>, String> {
    let (cert, key) = paths(cfg);
    if !cert.exists() || !key.exists() {
        generate(cfg)?;
    }
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert)
        .map_err(|e| format!("{}: {e}", cert.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", cert.display()))?;
    let key = PrivateKeyDer::from_pem_file(&key).map_err(|e| format!("{}: {e}", key.display()))?;
    let mut sc = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    sc.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(sc))
}

/// SHA-256 fingerprint of the certificate, to compare with the browser's.
pub fn fingerprint(cfg: &Config) -> Option<String> {
    let (cert, _) = paths(cfg);
    let der = CertificateDer::pem_file_iter(&cert).ok()?.next()?.ok()?;
    let d = Sha256::digest(der.as_ref());
    Some(d.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}

/// Client address for `ConnectInfo`, available on both the plain TCP and the
/// TLS listener (axum only provides `SocketAddr` for `TcpListener`).
#[derive(Clone, Copy, Debug)]
pub struct Peer(pub SocketAddr);

impl Connected<IncomingStream<'_, TlsListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        Peer(*stream.remote_addr())
    }
}

impl Connected<IncomingStream<'_, TcpListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        Peer(*stream.remote_addr())
    }
}

/// TLS listener: handshakes run concurrently (max 32, 10 s each) so one slow
/// client cannot stall the accept loop.
pub struct TlsListener {
    rx: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
}

pub async fn listen(addr: &str, sc: Arc<rustls::ServerConfig>) -> io::Result<TlsListener> {
    let tcp = TcpListener::bind(addr).await?;
    let local = tcp.local_addr()?;
    let (tx, rx) = mpsc::channel(16);
    let acceptor = TlsAcceptor::from(sc);
    let sem = Arc::new(Semaphore::new(32));
    tokio::spawn(async move {
        loop {
            let (stream, peer) = match tcp.accept().await {
                Ok(x) => x,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let Ok(permit) = sem.clone().try_acquire_owned() else { continue };
            let (acceptor, tx) = (acceptor.clone(), tx.clone());
            tokio::spawn(async move {
                let _permit = permit;
                if let Ok(Ok(tls)) = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await {
                    let _ = tx.send((tls, peer)).await;
                }
            });
        }
    });
    Ok(TlsListener { rx, local })
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.rx.recv().await {
            Some(x) => x,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local)
    }
}
