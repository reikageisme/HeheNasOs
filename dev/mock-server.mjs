// RustNAS UI preview: serves ../frontend and a simulated API with demo data.
// Runs on any OS with Node 18+, needs no packages, touches no real disks.
//   node dev/mock-server.mjs            -> http://localhost:8080  (admin / rustnas123)
//   PORT=9000 node dev/mock-server.mjs
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'frontend');
const PORT = Number(process.env.PORT || 8080);
const now = () => Math.floor(Date.now() / 1000);
const GB = 1024 ** 3;

// ---------------------------------------------------------------- demo state
const S = {
  admin: { user: 'admin', pass: 'rustnas123' },
  sessions: new Set(),
  hostname: 'rustnas',
  started: now() - 3 * 86400 - 5 * 3600,
  history: [],
  settings: { https: true, http_redirect: true },
  shares: {
    workgroup: 'WORKGROUP', nfs_enabled: false, users: ['anh', 'em'],
    list: [
      { name: 'media', path: '/mnt/data/Phim', comment: 'Phim và nhạc', read_only: true, guest_ok: false, users: [{ name: 'anh', access: 'rw' }, { name: 'em', access: 'ro' }], nfs: true, nfs_clients: '192.168.1.0/24' },
      { name: 'docs', path: '/mnt/data/docs', comment: '', read_only: false, guest_ok: false, users: [], nfs: false, nfs_clients: '' },
    ],
  },
  net: { mode: 'dhcp', address: '', gateway: '', dns: [] },
  fw: { enabled: true, allowed: ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16'], smb: true, nfs: true },
  pending: null,
  disks: [
    { name: 'mmcblk0', model: 'SD SC32G', size: 32 * GB, transport: 'sd', rotational: false, system: true, parts: [
      { name: 'mmcblk0p1', fstype: 'vfat', label: 'boot', size: 64 * 2 ** 20, mountpoint: '/boot', total: 63 * 2 ** 20, used: 31 * 2 ** 20 },
      { name: 'mmcblk0p2', fstype: 'ext4', label: 'rootfs', size: 256 * 2 ** 20, mountpoint: '/', total: 240 * 2 ** 20, used: 141 * 2 ** 20 },
      { name: 'mmcblk0p3', fstype: 'ext4', label: 'rootfs', size: 256 * 2 ** 20 },
      { name: 'mmcblk0p4', fstype: 'ext4', label: 'rustnas-data', size: 128 * 2 ** 20, mountpoint: '/var/lib/rustnas', total: 120 * 2 ** 20, used: 3 * 2 ** 20 },
    ] },
    { name: 'sda', model: 'Samsung SSD 870 EVO', size: 500 * GB, transport: 'usb', rotational: false, system: false, parts: [
      { name: 'sda1', fstype: 'ext4', label: 'data', uuid: 'a1', size: 500 * GB, mountpoint: '/mnt/data', total: 492 * GB, used: 161 * GB },
    ] },
    { name: 'sdb', model: 'WD Elements 2TB', size: 2000 * GB, transport: 'usb', rotational: true, system: false, parts: [
      { name: 'sdb1', fstype: 'ntfs', label: 'Backup', uuid: 'b1', size: 2000 * GB },
    ] },
    { name: 'sdc', model: 'Generic Flash Disk', size: 64 * GB, transport: 'usb', rotational: false, system: false, parts: [] },
  ],
  smart: {
    sda: { disk: 'sda', available: true, passed: true, temperature_c: 34, power_on_hours: 6120, reallocated: 0, pending: 0, uncorrectable: 0, model: 'Samsung SSD 870 EVO 500GB', serial: 'S6PXNM0T' },
    sdb: { disk: 'sdb', available: true, passed: true, temperature_c: 41, power_on_hours: 21877, reallocated: 8, pending: 0, uncorrectable: 0, model: 'WDC WD20SDRW', serial: 'WX12A' },
    sdc: { disk: 'sdc', available: false, message: 'Ổ không hỗ trợ SMART (hoặc hộp USB không chuyển tiếp lệnh)' },
  },
  trash: [],
  update: { slots: { a: '1.0.0', b: '0.9.2' }, current_slot: 'a', trying: null },
};

// Virtual file tree for /mnt/data
const D = (children) => ({ dir: true, mtime: now() - 86400 * 3, children });
const F = (size, ago = 3600) => ({ dir: false, size, mtime: now() - ago });
const TREE = { '/mnt/data': D({
  Phim: D({ 'Dune Part Two (2024).mkv': F(14.2 * GB, 86400 * 20), 'Spirited Away.mkv': F(4.1 * GB, 86400 * 90), Series: D({ 'S01E01.mkv': F(1.3 * GB) }) }),
  'Ảnh': D({ '2025': D({ 'Đà Lạt': D({ 'IMG_0001.jpg': F(4.2e6), 'IMG_0002.jpg': F(3.9e6) }) }), 'avatar.png': F(220e3) }),
  docs: D({ 'Hợp đồng thuê nhà.pdf': F(812e3, 86400 * 4), 'Ngân sách 2026.xlsx': F(54e3, 7200), 'ghi-chu.txt': F(1200, 60) }),
  'Phần mềm': D({ 'ubuntu-24.04-server.iso': F(2.6 * GB, 86400 * 60) }),
  'readme.txt': F(512, 86400),
}) };

function nodeAt(p) {
  if (!p.startsWith('/mnt/data')) return null;
  let n = TREE['/mnt/data'];
  for (const seg of p.slice('/mnt/data'.length).split('/').filter(Boolean)) {
    if (!n.dir || !n.children[seg]) return null;
    n = n.children[seg];
  }
  return n;
}

function snapshot() {
  const t = Date.now() / 1000;
  const cpu = Math.max(1, 6 + 5 * Math.sin(t / 17) + Math.random() * 6 + (Math.random() < 0.06 ? 35 : 0));
  const rx = Math.max(0, 1.5e6 + 1.2e6 * Math.sin(t / 23) + Math.random() * 4e5);
  const tx = Math.max(0, 3.2e5 + 2e5 * Math.cos(t / 19) + Math.random() * 1e5);
  const total = 948 * 2 ** 20;
  const avail = total - (212 * 2 ** 20 + Math.random() * 8 * 2 ** 20);
  return {
    hostname: S.hostname, model: 'Raspberry Pi 3 Model B Rev 1.2', kernel: '6.12.25-v8', version: '1.0.0',
    cpu_percent: cpu, cpu_cores: 4, load: [0.18 + cpu / 200, 0.22, 0.19],
    mem_total: total, mem_available: avail, mem_cached: 486 * 2 ** 20, swap_total: 384 * 2 ** 20, swap_free: 380 * 2 ** 20,
    temp_c: 51 + 3 * Math.sin(t / 40) + cpu / 20, uptime_secs: now() - S.started,
    net_rx_bps: rx, net_tx_bps: tx, daemon_rss: 6.4 * 2 ** 20, timestamp: now(),
  };
}
setInterval(() => {
  const s = snapshot();
  S.history.push({ t: s.timestamp, cpu: s.cpu_percent, mem: s.mem_total - s.mem_available, rx: s.net_rx_bps, tx: s.net_tx_bps });
  if (S.history.length > 200) S.history.shift();
}, 3000);
for (let i = 0; i < 60; i++) S.history.push({ t: now() - (60 - i) * 3, cpu: 5 + Math.random() * 10, mem: 215e6, rx: 1e6 + Math.random() * 1e6, tx: 3e5 });

function disksJson() {
  return S.disks.map((d) => ({
    name: d.name, dev: `/dev/${d.name}`, model: d.model, size_bytes: d.size, transport: d.transport,
    rotational: d.rotational, removable: false, system: d.system,
    partitions: d.parts.map((p) => ({
      name: p.name, dev: `/dev/${p.name}`, size_bytes: p.size, fstype: p.fstype || null, label: p.label || null,
      uuid: p.uuid || null, mountpoint: p.mountpoint || null,
      total_bytes: p.mountpoint ? (p.total ?? p.size * 0.98) : null, used_bytes: p.mountpoint ? (p.used ?? 0) : null,
    })),
  }));
}
const findPart = (name) => {
  for (const d of S.disks) { const p = d.parts.find((x) => x.name === name); if (p) return [d, p]; }
  return [null, null];
};

function alerts() {
  const a = [];
  const s = S.smart.sdb;
  if (s.reallocated) a.push({ level: 'warn', title: `SMART: ổ sdb có ${s.reallocated} sector lỗi`, detail: 'Theo dõi thêm và chuẩn bị thay ổ.', page: 'storage' });
  if (!S.fw.enabled) a.push({ level: 'info', title: 'Firewall đang tắt', detail: 'Bật để chỉ cho phép mạng nội bộ truy cập.', page: 'network' });
  if (!S.settings.https) a.push({ level: 'warn', title: 'HTTPS đang tắt', detail: 'Mật khẩu được gửi không mã hóa trong mạng LAN.', page: 'system' });
  if (S.pending) a.push({ level: 'warn', title: 'Thay đổi mạng đang chờ xác nhận', detail: 'Sẽ tự hoàn tác nếu chưa xác nhận.', page: 'network' });
  return a;
}

const LOGS = [
  'Nov 25 08:00:01 rustnas syslog.info syslogd started: BusyBox v1.37.0',
  'Nov 25 08:00:02 rustnas kern.info kernel: usb 1-1.2: new high-speed USB device number 4 using dwc_otg',
  'Nov 25 08:00:03 rustnas kern.info kernel: sd 0:0:0:0: [sda] 976773168 512-byte logical blocks: (500 GB/466 GiB)',
  'Nov 25 08:00:04 rustnas daemon.info udhcpc: lease of 192.168.1.42 obtained, lease time 86400',
  'Nov 25 08:00:05 rustnas user.info rustnasd: web server running as uid 1000 (privilege-separated)',
  'Nov 25 08:00:05 rustnas user.info rustnasd: RustNAS 1.0.0 — http://0.0.0.0:8080',
  'Nov 25 08:00:05 rustnas user.info rustnasd: HTTPS on https://0.0.0.0:8443',
  'Nov 25 08:00:06 rustnas user.info rustnasd: mounted /dev/sda1 at /mnt/data',
  'Nov 25 08:00:06 rustnas kern.info kernel: ksmbd: kill command received',
  'Nov 25 08:00:07 rustnas daemon.info ksmbd.mountd: Starting SMBD ksmbd-tools 3.5.3',
  'Nov 25 09:12:44 rustnas user.info rustnasd: login "admin" from 192.168.1.20',
];

// ---------------------------------------------------------------- helpers
const send = (res, code, obj, headers = {}) => {
  res.writeHead(code, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store', ...headers });
  res.end(JSON.stringify(obj));
};
const ok = (res, extra = {}) => send(res, 200, { ok: true, ...extra });
const bad = (res, msg, code = 400) => send(res, code, { error: msg });
const readBody = (req) => new Promise((resolve) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => resolve(Buffer.concat(chunks)));
});
const cookieToken = (req) => (req.headers.cookie || '').split(';').map((s) => s.trim()).find((s) => s.startsWith('rustnas_session='))?.slice(16);
const authed = (req) => S.sessions.has(cookieToken(req));
const delay = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- API
async function handleApi(req, res, url) {
  const p = url.pathname;
  const m = req.method;
  const q = url.searchParams;
  if (m !== 'GET' && req.headers['x-rustnas'] !== '1') return bad(res, 'Yêu cầu bị chặn (thiếu header X-RustNAS)', 403);

  const raw = m === 'PUT' ? null : await readBody(req);
  let body = {};
  if (raw && raw.length) { try { body = JSON.parse(raw.toString('utf8')); } catch { return bad(res, 'JSON không hợp lệ'); } }
  await delay(120 + Math.random() * 180); // feel like a real Pi

  // public
  if (p === '/api/auth/status') return send(res, 200, { setup_required: false, user: authed(req) ? S.admin.user : null, version: '1.0.0', hostname: S.hostname, secure: false, demo: true });
  if (p === '/api/auth/login' && m === 'POST') {
    if (body.username !== S.admin.user || body.password !== S.admin.pass) { await delay(800); return bad(res, 'Sai tên đăng nhập hoặc mật khẩu', 401); }
    const tok = randomBytes(16).toString('hex');
    S.sessions.add(tok);
    return send(res, 200, { ok: true, user: S.admin.user }, { 'Set-Cookie': `rustnas_session=${tok}; Path=/; HttpOnly; SameSite=Strict` });
  }
  if (p === '/api/auth/logout') { S.sessions.delete(cookieToken(req)); return send(res, 200, { ok: true }, { 'Set-Cookie': 'rustnas_session=; Path=/; Max-Age=0' }); }
  if (p === '/api/network/confirm' && m === 'POST') {
    if (!S.pending || body.token !== S.pending.token) return bad(res, 'Mã xác nhận không đúng hoặc đã hết hạn');
    if (S.pending.net) S.net = S.pending.net;
    if (S.pending.fw) S.fw = S.pending.fw;
    S.pending = null;
    return ok(res);
  }
  if (!authed(req)) return bad(res, 'Chưa đăng nhập hoặc phiên đã hết hạn', 401);

  // system
  if (p === '/api/auth/password') {
    if (body.old_password !== S.admin.pass) return bad(res, 'Mật khẩu hiện tại không đúng');
    if ((body.new_password || '').length < 8) return bad(res, 'Mật khẩu cần từ 8 đến 128 ký tự');
    S.admin.pass = body.new_password; S.sessions.clear(); return ok(res);
  }
  if (p === '/api/alerts') return send(res, 200, alerts());
  if (p === '/api/system/overview') return send(res, 200, snapshot());
  if (p === '/api/system/history') return send(res, 200, S.history);
  if (p === '/api/system/services') return send(res, 200, [
    { id: 'rustnasd', name: 'RustNAS daemon', running: true, pids: [312, 318], rss_bytes: 9.1 * 2 ** 20, controllable: false },
    { id: 'smb', name: 'SMB server (ksmbd)', running: true, pids: [401, 402], rss_bytes: 3.2 * 2 ** 20, controllable: true },
    { id: 'nfs', name: 'NFS server', running: S.shares.nfs_enabled, pids: S.shares.nfs_enabled ? [455] : [], rss_bytes: 1.8 * 2 ** 20, controllable: true },
    { id: 'network', name: 'Mạng / DHCP', running: true, pids: [201], rss_bytes: 0.6 * 2 ** 20, controllable: false },
    { id: 'syslog', name: 'System log', running: true, pids: [118], rss_bytes: 0.5 * 2 ** 20, controllable: true },
  ]);
  if (p.startsWith('/api/system/services/')) return ok(res);
  if (p === '/api/system/logs') {
    const src = q.get('source') || 'all', needle = (q.get('q') || '').toLowerCase();
    const tags = { rustnasd: ['rustnasd'], smb: ['ksmbd'], network: ['udhcpc', 'eth0'], kernel: ['kernel:'], nfs: ['nfsd', 'rpc.'] }[src];
    const lines = LOGS.filter((l) => (!tags || tags.some((t) => l.includes(t))) && (!needle || l.toLowerCase().includes(needle)));
    return send(res, 200, { file: '/var/log/messages', lines });
  }
  if (p === '/api/system/power') return ok(res);
  if (p === '/api/system/backup') {
    const b = { format: 'rustnas-backup-1', version: '1.0.0', created: now(), hostname: S.hostname, secrets: q.get('secrets') === '1', files: { 'shares.toml': '# demo' } };
    res.writeHead(200, { 'Content-Type': 'application/json', 'Content-Disposition': `attachment; filename="rustnas-backup-${S.hostname}.json"` });
    return res.end(JSON.stringify(b, null, 2));
  }
  if (p === '/api/system/restore') return body.format === 'rustnas-backup-1' ? ok(res) : bad(res, 'Không phải file sao lưu RustNAS');
  if (p === '/api/system/update' && m === 'GET') return send(res, 200, { supported: true, version: '1.0.0', board: 'rpi3b', ...S.update });
  if (p === '/api/system/update' && m === 'PUT') { await readBody(req); await delay(1500); return bad(res, 'Gói không hợp lệ: thiếu manifest.json ở đầu (bản xem thử)'); }
  if (p === '/api/system/update/rollback') return ok(res, { slot: 'b' });
  if (p === '/api/settings' && m === 'GET') return send(res, 200, { ...S.settings, http_port: 8080, https_port: 8443, fingerprint: '3A:91:7C:0E:44:B2:19:8D:FA:60:2C:7E:91:05:AB:CD:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00' });
  if (p === '/api/settings' && m === 'POST') { S.settings = { https: !!body.https, http_redirect: !!body.http_redirect }; return ok(res); }
  if (p === '/api/settings/regenerate-cert') return ok(res);

  // storage
  if (p === '/api/storage/disks') return send(res, 200, { disks: disksJson(), smart: S.smart });
  if (p.startsWith('/api/storage/smart/')) { await delay(600); const d = decodeURIComponent(p.split('/').pop()); return send(res, 200, S.smart[d] || { disk: d, available: false, message: 'Không đọc được SMART' }); }
  if (p === '/api/storage/mount') {
    const [d, part] = findPart(body.device);
    if (!part || d.system) return bad(res, 'Không tìm thấy phân vùng');
    if (!part.fstype) return bad(res, 'Phân vùng chưa có filesystem — hãy format trước');
    part.mountpoint = `/mnt/${part.label || part.name}`; part.total = part.size * 0.98; part.used = part.size * 0.41;
    return ok(res, { mountpoint: part.mountpoint });
  }
  if (p === '/api/storage/unmount') {
    const [, part] = findPart(body.device);
    if (!part) return bad(res, 'Không tìm thấy phân vùng');
    const used = S.shares.list.find((s) => s.path.startsWith(part.mountpoint));
    if (used) return bad(res, `Chia sẻ ${used.path} đang nằm trên ổ này — hãy xóa chia sẻ trước`);
    part.mountpoint = null; return ok(res);
  }
  if (p === '/api/storage/format' || p === '/api/storage/initialize') {
    if (body.confirm !== body.device) return bad(res, 'Tên xác nhận không khớp');
    await delay(2000);
    if (p.endsWith('initialize')) {
      const d = S.disks.find((x) => x.name === body.device);
      d.parts = [{ name: `${d.name}1`, fstype: 'ext4', label: body.label || 'data', size: d.size }];
    } else {
      const [, part] = findPart(body.device);
      Object.assign(part, { fstype: 'ext4', label: body.label || null });
    }
    return ok(res);
  }
  if (p === '/api/storage/fix-owner') { await delay(800); return ok(res); }

  // files
  if (p === '/api/files' && m === 'GET') {
    const fp = q.get('path') || '';
    if (!fp || fp === '/mnt') return send(res, 200, { roots: [{ name: 'data', path: '/mnt/data', total: 492 * GB, used: 161 * GB, avail: 331 * GB }] });
    const n = nodeAt(fp);
    if (!n || !n.dir) return bad(res, 'Không tìm thấy file/thư mục');
    const entries = Object.entries(n.children).map(([name, e]) => ({ name, dir: e.dir, symlink: false, size: e.dir ? 0 : e.size, mtime: e.mtime }))
      .sort((a, b) => (b.dir - a.dir) || a.name.localeCompare(b.name, 'vi'));
    return send(res, 200, { path: fp, mount: '/mnt/data', entries });
  }
  if (p === '/api/files/download') {
    const n = nodeAt(q.get('path') || '');
    if (!n || n.dir) return bad(res, 'Không tìm thấy file');
    const name = path.posix.basename(q.get('path'));
    res.writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8', 'Content-Disposition': `attachment; filename*=UTF-8''${encodeURIComponent(name)}` });
    return res.end(`Đây là file mẫu "${name}" từ bản xem thử RustNAS.\n`);
  }
  if (p === '/api/files/upload' && m === 'PUT') {
    const n = nodeAt(q.get('dir') || '');
    const data = await readBody(req);
    if (!n || !n.dir) return bad(res, 'Đích không phải thư mục');
    const name = q.get('name');
    if (n.children[name] && q.get('overwrite') !== '1') return bad(res, `'${name}' đã tồn tại`);
    n.children[name] = F(data.length, 0);
    return ok(res, { size: data.length });
  }
  if (p === '/api/files/mkdir') {
    const n = nodeAt(body.path);
    if (!n) return bad(res, 'Không tìm thấy thư mục');
    if (!body.name || body.name.includes('/')) return bad(res, 'Tên file không hợp lệ');
    if (n.children[body.name]) return bad(res, 'Đã tồn tại');
    n.children[body.name] = D({}); return ok(res);
  }
  if (p === '/api/files/rename') {
    const parent = nodeAt(path.posix.dirname(body.path)), old = path.posix.basename(body.path);
    if (!parent || !parent.children[old]) return bad(res, 'Không tìm thấy');
    if (parent.children[body.name]) return bad(res, 'Tên mới đã tồn tại');
    parent.children[body.name] = parent.children[old]; delete parent.children[old]; return ok(res);
  }
  if (p === '/api/files/delete') {
    for (const fp of body.paths || []) {
      const parent = nodeAt(path.posix.dirname(fp)), name = path.posix.basename(fp);
      if (!parent?.children[name]) continue;
      const e = parent.children[name];
      S.trash.unshift({ id: `${now()}-${randomBytes(2).toString('hex')}`, mount: '/mnt/data', name, original: fp, deleted_at: now(), dir: e.dir, size: e.dir ? 0 : e.size, node: e });
      delete parent.children[name];
    }
    return ok(res, { moved: (body.paths || []).length });
  }
  if (p === '/api/trash') return send(res, 200, S.trash.map(({ node, ...t }) => t));
  if (p === '/api/trash/restore') {
    const i = S.trash.findIndex((t) => t.id === body.id);
    if (i < 0) return bad(res, 'Không tìm thấy trong thùng rác');
    const t = S.trash.splice(i, 1)[0];
    const parent = nodeAt(path.posix.dirname(t.original));
    if (parent) parent.children[parent.children[t.name] ? `${t.name} (khôi phục)` : t.name] = t.node;
    return ok(res, { path: t.original });
  }
  if (p === '/api/trash/purge') { const n = body.id ? 1 : S.trash.length; S.trash = body.id ? S.trash.filter((t) => t.id !== body.id) : []; return ok(res, { removed: n }); }

  // shares
  if (p === '/api/shares' && m === 'GET') return send(res, 200, { shares: S.shares.list, users: S.shares.users, workgroup: S.shares.workgroup, nfs_enabled: S.shares.nfs_enabled, nfs_available: true });
  if (p === '/api/shares' && m === 'POST') {
    if (!/^[A-Za-z0-9_-]{1,32}$/.test(body.name || '')) return bad(res, 'Tên chia sẻ chỉ gồm chữ, số, _ và - (tối đa 32 ký tự)');
    if (!(body.path || '').startsWith('/mnt/data')) return bad(res, 'Thư mục phải nằm trên ổ dữ liệu đã mount');
    if (body.path.includes('..')) return bad(res, 'Đường dẫn không được chứa . hoặc ..');
    if (body.nfs && !body.nfs_clients) return bad(res, 'Hãy nhập mạng được phép truy cập NFS, ví dụ 192.168.1.0/24');
    if (body.guest_ok) body.users = [];
    const i = S.shares.list.findIndex((s) => s.name.toLowerCase() === body.name.toLowerCase());
    if (i >= 0) S.shares.list[i] = body; else S.shares.list.push(body);
    S.shares.list.sort((a, b) => a.name.localeCompare(b.name));
    return ok(res);
  }
  if (p.startsWith('/api/shares/') && m === 'DELETE') { const n = decodeURIComponent(p.split('/').pop()); S.shares.list = S.shares.list.filter((s) => s.name !== n); return ok(res); }
  if (p === '/api/nfs') { S.shares.nfs_enabled = !!body.enabled; return ok(res); }
  if (p === '/api/smb-users' && m === 'POST') {
    if (!/^[a-z_][a-z0-9_-]{0,31}$/.test(body.username || '')) return bad(res, 'Tên người dùng: chữ thường, số, _ và -, bắt đầu bằng chữ (tối đa 32)');
    if ((body.password || '').length < 8) return bad(res, 'Mật khẩu SMB cần 8–128 ký tự, không chứa ký tự điều khiển');
    if (!S.shares.users.includes(body.username)) S.shares.users.push(body.username);
    S.shares.users.sort(); return ok(res);
  }
  if (p.startsWith('/api/smb-users/') && m === 'DELETE') {
    const u = decodeURIComponent(p.split('/').pop());
    const used = S.shares.list.filter((s) => s.users.some((x) => x.name === u)).map((s) => s.name);
    if (used.length) return bad(res, `'${u}' đang được gán cho chia sẻ: ${used.join(', ')} — hãy sửa chia sẻ trước`);
    S.shares.users = S.shares.users.filter((x) => x !== u); return ok(res);
  }

  // network
  if (p === '/api/network' && m === 'GET') return send(res, 200, {
    info: { hostname: S.hostname, gateway: '192.168.1.1', dns: ['192.168.1.1'], interfaces: [
      { name: 'eth0', mac: 'b8:27:eb:12:34:56', state: 'up', mtu: 1500, speed_mbps: 100, ipv4: [S.net.mode === 'static' ? S.net.address : '192.168.1.42/24'], rx_bytes: 48.2 * GB, tx_bytes: 9.7 * GB },
    ] },
    config: S.net, pending: S.pending && { kind: S.pending.kind, deadline: S.pending.deadline }, rollback_secs: 120,
  });
  const arm = (kind, net, fw) => {
    if (S.pending) return null;
    const token = randomBytes(16).toString('hex');
    S.pending = { kind, token, deadline: now() + 120, net, fw };
    setTimeout(() => { if (S.pending?.token === token) S.pending = null; }, 120000);
    return S.pending;
  };
  if (p === '/api/network/config') {
    if (body.mode === 'static' && !/^\d+\.\d+\.\d+\.\d+\/\d+$/.test(body.address || '')) return bad(res, 'Địa chỉ IP phải có dạng 192.168.1.50/24');
    const pd = arm('network', body, null);
    return pd ? ok(res, { token: pd.token, deadline: pd.deadline, config: body }) : bad(res, 'Đang có một thay đổi mạng chờ xác nhận', 409);
  }
  if (p === '/api/network/revert') { S.pending = null; return ok(res); }
  if (p === '/api/network/hostname') {
    if (!/^[A-Za-z0-9]+(-[A-Za-z0-9]+)*$/.test(body.hostname || '')) return bad(res, 'Tên máy chỉ gồm chữ, số và dấu - (tối đa 63 ký tự)');
    S.hostname = body.hostname; return ok(res);
  }
  if (p === '/api/firewall' && m === 'GET') return send(res, 200, S.fw);
  if (p === '/api/firewall' && m === 'POST') {
    if (body.enabled && !(body.allowed || []).length) return bad(res, 'Cần ít nhất một mạng được phép, nếu không bạn sẽ tự khóa mình');
    const pd = arm('firewall', null, body);
    return pd ? ok(res, { token: pd.token, deadline: pd.deadline }) : bad(res, 'Đang có một thay đổi mạng chờ xác nhận', 409);
  }
  return bad(res, 'Không có API này', 404);
}

// ---------------------------------------------------------------- static
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml' };
createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  try {
    if (url.pathname.startsWith('/api/')) return await handleApi(req, res, url);
    const name = ['/app.js', '/style.css', '/favicon.svg'].includes(url.pathname) ? url.pathname.slice(1) : 'index.html';
    const data = await readFile(path.join(ROOT, name));
    res.writeHead(200, { 'Content-Type': TYPES[path.extname(name)], 'Cache-Control': 'no-cache' });
    res.end(data);
  } catch (e) {
    console.error(e);
    if (!res.headersSent) bad(res, String(e), 500);
  }
}).listen(PORT, () => console.log(`RustNAS preview: http://localhost:${PORT}  (admin / rustnas123)`));
