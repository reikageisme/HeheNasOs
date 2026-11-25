# HeheNAS OS

Hệ điều hành NAS siêu nhẹ, phần quản lý viết bằng **Rust**, tối ưu cho **Raspberry Pi 3B (1 GB RAM)**, có thêm bản **ISO x86_64** cho PC. Bạn flash thẻ nhớ, cắm dây mạng, bật nguồn, rồi mở `https://<IP>:8443` (hoặc `http://<IP>:8080`, sẽ tự chuyển sang HTTPS) để quản lý ổ đĩa, file, chia sẻ SMB/NFS, mạng, firewall, cập nhật và sao lưu.

> **Xem giao diện ngay trên Windows/macOS/Linux**, không cần Rust hay Pi:
> ```powershell
> node dev/mock-server.mjs
> ```
> Rồi mở http://localhost:8080 và đăng nhập bằng `admin` / `hehenas123`. Đây là bản giả lập: giao diện thật, API mô phỏng với dữ liệu mẫu, không đụng tới ổ đĩa nào.

## Tính năng

| Nhóm | Có gì |
|---|---|
| Giám sát | CPU, RAM, nhiệt độ, mạng (biểu đồ 10 phút), RAM từng dịch vụ, **cảnh báo** (ổ đầy, SMART lỗi, quá nhiệt, HTTPS/firewall tắt…) |
| Ổ đĩa | Liệt kê ổ USB/SATA/NVMe; mount/tháo; format ext4; **khởi tạo ổ mới** (GPT + ext4); **SMART**; tự kiểm tra lỗi (e2fsck) và tự mount lại sau khi khởi động; "Sửa quyền" cho ổ mang từ máy khác sang |
| Tệp | Trình quản lý file trên web: duyệt, tải lên (có tiến độ), tải xuống, tạo thư mục, đổi tên, xóa vào **thùng rác** (giữ 30 ngày, khôi phục được) |
| Chia sẻ | SMB qua **ksmbd** (SMB3 chạy trong kernel), **quyền đọc/ghi theo từng người dùng**, khách; **NFS** theo dải mạng |
| Mạng | DHCP hoặc **IP tĩnh**; **firewall nftables**. Thay đổi nào có nguy cơ làm mất kết nối sẽ **tự hoàn tác sau 2 phút** nếu không được xác nhận |
| Hệ thống | Dịch vụ, nhật ký lọc theo nguồn, đổi mật khẩu, tắt/khởi động lại, **HTTPS** (chứng chỉ tự ký, hiện vân tay để so), **sao lưu/khôi phục** cấu hình |
| Cập nhật | **A/B**: ghi bản mới vào slot dự phòng, kiểm tra SHA-256; nếu bản mới không chạy ổn định thì **tự quay về bản cũ** |
| Khôi phục | Quên mật khẩu: tạo file `hehenas-reset-admin.txt` (đặt lại admin) hoặc `hehenas-factory-reset.txt` (xóa toàn bộ cấu hình) trên phân vùng FAT của thẻ nhớ |

## Bảo mật

- **Tách quyền**: khởi động bằng root, hehenasd tự tách làm hai tiến trình. Một **helper root** rất nhỏ chỉ nhận một danh sách thao tác cố định (mount, mkfs, ghi cấu hình SMB…). Helper tự kiểm tra lại mọi tham số và tự sinh mọi file cấu hình từ dữ liệu có cấu trúc. **Tiến trình web chạy bằng user `nas`** không có quyền root. Hai bên nói chuyện qua Unix socket mode 0600 và có kiểm tra UID của bên gọi.
- Không bao giờ gọi shell, mọi lệnh đều chạy với tham số đã kiểm tra. Đường dẫn bị khóa trong ổ dữ liệu: chặn `..`, chặn symlink trỏ ra ngoài. Thẻ hệ thống luôn bị chặn thao tác.
- Mật khẩu admin băm bằng **argon2id**. Sau 5 lần đăng nhập sai, IP đó bị **khóa 5 phút**. Session dùng cookie `HttpOnly; SameSite=Strict; Secure`.
- **CSRF**: mọi request thay đổi dữ liệu phải có header `X-HeheNAS: 1`.
- Có các header CSP chặt, `X-Frame-Options: DENY`, `nosniff`, HSTS (khi chạy HTTPS), `no-store` cho API.
- Mount ổ dữ liệu với `nodev,nosuid` (và `noexec` cho FAT/NTFS). SMB: tối thiểu SMB2.1, `restrict anonymous`, ẩn thư mục rác. NFS: `all_squash`, chỉ cho phép theo dải mạng.
- Rootfs **chỉ-đọc**, trạng thái nằm trên phân vùng riêng.
- **Firewall bật sẵn**: chỉ các dải mạng nội bộ (192.168.x.x, 10.x.x.x, 172.16–31.x.x) mới vào được NAS.
- **Không có tài khoản đăng nhập console** (root bị khóa), không SSH, không telnet. Màn hình chỉ hiện địa chỉ web. Muốn có root console để debug thì build với `HEHENAS_ROOT_CONSOLE=1`.
- sysctl cứng hóa: `kptr_restrict`, `dmesg_restrict`, `protected_*`, chặn ICMP redirect và source route, `rp_filter`, SYN cookies, tắt IP forwarding.

## Kiến trúc

```
Trình duyệt ──HTTPS :8443──► hehenasd (user nas): REST API + giao diện nhúng
                                   │ Unix socket, thao tác cố định
                                   ▼
                             helper (root): mount · mkfs · sfdisk · ksmbd · nft · ifup · cập nhật A/B
Linux (Buildroot, BusyBox init, musl) · ksmbd · nfsd · nftables · ext4 · zram
```

| Phân vùng thẻ nhớ (Pi) | Nội dung |
|---|---|
| p1 `boot` FAT32 | firmware, `Image-a/b`, `cmdline-a/b.txt`, `slot.txt`, file khôi phục |
| p2 `rootfs-a` ext4 (chỉ-đọc) | hệ thống, slot A |
| p3 `rootfs-b` ext4 (chỉ-đọc) | hệ thống, slot B (bản cập nhật được ghi vào slot đang không dùng) |
| p4 `persist` ext4 | `/var/lib/hehenas`: tài khoản, chia sẻ, mạng, chứng chỉ TLS… |

Ngân sách RAM ước tính khi rảnh khoảng 50–90 MB: kernel + BusyBox, hehenasd (~6–10 MB cho cả hai tiến trình), ksmbd. Trên Pi, `gpu_mem=16` trả lại khoảng 60 MB RAM, và có 384 MB swap nén bằng zram.

## Cấu trúc mã nguồn

```
core/src/
  main.rs        khởi động, tách quyền (fork), HTTPS + HTTP redirect, tác vụ nền
  privileged.rs  helper root: danh sách thao tác + kiểm tra lại
  api.rs         REST API          web.rs   giao diện, header bảo mật, CSRF
  auth.rs        argon2id, session, khóa theo IP
  storage.rs     ổ đĩa, mount, mkfs, khởi tạo GPT, SMART
  files.rs       trình quản lý file + thùng rác
  shares.rs      SMB (ksmbd) + NFS, quyền theo user
  netcfg.rs      IP tĩnh/DHCP, firewall nftables, tên máy (tự hoàn tác)
  update.rs      cập nhật A/B (tar + SHA-256)
  backup.rs      sao lưu/khôi phục   tls.rs  chứng chỉ, HTTPS listener
  alerts.rs  monitor.rs  network.rs  system.rs  config.rs  util.rs
frontend/        HTML/CSS/JS thuần, nhúng vào binary
dev/mock-server.mjs   server giả lập để xem giao diện (Node, không phụ thuộc)
buildroot-external/
  configs/hehenas_rpi3.fragment     gộp lên raspberrypi3_64_defconfig
  configs/hehenas_x86_64.fragment   gộp lên pc_x86_64_bios_defconfig
  board/common/   post-build, overlay (zram, A/B watchdog, sysctl), kernel options
  board/rpi3/     config.txt, cmdline, genimage A/B, post-image (+ gói cập nhật)
  package/hehenas/
scripts/  build-core.sh · build-image.sh · dev-run.sh · ci-annotate.py
```

## Build

**Cách khuyến nghị: GitHub Actions.** Mỗi lần push lên `main`, workflow sẽ chạy test, clippy, biên dịch chéo, chạy thử daemon (smoke test), rồi build cả hai image. Kết quả nằm trong tab *Actions*, mục *Artifacts*:
- `hehenas-rpi3`: `hehenas-rpi3b-aarch64.img.xz` (để flash) và `hehenas-rpi3b-update-<ver>.tar` (để cập nhật qua web)
- `hehenas-x86_64`: `hehenas-x86_64.iso`

**Build tại máy** (Linux hoặc WSL2, cần khoảng 6 GB RAM và 15 GB ổ đĩa):
```bash
sudo apt install build-essential git rsync bc cpio unzip wget file perl python3 patch xz-utils libncurses-dev gcc-aarch64-linux-gnu
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
BOARD=rpi3 bash scripts/build-image.sh       # hoặc BOARD=x86_64
```

## Cài đặt

**Raspberry Pi 3B**
1. Flash `hehenas-rpi3b-aarch64.img.xz` bằng Raspberry Pi Imager (chọn *Use custom*) hoặc balenaEtcher. **Việc này xóa sạch thẻ nhớ.**
2. Cắm dây mạng và ổ USB, rồi cấp nguồn 5V/2.5A. Ổ HDD 2.5" nên cắm qua hub USB có nguồn riêng.
3. Giống Proxmox, khi boot xong màn hình HDMI (và cổng serial) hiện khung:
   ```
     ==============================================================
       HeheNAS OS  -  hehenas
     ==============================================================
       Open a web browser and connect to:
           https://192.168.1.42:8443/
   ```
   Mở địa chỉ đó, bỏ qua cảnh báo chứng chỉ tự ký, rồi **tạo tài khoản admin ngay**. Khung này tự cập nhật nếu IP thay đổi.

**PC x86_64**: ghi `hehenas-x86_64.iso` ra USB bằng balenaEtcher hoặc Rufus (chế độ DD), rồi boot từ USB. Hệ thống chạy trong RAM. Muốn cấu hình được giữ lại sau khi tắt máy, hãy tạo một phân vùng ext4 có nhãn `hehenas-data` trên một ổ bất kỳ.

## Phát triển

- Chỉ chỉnh giao diện: chạy `node dev/mock-server.mjs`, sửa file trong `frontend/` rồi tải lại trình duyệt.
- Daemon thật trên Linux/WSL ở chế độ dry-run: `bash scripts/dev-run.sh`. Test: `cargo test --manifest-path core/Cargo.toml`.

## Giới hạn đã biết

- ksmbd **không có thùng rác SMB**: file xóa qua Windows hay macOS sẽ mất ngay. Thùng rác chỉ áp dụng cho thao tác xóa trên web.
- Cơ chế A/B trên Pi 3 tự quay về bản cũ khi hệ mới khởi động lên nhưng không chạy ổn định. Nếu **kernel mới bị panic ngay khi boot**, firmware Pi 3 không tự chuyển slot được. Khi đó bạn sửa `slot.txt` trên phân vùng FAT từ máy tính, đổi `-b` thành `-a`.
- Gói cập nhật được kiểm tra bằng SHA-256 nhưng **chưa ký số**. Chỉ admin đã đăng nhập mới tải gói lên được.
- Ethernet trên Pi 3B là 100 Mbit, tối đa khoảng 11 MB/s.
- Những phần cần kiểm chứng trên phần cứng thật trước khi dùng cho dữ liệu quan trọng: tên một số tùy chọn Buildroot (script build sẽ cảnh báo nếu tùy chọn không được bật), cú pháp `ksmbd.adduser` của bản ksmbd-tools đi kèm, và việc boot ISO x86 trên các dòng mainboard khác nhau.

## Cập nhật tự động (OTA)

Từ v1.1.0, HeheNAS tự kiểm tra bản mới trên [GitHub Releases](https://github.com/reikageisme/HeheNasOs/releases) mỗi 6 giờ. Cài đặt nằm ở **Cập nhật → Chế độ tự động**:

| Chế độ | Hoạt động |
|---|---|
| Tự động tải, cài và khởi động lại *(mặc định)* | Tải gói, kiểm tra chữ ký, ghi vào slot dự phòng, rồi khởi động lại vào giờ bạn chọn (mặc định 03:00) |
| Chỉ thông báo | Hiện thông báo khi có bản mới, bạn tự bấm "Cập nhật" |
| Tắt | Không kiểm tra |

An toàn:
- **Mọi gói phải có chữ ký ed25519 hợp lệ.** Khóa công khai được nhúng sẵn trong OS, và helper root sẽ từ chối gói không ký hoặc ký sai, kể cả khi gói đến từ GitHub.
- Tải qua HTTPS có kiểm tra chứng chỉ.
- Đồng hồ được đồng bộ bằng NTP; giờ cũng được lưu lại vì Pi không có pin đồng hồ.
- Nếu bản mới không chạy ổn định sau 2 lần khởi động, máy tự quay về bản cũ (A/B).

### Phát hành bản mới (cho người giữ repo)

1. Tăng `version` trong `core/Cargo.toml`, push lên `main`, rồi đợi workflow **build** chạy xong.
2. Push tag tương ứng: `git tag -a v1.2.0 -m v1.2.0 && git push origin v1.2.0`. Workflow **release** sẽ ký gói cập nhật và đăng lên Releases. Các máy đang chạy sẽ tự cập nhật.
3. Workflow cần secret **`HEHENAS_SIGNING_KEY`** (khóa riêng ed25519 dạng PEM) trong *Settings → Secrets and variables → Actions*. Thiếu secret này thì gói sẽ không có chữ ký và các máy sẽ từ chối cài. **Giữ bản sao khóa ở nơi an toàn**: mất khóa thì các máy chỉ có thể lên bản mới bằng cách flash lại.
