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
    /// Resident memory of hehenasd itself.
    pub daemon_rss: u64,
    pub timestamp: u64,
    /// Per-core CPU usage (%).
    pub cores: Vec<f32>,
    /// Per-disk throughput.
    pub disks: Vec<DiskIo>,
}

#[derive(Serialize, Clone, Default)]
pub struct DiskIo {
    pub name: String,
    pub read_bps: u64,
    pub write_bps: u64,
}

/// One sampling interval's measured rates.
struct Sample {
    cpu: f32,
    cores: Vec<f32>,
    rx: u64,
    tx: u64,
    disks: Vec<DiskIo>,
}

#[derive(Serialize, Clone)]
pub struct Point {
    pub t: u64,
    pub cpu: f32,
    pub mem: u64,
    pub rx: u64,
    pub tx: u64,
    /// Total disk read / write bytes per second.
    pub dr: u64,
    pub dw: u64,
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

    fn update(&self, x: Sample) {
        let mut s = Snapshot::default();
        fill(&mut s);
        s.cpu_percent = x.cpu;
        s.net_rx_bps = x.rx;
        s.net_tx_bps = x.tx;
        let (dr, dw) = x.disks.iter().fold((0, 0), |(r, w), d| (r + d.read_bps, w + d.write_bps));
        let p = Point { t: s.timestamp, cpu: x.cpu, mem: s.mem_total.saturating_sub(s.mem_available), rx: x.rx, tx: x.tx, dr, dw };
        s.cores = x.cores;
        s.disks = x.disks;
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
        let mut prev_cores = read_cores();
        let mut prev_net = read_net();
        let mut prev_disks = read_diskstats();
        let mut prev_t = Instant::now();
        let mut iv = tokio::time::interval(INTERVAL);
        iv.tick().await; // first tick fires immediately
        loop {
            iv.tick().await;
            let cpu = read_cpu();
            let cores = read_cores();
            let net = read_net();
            let disks = read_diskstats();
            let now = Instant::now();
            let dt = now.duration_since(prev_t).as_secs_f64().max(0.001);
            let rate = |a: u64, b: u64| (b.saturating_sub(a) as f64 / dt) as u64;
            let disk_io = disks
                .iter()
                .map(|(name, &(r, w))| {
                    let (pr, pw) = prev_disks.get(name).copied().unwrap_or((r, w));
                    DiskIo { name: name.clone(), read_bps: rate(pr, r) * 512, write_bps: rate(pw, w) * 512 }
                })
                .collect();
            st.monitor.update(Sample {
                cpu: cpu_percent(prev_cpu, cpu),
                cores: cores.iter().zip(prev_cores.iter()).map(|(c, p)| cpu_percent(*p, *c)).collect(),
                rx: rate(prev_net.0, net.0),
                tx: rate(prev_net.1, net.1),
                disks: disk_io,
            });
            (prev_cpu, prev_cores, prev_net, prev_disks, prev_t) = (cpu, cores, net, disks, now);
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

/// (total, idle) jiffies for each cpuN line.
fn read_cores() -> Vec<(u64, u64)> {
    fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .map(|l| {
            let v: Vec<u64> = l.split_whitespace().skip(1).take(8).filter_map(|x| x.parse().ok()).collect();
            if v.len() < 5 { (0, 0) } else { (v.iter().sum(), v[3] + v[4]) }
        })
        .collect()
}

/// Whole disks only: name -> (sectors read, sectors written).
fn read_diskstats() -> std::collections::BTreeMap<String, (u64, u64)> {
    parse_diskstats(&fs::read_to_string("/proc/diskstats").unwrap_or_default())
}

fn is_whole_disk(n: &str) -> bool {
    let tail_digit = n.ends_with(|c: char| c.is_ascii_digit());
    (n.starts_with("sd") && !tail_digit)
        || (n.starts_with("mmcblk") && !n.contains('p'))
        || (n.starts_with("nvme") && !n.contains('p'))
        || (n.starts_with("vd") && !tail_digit)
}

fn parse_diskstats(s: &str) -> std::collections::BTreeMap<String, (u64, u64)> {
    s.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let name = *f.get(2)?;
            if !is_whole_disk(name) {
                return None;
            }
            Some((name.to_string(), (f.get(5)?.parse().ok()?, f.get(9)?.parse().ok()?)))
        })
        .collect()
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
    fn diskstats() {
        let d = super::parse_diskstats(
            "   8       0 sda 100 0 2048 10 50 0 4096 5 0 0 0
   8       1 sda1 1 0 8 0 0 0 0 0 0 0 0
 179       0 mmcblk0 5 0 40 0 2 0 16 0 0 0 0
 179       2 mmcblk0p2 1 0 8 0 0 0 0 0 0 0 0
",
        );
        assert_eq!(d.len(), 2);
        assert_eq!(d["sda"], (2048, 4096));
        assert_eq!(d["mmcblk0"], (40, 16));
    }

    #[test]
    fn cpu_math() {
        assert_eq!(super::cpu_percent((100, 50), (200, 100)), 50.0);
        assert_eq!(super::cpu_percent((100, 50), (100, 50)), 0.0);
    }
}
