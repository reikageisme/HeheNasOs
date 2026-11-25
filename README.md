# RustNAS OS

Hệ điều hành NAS siêu nhẹ cho **Raspberry Pi 3B (1 GB RAM)**. Bạn flash thẻ nhớ, cắm dây mạng, bật nguồn, rồi mở `http://<IP>:8080` để quản lý ổ đĩa, thư mục chia sẻ SMB, người dùng, mạng và dịch vụ. Cách dùng giống Proxmox/OpenMediaVault nhưng tốn ít RAM hơn nhiều.

- Nền: Linux tối giản dựng bằng **Buildroot** (BusyBox init, musl).
- Phần quản lý: một daemon **Rust** duy nhất là `rustnasd` (Axum, Tokio chạy 1 luồng). Daemon phục vụ cả REST API lẫn giao diện web nhúng sẵn trong file thực thi.
- Chia sẻ file: **ksmbd**, server SMB3 chạy trong kernel.
- Trạng thái hiện tại: **v0.1 alpha**. Code đã đủ phần chính, nhưng **chưa được build và chạy thử trên Pi thật** (xem mục [Cần kiểm chứng](#cần-kiểm-chứng-trên-phần-cứng-thật)).

## Hướng đã chọn: NAS chuyên dụng

Bản này tập trung làm NAS cho Pi 3B. Không có Docker, VM, ZFS hay RAID5/6. Danh sách dịch vụ trong `system.rs` và cấu trúc module vẫn để chỗ cho việc mở rộng thành server đa năng ở các bản sau.

## Thay đổi so với tài liệu nghiên cứu ban đầu

| Tài liệu đề xuất | Bản này dùng | Lý do |
|---|---|---|
| File `.iso` | `rustnas-rpi3b-aarch64.img.xz` | Pi 3B khởi động bằng firmware trên phân vùng FAT, không boot được ISO kiểu PC. Bản ISO x86_64 để dành cho sau này. |
| Samba (25–65 MB RAM) | **ksmbd** (vài MB) | SMB3 chạy trong kernel, cấu hình cùng cú pháp `smb.conf`. Trên máy 1 GB RAM thì đây là khoản tiết kiệm lớn nhất. |
| Biên dịch Rust trong Buildroot | Biên dịch riêng thành **binary tĩnh musl** | Không phụ thuộc toolchain hay libc của Buildroot. Chỉ cần `rustup`, việc link do `rust-lld` lo. |
| `gpu_mem` mặc định (~76–100 MB) | `gpu_mem=16` + firmware rút gọn | NAS không có màn hình nên trả khoảng 60–80 MB lại cho hệ thống. |
| (không có) | **zram swap** 384 MB | Có thêm khoảng trống khi tải tăng đột ngột mà không làm mòn thẻ SD. |
| (không có) | Phân vùng `persist` riêng (p3) | Tài khoản, cấu hình share và DB mật khẩu SMB nằm tách khỏi rootfs, sau này dễ làm rootfs chỉ-đọc hoặc cập nhật A/B. |
| Script init | `rustnasd` chạy dưới `respawn` của init | Daemon tự khởi động lại nếu bị crash, đáp ứng yêu cầu chạy 24/7. |

## Ngân sách RAM dự kiến (chưa đo trên máy thật)

| Thành phần | Ước tính |
|---|---|
| Kernel + BusyBox + udhcpc + syslogd | 40–70 MB |
| rustnasd (khi rảnh) | 3–8 MB |
| ksmbd + ksmbd.mountd | 3–10 MB |
| Tổng khi không có tải | **~50–90 MB** |

Ngân sách trong tài liệu gốc là 130–270 MB. RAM còn trống được dùng làm page cache cho việc truyền file. Trang **Tổng quan** và **Hệ thống** hiển thị RAM thực tế của từng dịch vụ, nên bạn có thể đo trực tiếp ngay trên Pi.

## Cấu trúc thư mục

```
core/                     daemon Rust (rustnasd)
  src/main.rs             khởi động, runtime, tự mount lại ổ
  src/api.rs              các endpoint REST
  src/auth.rs             tài khoản admin (argon2id), session cookie
  src/monitor.rs          CPU/RAM/nhiệt độ/mạng từ /proc, lịch sử 10 phút
  src/storage.rs          liệt kê ổ (/sys/block), mount/unmount/format ext4
  src/shares.rs           chia sẻ SMB + người dùng, sinh ksmbd.conf
  src/network.rs          card mạng, gateway, DNS, đổi tên máy
  src/system.rs           dịch vụ, log, RAM theo tiến trình
  src/web.rs              phục vụ giao diện web nhúng
frontend/                 giao diện web: HTML/CSS/JS thuần, không cần bước build
buildroot-external/       cây BR2_EXTERNAL
  configs/rustnas_rpi3.fragment   gộp lên raspberrypi3_64_defconfig
  board/rpi3/             config.txt, cmdline, kernel fragment, genimage, hook
  board/rpi3/rootfs-overlay/      zram, sysctl, hiện IP lên màn hình
  package/rustnas/        đóng gói rustnasd + init script ksmbd
scripts/
  build-core.sh           biên dịch chéo rustnasd cho aarch64
  build-image.sh          build toàn bộ image thẻ nhớ
  dev-run.sh              chạy daemon trên PC (dry-run) để làm giao diện
dev/rustnasd.dev.toml     cấu hình cho môi trường phát triển
```

## Build image (Windows dùng WSL2)

1. **Tăng RAM cho WSL.** Hiện WSL Ubuntu trên máy này chỉ có khoảng 400 MB, không đủ để build GCC. Tạo file `%UserProfile%\.wslconfig` với nội dung:

   ```ini
   [wsl2]
   memory=6GB
   processors=4
   ```

   Rồi chạy lệnh:

   ```powershell
   wsl --shutdown
   ```

2. **Cài công cụ trong Ubuntu:**

   ```bash
   sudo apt install build-essential git rsync bc cpio unzip wget file perl python3 patch xz-utils libncurses-dev
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. **Build** (lần đầu mất 1–3 giờ và khoảng 15 GB ổ đĩa, vì Buildroot tự dựng toolchain, kernel và firmware):

   ```bash
   cd /mnt/d/HeheNasOs
   bash scripts/build-image.sh
   ```

   Script sẽ chép mã nguồn sang `~/rustnas-build` (ổ ext4 của WSL nên nhanh hơn `/mnt/d`), biên dịch `rustnasd`, tải Buildroot `2025.02.x` (bản LTS), gộp cấu hình và build. Kết quả nằm ở `dist/rustnas-rpi3b-aarch64.img.xz`.

   Muốn chỉnh tùy chọn thì chạy `bash scripts/build-image.sh menuconfig` hoặc `... linux-menuconfig`.

## Cài lên Pi 3B

1. Flash file `.img.xz` bằng Raspberry Pi Imager (chọn *Use custom*) hoặc balenaEtcher. Thẻ 4 GB trở lên là đủ.
2. Cắm dây mạng và ổ USB, rồi cấp nguồn 5V/2.5A. Ổ HDD 2.5" nên cắm qua hub USB có nguồn riêng.
3. Sau khoảng 20–40 giây, địa chỉ web hiện lên màn hình HDMI hoặc cổng serial. Bạn cũng có thể xem IP trong trang quản lý của router.
4. Mở `http://<IP>:8080` và **tạo tài khoản quản trị ngay**. Người đầu tiên mở trang sẽ thành quản trị viên.
5. Vào **Ổ đĩa** để Mount (hoặc Format sang ext4) ổ USB. Vào **Chia sẻ** để tạo người dùng SMB và thư mục chia sẻ. Trên Windows, mở `\\<IP>` để kết nối.

## Phát triển giao diện trên PC

Trong WSL hoặc Linux, chạy:

```bash
bash scripts/dev-run.sh      # http://localhost:8080
```

Ở chế độ `dry_run`, các lệnh mount, mkfs và ksmbd chỉ được ghi log chứ không thực thi. Giao diện được đọc thẳng từ `frontend/`, nên sửa file xong chỉ cần tải lại trình duyệt. Muốn chạy test thì dùng `cargo test --manifest-path core/Cargo.toml`.

## REST API

Mọi endpoint đều yêu cầu đăng nhập, trừ `/api/auth/status|setup|login|logout`. Session lưu trong cookie `HttpOnly; SameSite=Strict`.

| Phương thức | Đường dẫn | Chức năng |
|---|---|---|
| GET | `/api/auth/status` | cần setup lần đầu không, ai đang đăng nhập |
| POST | `/api/auth/setup` · `/login` · `/logout` · `/password` | xác thực |
| GET | `/api/system/overview` · `/history` | CPU, RAM, nhiệt độ, mạng, lịch sử |
| GET/POST | `/api/system/services` · `/services/{id}/{start\|stop\|restart}` | dịch vụ |
| GET | `/api/system/logs` | 300 dòng log cuối |
| POST | `/api/system/power` `{action: reboot\|poweroff}` | nguồn |
| GET | `/api/storage/disks` | ổ đĩa và phân vùng |
| POST | `/api/storage/mount` · `/unmount` `{device}` | mount và tháo ổ |
| POST | `/api/storage/format` `{device, label, confirm}` | format ext4 (`confirm` phải trùng tên thiết bị) |
| GET/POST | `/api/shares` · DELETE `/api/shares/{name}` | thư mục chia sẻ |
| POST | `/api/smb-users` · DELETE `/api/smb-users/{name}` | người dùng SMB |
| GET | `/api/network` · POST `/api/network/hostname` | mạng |

## Bảo mật

- Daemon chạy bằng quyền root nhưng **không bao giờ gọi shell**. Mọi lệnh đều được chạy trực tiếp với tham số đã kiểm tra: tên thiết bị chỉ gồm `[a-z0-9]`, đường dẫn share phải nằm trong `/mnt/<ổ đã mount>`, không chấp nhận `..`. Thẻ hệ thống luôn bị chặn mount và format.
- Mật khẩu admin được băm bằng argon2id. Đăng nhập sai sẽ bị chờ 1,5 giây. Trang có CSP chặt, `frame-ancestors 'none'` và `nosniff`.
- Bản v0.1 chạy HTTP thuần, **chỉ nên dùng trong mạng LAN**. HTTPS, tách quyền root ra helper riêng và firewall nằm trong kế hoạch v0.5.
- Tài khoản `root` trên console chưa có mật khẩu (Buildroot mặc định) để tiện debug qua HDMI hoặc serial. Hãy đặt `BR2_TARGET_GENERIC_ROOT_PASSWD` trước khi dùng thật. Image không có SSH.

## Cần kiểm chứng trên phần cứng thật

Code đã viết xong và kiểm tra logic, nhưng những điểm sau phụ thuộc vào phiên bản Buildroot và phần cứng nên cần thử thật:

- Tên tùy chọn `BR2_PACKAGE_RPI_FIRMWARE_CD` trong bản Buildroot đang dùng. Script build sẽ cảnh báo nếu tùy chọn này không được bật, và `post-image.sh` tự chuyển sang `gpu_mem=64`.
- Cú pháp `ksmbd.adduser -a/-u/-d USER -p PASS` và `ksmbd.control -r` của bản ksmbd-tools đi kèm. Nếu `ksmbd.control -r` không chạy, script tự gửi `SIGHUP` thay thế.
- Tốc độ SMB thực tế qua cổng Ethernet 100 Mbit (lý thuyết tối đa ~11 MB/s) và mức RAM thực tế.
- Việc tự mount lại ổ USB sau khi khởi động lại hoặc mất điện.

## Lộ trình

| Bản | Nội dung |
|---|---|
| **v0.1** (bản này) | image boot, DHCP, web UI, đăng nhập, giám sát, ổ đĩa, SMB qua ksmbd |
| v0.2 | khởi tạo ổ mới (tạo bảng phân vùng), SMART, cảnh báo ổ đầy |
| v0.3 | NFS (tùy chọn), quyền theo thư mục, thùng rác SMB |
| v0.4 | trình quản lý file và upload/download qua web, xem log theo dịch vụ |
| v0.5 | HTTPS, IP tĩnh, firewall, sao lưu cấu hình, helper tách quyền root |
| v1.0 | rootfs chỉ-đọc + cập nhật A/B, khôi phục, thử mất điện, image x86_64 `.iso` |
