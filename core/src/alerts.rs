//! Health alerts shown on the dashboard. Recomputed in the background every
//! minute (disk listing forks blkid, so never on every page poll).

use std::path::Path;

use serde::Serialize;

use crate::{netcfg, storage, AppState};

#[derive(Serialize, Clone, Debug)]
pub struct Alert {
    /// "crit", "warn" or "info"
    pub level: &'static str,
    pub title: String,
    pub detail: String,
    /// Dashboard page to fix it ("storage", "system"...).
    pub page: &'static str,
}

fn a(level: &'static str, page: &'static str, title: impl Into<String>, detail: impl Into<String>) -> Alert {
    Alert { level, title: title.into(), detail: detail.into(), page }
}

pub fn compute(st: &AppState) -> Vec<Alert> {
    let cfg = &st.cfg;
    let mut out = Vec::new();
    let snap = st.monitor.snapshot();

    if let Some(t) = snap.temp_c {
        if t >= 80.0 {
            out.push(a("crit", "overview", format!("CPU quá nóng: {t:.0} °C"), "CPU đang bị hạ xung. Hãy gắn tản nhiệt hoặc quạt."));
        } else if t >= 72.0 {
            out.push(a("warn", "overview", format!("CPU nóng: {t:.0} °C"), "Gần ngưỡng hạ xung 80 °C."));
        }
    }
    if snap.mem_total > 0 && snap.mem_available * 10 < snap.mem_total {
        out.push(a("warn", "system", "RAM sắp hết", "Còn dưới 10% RAM khả dụng."));
    }

    let disks = storage::list(cfg);
    let mut data_mounted = 0;
    for d in &disks {
        for p in &d.partitions {
            let (Some(mp), Some(total), Some(used)) = (&p.mountpoint, p.total_bytes, p.used_bytes) else { continue };
            if !d.system {
                data_mounted += 1;
            }
            if total == 0 {
                continue;
            }
            let pct = used * 100 / total;
            if pct >= 97 {
                out.push(a("crit", "storage", format!("{mp} gần đầy ({pct}%)"), "Ghi file có thể thất bại. Hãy dọn bớt dữ liệu hoặc thùng rác."));
            } else if pct >= 90 {
                out.push(a("warn", "storage", format!("{mp} đã dùng {pct}%"), "Nên dọn bớt dữ liệu."));
            }
        }
    }
    if data_mounted == 0 {
        out.push(a("info", "storage", "Chưa có ổ dữ liệu", "Cắm ổ USB rồi vào Ổ đĩa để mount hoặc khởi tạo."));
    }

    for s in st.smart.lock().unwrap().values() {
        if s.passed == Some(false) {
            out.push(a("crit", "storage", format!("SMART: ổ {} báo lỗi", s.disk), "Ổ có thể sắp hỏng — hãy sao lưu dữ liệu ngay."));
        }
        let bad = s.reallocated.unwrap_or(0) + s.pending.unwrap_or(0) + s.uncorrectable.unwrap_or(0);
        if bad > 0 {
            out.push(a("warn", "storage", format!("SMART: ổ {} có {bad} sector lỗi", s.disk), "Theo dõi thêm và chuẩn bị thay ổ."));
        }
        if s.temperature_c.unwrap_or(0) >= 55 {
            out.push(a("warn", "storage", format!("Ổ {} nóng: {} °C", s.disk, s.temperature_c.unwrap_or(0)), "Hãy để ổ nơi thoáng."));
        }
    }

    let settings = st.settings.lock().unwrap().clone();
    if !settings.https {
        out.push(a("warn", "system", "HTTPS đang tắt", "Mật khẩu được gửi không mã hóa trong mạng LAN."));
    }
    if !netcfg::load_fw(cfg).enabled {
        out.push(a("info", "network", "Firewall đang tắt", "Bật để chỉ cho phép mạng nội bộ truy cập."));
    }
    if let Some(p) = st.pending.lock().unwrap().as_ref() {
        out.push(a("warn", "network", "Thay đổi mạng đang chờ xác nhận", format!("Sẽ tự hoàn tác nếu chưa xác nhận ({}).", p.kind)));
    }
    if !cfg.dry_run && !storage::is_mountpoint(&cfg.state_dir) && Path::new("/proc/cmdline").exists() {
        out.push(a("warn", "system", "Cấu hình không được lưu bền", format!("{} không phải phân vùng riêng.", cfg.state_dir.display())));
    }
    let up = crate::update::status(cfg);
    if up.trying.is_some() {
        out.push(a("info", "system", "Đang chạy thử bản cập nhật", "Hệ thống sẽ tự xác nhận nếu chạy ổn định."));
    }
    out.sort_by_key(|x| match x.level {
        "crit" => 0,
        "warn" => 1,
        _ => 2,
    });
    out
}
