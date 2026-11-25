//! Network information (interfaces, IPv4, gateway, DNS) and hostname changes.

use std::{collections::HashMap, ffi::CStr, fs, net::Ipv4Addr, path::Path};

use serde::Serialize;

use crate::util::read_trim;

#[derive(Serialize)]
pub struct Iface {
    pub name: String,
    pub mac: String,
    pub state: String,
    pub mtu: u32,
    pub speed_mbps: Option<i64>,
    pub ipv4: Vec<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Serialize)]
pub struct NetInfo {
    pub hostname: String,
    pub interfaces: Vec<Iface>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
}

pub fn hostname() -> String {
    read_trim("/proc/sys/kernel/hostname").unwrap_or_else(|| "hehenas".into())
}

pub fn info() -> NetInfo {
    let addrs = ipv4_addrs();
    let mut interfaces = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/net") {
        let mut names: Vec<String> =
            rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names.into_iter().filter(|n| n != "lo") {
            let base = Path::new("/sys/class/net").join(&name);
            let num = |f: &str| read_trim(base.join(f)).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            interfaces.push(Iface {
                mac: read_trim(base.join("address")).unwrap_or_default(),
                state: read_trim(base.join("operstate")).unwrap_or_default(),
                mtu: num("mtu") as u32,
                speed_mbps: read_trim(base.join("speed")).and_then(|s| s.parse().ok()).filter(|s: &i64| *s > 0),
                ipv4: addrs.get(&name).cloned().unwrap_or_default(),
                rx_bytes: num("statistics/rx_bytes"),
                tx_bytes: num("statistics/tx_bytes"),
                name,
            });
        }
    }
    NetInfo { hostname: hostname(), interfaces, gateway: default_gateway(), dns: dns_servers() }
}

fn default_gateway() -> Option<String> {
    let s = fs::read_to_string("/proc/net/route").ok()?;
    s.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() > 2 && f[1] == "00000000" {
            let raw = u32::from_str_radix(f[2], 16).ok()?;
            // The kernel prints the network-order value as a host-order hex word.
            Some(Ipv4Addr::from(u32::from_be(raw)).to_string())
        } else {
            None
        }
    })
}

fn dns_servers() -> Vec<String> {
    fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver").map(|s| s.trim().to_string()))
        .collect()
}

fn ipv4_addrs() -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    // SAFETY: standard getifaddrs/freeifaddrs usage; every pointer is
    // null-checked and the list is only read before it is freed.
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return map;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null() && i32::from((*ifa.ifa_addr).sa_family) == libc::AF_INET {
                let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                let prefix = if ifa.ifa_netmask.is_null() {
                    32
                } else {
                    let m = &*(ifa.ifa_netmask as *const libc::sockaddr_in);
                    u32::from_be(m.sin_addr.s_addr).count_ones()
                };
                let name = CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned();
                map.entry(name).or_default().push(format!("{ip}/{prefix}"));
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    map
}
