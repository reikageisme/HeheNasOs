## HeheNAS OS @VERSION@

NAS OS siêu nhẹ cho **Raspberry Pi 3B (1 GB RAM)**, phần quản lý viết bằng Rust, quản trị qua web giống Proxmox.

### Tải file nào?

| File | Dùng cho |
|---|---|
| **`hehenas-rpi3b-aarch64.img.xz`** | **Raspberry Pi 3B: flash vào thẻ nhớ** |
| `hehenas-rpi3b-update-*.tar` | Cập nhật một máy đang chạy HeheNAS (Hệ thống → Cập nhật) |
| `hehenas-x86_64.iso` | PC x86_64: ghi ra USB (balenaEtcher hoặc Rufus chế độ DD) và boot (chạy trong RAM) |
| `hehenasd-aarch64`, `hehenasd-x86_64` | Chỉ binary daemon (dành cho dev) |
| `SHA256SUMS` | Kiểm tra file tải về có nguyên vẹn không |

### Cài lên Raspberry Pi 3B

1. Mở **Raspberry Pi Imager** → *Choose OS* → **Use custom** → chọn `hehenas-rpi3b-aarch64.img.xz` (không cần giải nén).
2. Chọn thẻ nhớ → nếu được hỏi về *OS customisation* thì bấm **No** → *Write*. ⚠️ Toàn bộ dữ liệu trên thẻ sẽ bị xóa.
3. Cắm thẻ vào Pi, cắm **dây mạng**, rồi cấp nguồn 5V/2.5A.
4. Khoảng 30–60 giây sau, màn hình HDMI hiện:
   ```
   Open a web browser and connect to:
       https://<IP>:8443/
   ```
   Không có màn hình thì xem IP của máy "hehenas" trong trang quản lý router.
5. Mở địa chỉ đó, bỏ qua cảnh báo chứng chỉ (chứng chỉ tự ký, tạo riêng cho máy của bạn), rồi **tạo tài khoản admin ngay**.

### Lưu ý

- Ổ HDD 2.5" nên cắm qua hub USB có nguồn riêng. Dữ liệu NAS nằm trên ổ USB, thẻ nhớ chỉ chứa hệ điều hành.
- Console không cho đăng nhập: mọi thao tác quản trị làm qua web. **Quên mật khẩu**: cắm thẻ nhớ vào máy tính, tạo file trống `hehenas-reset-admin.txt` trên phân vùng `boot`, rồi cắm lại vào Pi.
- Firewall bật sẵn, chỉ mạng nội bộ (192.168.x.x, 10.x.x.x, 172.16–31.x.x) mới truy cập được.
- Đây là bản đầu tiên chạy trên phần cứng thật. Hãy thử với dữ liệu không quan trọng trước, và báo lỗi qua tab *Issues* nếu gặp vấn đề.

<sub>Build từ commit @SHA@, lượt CI #@RUN@.</sub>
