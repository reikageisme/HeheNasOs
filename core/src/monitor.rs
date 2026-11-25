//! System monitor: samples /proc every few seconds and keeps a short history
//! (about 10 minutes) for the dashboard charts. Pure file reads, no forks.

use std::{
    collections::{HashMap, VecDeque},
    fs,
    sync::RwLock,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    util::{now_secs, read_trim},
    Shared,
};

const HISTORY: usize = 200;
const INTERVAL: Duration = Duration::from_secs(3);

#[derive(Serialize, Clone, Default)]
pub struct Snapshot {
    pub hostname: String,
    pub model: String,
    pub kernel: String,
    pub version: &'static str,
    pub cpu_percent: f32,
    pub cpu_cores: usize,
    pub load: [f32; 3],
    pub mem_total: u64,
    pub mem_available: u64,
    pub mem_cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub temp_c: Option<f32>,
    pub uptime_secs: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    /// Resident memory of rustnasd itself.
    pub daemon_rss: u64,
    pub timestamp: u64,
}

#[derive(Serialize, Clone)]
pub struct Point {
    pub t: u64,
    pub cpu: f32,
    pub mem: u64,
    pub rx: u64,
    pub tx: u64,
}

pub struct Monitor {
    inner: RwLock<(Snapshot, VecDeque<Point>)>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    pub fn new() -> Self {
        let mut s = Snapshot::default();
        fill(&mut s);
        Self { inner: RwLock::new((s, VecDeque::with_capacity(HISTORY))) }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.read().unwrap().0.clone()
    }

    pub fn history(&self) -> Vec<Point> {
        self.inner.read().unwrap().1.iter().cloned().collect()
    }

    fn update(&self, cpu: f32, rx: u64, tx: u64) {
        let mut s = Snapshot::default();
        fill(&mut s);
        s.cpu_percent = cpu;
        s.net_rx_bps = rx;
        s.net_tx_bps = tx;
        let p = Point { t: s.timestamp, cpu, mem: s.mem_total.saturating_sub(s.mem_available), rx, tx };
        let mut g = self.inner.write().unwrap();
        if g.1.len() >= HISTORY {
            g.1.pop_front();
        }
        g.1.push_back(p);
        g.0 = s;
    }
}

pub fn spawn_sampler(st: Shared) {
    tokio::spawn(async move {
        let mut prev_cpu = read_cpu();
        let mut prev_net = read_net();
        let mut prev_t = Instant::now();
        let mut iv = tokio::time::interval(INTERVAL);
        iv.tick().await; // first tick fires immediately
        loop {
            iv.tick().await;
            let cpu = read_cpu();
            let net = read_net();
            let now = Instant::now();
            let dt = now.duration_since(prev_t).as_secs_f64().max(0.001);
            let rx = (net.0.saturating_sub(prev_net.0) as f64 / dt) as u64;
            let tx = (net.1.saturating_sub(prev_net.1) as f64 / dt) as u64;
            st.monitor.update(cpu_percent(prev_cpu, cpu), rx, tx);
            (prev_cpu, prev_net, prev_t) = (cpu, net, now);
        }
    });
}

fn fill(s: &mut Snapshot) {
    s.hostname = crate::network::hostname();
    s.model = read_trim("/proc/device-tree/model")
        .or_else(|| read_trim("/sys/devices/virtual/dmi/id/product_name"))
        .unwrap_or_default();
    s.kernel = read_trim("/proc/sys/kernel/osrelease").unwrap_or_default();
    s.version = env!("CARGO_PKG_VERSION");
    s.cpu_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);

    let m = meminfo();
    let kb = |k: &str| m.get(k).copied().unwrap_or(0) * 1024;
    s.mem_total = kb("MemTotal");
    s.mem_available = kb("MemAvailable");
    s.mem_cached = kb("Cached") + kb("Buffers");
    s.swap_total = kb("SwapTotal");
    s.swap_free = kb("SwapFree");

    if let Some(l) = read_trim("/proc/loadavg") {
        for (i, v) in l.split_whitespace().take(3).enumerate() {
            s.load[i] = v.parse().unwrap_or(0.0);
        }
    }
    s.uptime_secs = read_trim("/proc/uptime")
        .and_then(|u| u.split_whitespace().next()?.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0);
    s.temp_c = read_trim("/sys/class/thermal/thermal_zone0/temp")
        .and_then(|t| t.parse::<f32>().ok())
        .map(|t| t / 1000.0);
    s.daemon_rss = crate::system::rss("self");
    s.timestamp = now_secs();
}

fn meminfo() -> HashMap<String, u64> {
    fs::read_to_string("/proc/meminfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.to_string(), v.split_whitespace().next()?.parse().ok()?))
        })
        .collect()
}

/// (total jiffies, idle jiffies) from the aggregate "cpu" line.
fn read_cpu() -> (u64, u64) {
    let s = fs::read_to_string("/proc/stat").unwrap_or_default();
    let v: Vec<u64> = s
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 5 {
        return (0, 0);
    }
    (v.iter().sum(), v[3] + v[4])
}

fn cpu_percent(a: (u64, u64), b: (u64, u64)) -> f32 {
    let total = b.0.saturating_sub(a.0);
    let idle = b.1.saturating_sub(a.1);
    if total == 0 {
        return 0.0;
    }
    ((1.0 - idle as f32 / total as f32) * 100.0).clamp(0.0, 100.0)
}

/// Total (rx, tx) bytes over all interfaces except loopback.
fn read_net() -> (u64, u64) {
    let s = fs::read_to_string("/proc/net/dev").unwrap_or_default();
    let mut rx = 0;
    let mut tx = 0;
    for line in s.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        if name.trim() == "lo" {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if f.len() >= 9 {
            rx += f[0];
            tx += f[8];
        }
    }
    (rx, tx)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cpu_math() {
        assert_eq!(super::cpu_percent((100, 50), (200, 100)), 50.0);
        assert_eq!(super::cpu_percent((100, 50), (100, 50)), 0.0);
    }
}
