'use strict';
/* RustNAS dashboard — vanilla JS, ~1 request every 3 s while a page is open. */

const $ = (sel, root = document) => root.querySelector(sel);
const view = $('#view');
const SVGNS = 'http://www.w3.org/2000/svg';
let timers = [];
let seq = 0; // bumps on every navigation so stale renders are dropped

// ---- tiny DOM helpers (text is always inserted as text, never as HTML) ----
function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v == null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
      else if (k === 'value') el.value = v;
      else if (k === 'checked') el.checked = !!v;
      else el.setAttribute(k, v === true ? '' : v);
    }
  }
  for (const c of kids.flat(Infinity)) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : String(c));
  }
  return el;
}
function svg(tag, attrs = {}) {
  const el = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  return el;
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pct = (a, b) => (b ? Math.round((a / b) * 100) : 0);
const tag = (text, kind = '') => h('span', { class: `tag ${kind}` }, text);

function fmtBytes(n, digits = 1) {
  if (n == null) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  n = Number(n);
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(i === 0 ? 0 : digits)} ${u[i]}`;
}
const fmtRate = (bps) => `${fmtBytes(bps)}/s`;
function fmtUptime(sec) {
  const d = Math.floor(sec / 86400), hh = Math.floor((sec % 86400) / 3600), m = Math.floor((sec % 3600) / 60);
  return `${d ? d + ' ngày ' : ''}${hh} giờ ${m} phút`;
}

function toast(msg, kind = 'ok') {
  const t = h('div', { class: `toast ${kind}`, role: 'status' }, msg);
  $('#toasts').append(t);
  setTimeout(() => t.remove(), kind === 'err' ? 6000 : 3000);
}

async function api(path, { method = 'GET', body } = {}) {
  const opt = { method, headers: {}, credentials: 'same-origin' };
  if (body !== undefined) {
    opt.headers['Content-Type'] = 'application/json';
    opt.body = JSON.stringify(body);
  }
  const res = await fetch(path, opt);
  let data = null;
  try { data = await res.json(); } catch { /* empty body */ }
  if (res.status === 401 && !path.startsWith('/api/auth/')) {
    showAuth(false);
    throw new Error('Phiên đăng nhập đã hết hạn');
  }
  if (!res.ok) throw new Error((data && data.error) || `Lỗi HTTP ${res.status}`);
  return data;
}

/** Run an action with a busy button and a toast; resolves to true on success. */
async function act(btn, fn, okMsg) {
  if (btn) btn.disabled = true;
  try {
    await fn();
    if (okMsg) toast(okMsg);
    return true;
  } catch (e) {
    toast(e.message, 'err');
    return false;
  } finally {
    if (btn) btn.disabled = false;
  }
}

/** Modal form. `submit(formData)` may throw: the error is shown and the dialog stays open. */
function modal(title, body, { ok = 'OK', danger = false, submit }) {
  return new Promise((resolve) => {
    const err = h('p', { class: 'form-err', role: 'alert' });
    const okBtn = h('button', { type: 'submit', class: danger ? 'btn danger' : 'btn primary' }, ok);
    const dlg = h('dialog', { class: 'modal' });
    const form = h('form', {},
      h('h3', {}, title), body, err,
      h('div', { class: 'actions' },
        h('button', { type: 'button', class: 'btn', onclick: () => dlg.close() }, 'Hủy'), okBtn));
    let done = false;
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      okBtn.disabled = true;
      err.textContent = '';
      try {
        await submit(new FormData(form));
        done = true;
        dlg.close();
      } catch (ex) {
        err.textContent = ex.message;
      } finally {
        okBtn.disabled = false;
      }
    });
    dlg.addEventListener('close', () => { dlg.remove(); resolve(done); });
    dlg.append(form);
    document.body.append(dlg);
    dlg.showModal();
    const first = form.querySelector('input:not([readonly])');
    if (first) first.focus();
  });
}

const field = (label, input, hint) =>
  h('label', { class: 'field' }, h('span', {}, label), input, hint && h('small', {}, hint));
const check = (name, label, checked, value) =>
  h('label', { class: 'check' }, h('input', { type: 'checkbox', name, checked, value: value ?? 'on' }), h('span', {}, label));
const panel = (title, body, tools) =>
  h('section', { class: 'panel' },
    h('header', {}, h('h2', {}, title), tools && h('div', { class: 'tools' }, tools)),
    h('div', { class: 'body' }, body));
const kvList = (pairs) => h('dl', { class: 'kv' }, pairs.map(([k, v]) => [h('dt', {}, k), h('dd', {}, v)]));

function meter(p) {
  p = Math.min(100, Math.max(0, p || 0));
  const bar = h('span');
  bar.style.width = `${p}%`;
  return h('div', { class: `meter${p >= 90 ? ' crit' : p >= 75 ? ' warn' : ''}` }, bar);
}

function table(headers, rows, empty = 'Không có dữ liệu') {
  if (!rows.length) return h('p', { class: 'muted' }, empty);
  return h('div', { class: 'table-wrap' },
    h('table', {},
      h('thead', {}, h('tr', {}, headers.map((x) => h('th', {}, x)))),
      h('tbody', {}, rows.map((r) => h('tr', {}, r.map((c) => h('td', {}, c)))))));
}

/** Line chart of one or two series; `max` null = auto-scale. */
function sparkChart(series, max, fmt, names) {
  const W = 300, H = 100;
  const n = series[0].length;
  if (n < 2) return h('p', { class: 'muted' }, 'Đang thu thập dữ liệu…');
  const top = max ?? Math.max(1, ...series.flat());
  const s = svg('svg', { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: 'none', role: 'img' });
  for (const y of [0.25, 0.5, 0.75]) s.append(svg('line', { class: 'grid-line', x1: 0, x2: W, y1: H * y, y2: H * y }));
  series.forEach((pts, i) => {
    const points = pts.map((v, j) => `${((j / (n - 1)) * W).toFixed(1)},${(H - 2 - (v / top) * (H - 4)).toFixed(1)}`).join(' ');
    s.append(svg('polyline', { class: `s${i}`, points }));
  });
  const legend = h('div', { class: 'legend' },
    series.map((pts, i) => h('span', {}, h('i', { class: `s${i}` }), `${names ? names[i] + ': ' : ''}${fmt(pts[n - 1])}`)),
    h('span', {}, `đỉnh trục: ${fmt(top)}`));
  return h('div', {}, s, legend);
}

// ---- auth -----------------------------------------------------------------
function logo() { return h('img', { src: '/favicon.svg', alt: '', width: 28, height: 28 }); }

function showAuth(setup) {
  seq++;
  timers.forEach(clearInterval);
  timers = [];
  $('#app').hidden = true;
  const box = $('#auth');
  box.hidden = false;
  const err = h('p', { class: 'form-err', role: 'alert' });
  const form = h('form', { class: 'auth-card' },
    h('div', { class: 'logo' }, logo(), h('span', {}, 'RustNAS')),
    h('h1', {}, setup ? 'Thiết lập lần đầu' : 'Đăng nhập'),
    setup && h('p', { class: 'muted' }, 'Tạo tài khoản quản trị. Người đầu tiên mở trang này sẽ thành quản trị viên, nên hãy làm ngay sau khi bật NAS.'),
    field('Tên đăng nhập', h('input', { name: 'username', autocomplete: 'username', required: true, value: setup ? 'admin' : '' })),
    field('Mật khẩu', h('input', { name: 'password', type: 'password', required: true, minlength: setup ? 8 : null, autocomplete: setup ? 'new-password' : 'current-password' })),
    setup && field('Nhập lại mật khẩu', h('input', { name: 'password2', type: 'password', required: true, autocomplete: 'new-password' })),
    err,
    h('button', { class: 'btn primary block', type: 'submit' }, setup ? 'Tạo tài khoản' : 'Đăng nhập'));
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(form);
    if (setup && fd.get('password') !== fd.get('password2')) { err.textContent = 'Mật khẩu nhập lại không khớp'; return; }
    const btn = form.querySelector('button[type=submit]');
    btn.disabled = true;
    err.textContent = '';
    try {
      const r = await api(setup ? '/api/auth/setup' : '/api/auth/login', {
        method: 'POST', body: { username: fd.get('username'), password: fd.get('password') },
      });
      showApp(r.user);
    } catch (ex) {
      err.textContent = ex.message;
    } finally {
      btn.disabled = false;
    }
  });
  box.replaceChildren(form);
  form.querySelector(setup ? 'input[name=password]' : 'input').focus();
}

function showApp(user) {
  $('#auth').hidden = true;
  $('#app').hidden = false;
  $('#user-name').textContent = user;
  route();
}

// ---- routing --------------------------------------------------------------
const pages = { overview: pageOverview, storage: pageStorage, shares: pageShares, network: pageNetwork, system: pageSystem };

function route() {
  const my = ++seq;
  timers.forEach(clearInterval);
  timers = [];
  const id = location.hash.replace(/^#\/?/, '') || 'overview';
  const page = pages[id] ? id : 'overview';
  document.querySelectorAll('.nav a').forEach((a) => a.classList.toggle('active', a.dataset.page === page));
  view.replaceChildren(h('p', { class: 'muted pad' }, 'Đang tải…'));
  const ctx = {
    show: (...nodes) => { if (my === seq) view.replaceChildren(...nodes.flat().filter(Boolean)); },
    every: (fn, ms) => {
      if (my !== seq) return;
      timers.push(setInterval(() => fn().catch(() => {}), ms));
    },
  };
  pages[page](ctx).catch((e) => {
    if (my === seq && !$('#app').hidden) view.replaceChildren(h('div', { class: 'alert err' }, e.message));
  });
}
window.addEventListener('hashchange', route);

// ---- Overview -------------------------------------------------------------
function card(title, value, meterEl, sub) {
  return h('div', { class: 'card' }, h('div', { class: 'card-title' }, title), h('div', { class: 'card-value' }, value), meterEl, h('div', { class: 'card-sub' }, sub));
}

function diskSummary(disks) {
  const parts = disks.flatMap((d) => d.partitions.map((p) => ({ d, p }))).filter(({ p }) => p.mountpoint && p.total_bytes);
  if (!parts.length) return h('p', { class: 'muted' }, 'Chưa có ổ nào được mount.');
  return parts.map(({ d, p }) => h('div', { class: 'field' },
    h('span', {}, `${p.label || p.name} `, h('code', {}, p.mountpoint), d.system ? ' (hệ thống)' : ''),
    meter(pct(p.used_bytes, p.total_bytes)),
    h('small', {}, `${fmtBytes(p.used_bytes)} đã dùng / ${fmtBytes(p.total_bytes)} · ${p.fstype || ''}`)));
}

async function pageOverview({ show, every }) {
  const cards = h('div', { class: 'cards' });
  const cpuChart = h('div', { class: 'chart' });
  const netChart = h('div', { class: 'chart' });
  const info = h('div');
  const draw = (o, hist) => {
    $('#node-name').textContent = o.hostname;
    $('#version').textContent = `v${o.version}`;
    const memUsed = o.mem_total - o.mem_available;
    const t = o.temp_c;
    cards.replaceChildren(
      card('CPU', `${o.cpu_percent.toFixed(0)}%`, meter(o.cpu_percent), `${o.cpu_cores} nhân · tải ${o.load.map((x) => x.toFixed(2)).join(' / ')}`),
      card('Bộ nhớ', fmtBytes(memUsed), meter(pct(memUsed, o.mem_total)), `trên ${fmtBytes(o.mem_total)} · cache ${fmtBytes(o.mem_cached)}`),
      card('Nhiệt độ', t != null ? `${t.toFixed(1)} °C` : '—', meter(t != null ? (t / 85) * 100 : 0), t >= 80 ? 'Quá nóng: CPU đang bị hạ xung' : 'Pi 3B hạ xung ở 80 °C'),
      card('Mạng', `↓ ${fmtRate(o.net_rx_bps)}`, null, `↑ ${fmtRate(o.net_tx_bps)} · Ethernet tối đa ~11 MB/s`),
    );
    cpuChart.replaceChildren(sparkChart([hist.map((p) => p.cpu)], 100, (v) => `${v.toFixed(0)}%`));
    netChart.replaceChildren(sparkChart([hist.map((p) => p.rx), hist.map((p) => p.tx)], null, fmtRate, ['Nhận', 'Gửi']));
    const swapUsed = o.swap_total - o.swap_free;
    info.replaceChildren(kvList([
      ['Tên máy', o.hostname],
      ['Phần cứng', o.model || '—'],
      ['Kernel', o.kernel],
      ['RustNAS', `v${o.version}`],
      ['Thời gian chạy', fmtUptime(o.uptime_secs)],
      ['Swap (zram)', o.swap_total ? `${fmtBytes(swapUsed)} / ${fmtBytes(o.swap_total)}` : 'tắt'],
      ['RAM của rustnasd', fmtBytes(o.daemon_rss)],
    ]));
  };
  const [o, hist, disks] = await Promise.all([api('/api/system/overview'), api('/api/system/history'), api('/api/storage/disks')]);
  draw(o, hist);
  show(
    h('h1', {}, 'Tổng quan'),
    cards,
    h('div', { class: 'grid2' }, panel('Tải CPU (10 phút)', cpuChart), panel('Lưu lượng mạng', netChart)),
    h('div', { class: 'grid2' },
      panel('Ổ đĩa', diskSummary(disks), h('a', { href: '#/storage', class: 'btn small' }, 'Quản lý')),
      panel('Hệ thống', info)),
  );
  every(async () => {
    const [o2, h2] = await Promise.all([api('/api/system/overview'), api('/api/system/history')]);
    draw(o2, h2);
  }, 3000);
}

// ---- Storage --------------------------------------------------------------
function partActions(p, reload) {
  const post = (path, body, msg) => (e) => act(e.currentTarget, () => api(path, { method: 'POST', body }), msg).then((ok) => ok && reload());
  if (p.mountpoint) {
    return h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: post('/api/storage/unmount', { device: p.name }, `Đã tháo ${p.name}`) }, 'Tháo'));
  }
  return h('div', { class: 'row-actions' },
    p.fstype && h('button', { class: 'btn small primary', onclick: post('/api/storage/mount', { device: p.name }, `Đã mount ${p.name}`) }, 'Mount'),
    h('button', { class: 'btn small danger', onclick: () => formatDialog(p, reload) }, 'Format'));
}

async function formatDialog(p, reload) {
  const ok = await modal(`Format ${p.name} thành ext4`, [
    h('div', { class: 'alert err' }, `Toàn bộ dữ liệu trên /dev/${p.name} (${fmtBytes(p.size_bytes)}${p.fstype ? ', ' + p.fstype : ''}) sẽ bị xóa vĩnh viễn.`),
    field('Nhãn ổ (tùy chọn)', h('input', { name: 'label', maxlength: 16, pattern: '[A-Za-z0-9_]*', value: p.label || 'data' }), 'Ổ sẽ được mount tại /mnt/<nhãn>.'),
    field(`Gõ "${p.name}" để xác nhận`, h('input', { name: 'confirm', required: true, autocomplete: 'off' })),
  ], {
    ok: 'Format', danger: true,
    submit: (fd) => api('/api/storage/format', { method: 'POST', body: { device: p.name, label: fd.get('label') || '', confirm: fd.get('confirm') } }),
  });
  if (ok) { toast(`Đã format ${p.name}`); reload(); }
}

async function pageStorage({ show }) {
  const disks = await api('/api/storage/disks');
  const reload = () => route();
  const blocks = disks.map((d) => panel(
    `${d.name} · ${d.model || 'Không rõ model'}`,
    table(['Phân vùng', 'Filesystem', 'Nhãn', 'Dung lượng', 'Đã dùng', 'Mount', ''], d.partitions.map((p) => [
      h('code', {}, p.name),
      p.fstype || h('span', { class: 'muted' }, 'trống'),
      p.label || '—',
      fmtBytes(p.size_bytes),
      p.total_bytes ? h('div', { class: 'usage' }, meter(pct(p.used_bytes, p.total_bytes)), h('small', {}, `${fmtBytes(p.used_bytes)} / ${fmtBytes(p.total_bytes)}`)) : '—',
      p.mountpoint ? h('code', {}, p.mountpoint) : h('span', { class: 'muted' }, 'chưa mount'),
      d.system ? tag('hệ thống') : partActions(p, reload),
    ]), 'Ổ chưa có phân vùng hoặc filesystem nào được nhận ra.'),
    h('span', { class: 'tags' },
      tag(d.transport.toUpperCase()), tag(fmtBytes(d.size_bytes)), tag(d.rotational ? 'HDD' : 'SSD/Flash'),
      d.system && tag('Thẻ hệ thống', 'warn')),
  ));
  show(
    h('div', { class: 'page-head' }, h('h1', {}, 'Ổ đĩa'), h('button', { class: 'btn', onclick: reload }, 'Làm mới')),
    h('p', { class: 'muted' }, 'Dữ liệu NAS nên nằm trên ổ USB, thẻ nhớ chỉ chứa hệ điều hành. Ổ HDD 2.5" nên cắm qua hub USB có nguồn riêng. Ổ đã mount sẽ tự mount lại sau khi khởi động.'),
    blocks.length ? blocks : h('div', { class: 'alert' }, 'Không tìm thấy ổ đĩa nào.'),
  );
}

// ---- Shares ---------------------------------------------------------------
async function shareDialog(s, users, mounts, reload) {
  const editing = !!s;
  s = s || { name: '', path: mounts[0] ? `${mounts[0]}/share` : '/mnt/', comment: '', read_only: false, guest_ok: false, valid_users: [] };
  const ok = await modal(editing ? `Sửa chia sẻ ${s.name}` : 'Thêm chia sẻ', [
    field('Tên chia sẻ', h('input', { name: 'name', value: s.name, required: true, maxlength: 32, pattern: '[A-Za-z0-9_]+(-[A-Za-z0-9_]+)*', readonly: editing })),
    field('Thư mục', h('input', { name: 'path', value: s.path, required: true, list: 'mount-list' }), 'Nằm trên ổ đã mount, ví dụ /mnt/data/phim. Thư mục chưa có sẽ được tạo.'),
    h('datalist', { id: 'mount-list' }, mounts.map((m) => h('option', { value: m }))),
    field('Mô tả', h('input', { name: 'comment', value: s.comment, maxlength: 80 })),
    check('read_only', 'Chỉ đọc', s.read_only),
    check('guest_ok', 'Cho khách truy cập (không cần mật khẩu)', s.guest_ok),
    users.length
      ? h('fieldset', {}, h('legend', {}, 'Người dùng được phép (bỏ trống = mọi người dùng SMB)'), users.map((u) => check('user', u, s.valid_users.includes(u), u)))
      : h('p', { class: 'muted' }, 'Chưa có người dùng SMB. Hãy tạo ở trang Chia sẻ hoặc bật quyền khách.'),
  ], {
    ok: editing ? 'Lưu' : 'Tạo',
    submit: (fd) => api('/api/shares', {
      method: 'POST',
      body: {
        name: fd.get('name'), path: fd.get('path'), comment: fd.get('comment') || '',
        read_only: fd.has('read_only'), guest_ok: fd.has('guest_ok'), valid_users: fd.getAll('user'),
      },
    }),
  });
  if (ok) { toast('Đã lưu chia sẻ'); reload(); }
}

async function userDialog(user, reload) {
  const ok = await modal(user ? `Đổi mật khẩu SMB của ${user}` : 'Thêm người dùng SMB', [
    field('Tên người dùng', h('input', { name: 'username', value: user || '', required: true, maxlength: 32, pattern: '[a-z_][a-z0-9_]*(-[a-z0-9_]+)*', readonly: !!user, autocomplete: 'off' }), !user && 'Chữ thường, số, _ và -.'),
    field('Mật khẩu', h('input', { name: 'password', type: 'password', required: true, minlength: 6, autocomplete: 'new-password' })),
    field('Nhập lại mật khẩu', h('input', { name: 'password2', type: 'password', required: true, autocomplete: 'new-password' })),
  ], {
    ok: user ? 'Đổi mật khẩu' : 'Tạo',
    submit: async (fd) => {
      if (fd.get('password') !== fd.get('password2')) throw new Error('Mật khẩu nhập lại không khớp');
      await api('/api/smb-users', { method: 'POST', body: { username: fd.get('username'), password: fd.get('password') } });
    },
  });
  if (ok) { toast('Đã lưu người dùng'); reload(); }
}

async function confirmDelete(title, text, path, reload) {
  const ok = await modal(title, [h('p', {}, text)], { ok: 'Xóa', danger: true, submit: () => api(path, { method: 'DELETE' }) });
  if (ok) { toast('Đã xóa'); reload(); }
}

async function pageShares({ show }) {
  const [data, disks, net] = await Promise.all([api('/api/shares'), api('/api/storage/disks'), api('/api/network')]);
  const ip = (net.interfaces.flatMap((i) => i.ipv4)[0] || location.hostname).split('/')[0];
  const mounts = disks.filter((d) => !d.system).flatMap((d) => d.partitions).filter((p) => p.mountpoint).map((p) => p.mountpoint);
  const reload = () => route();

  const shareRows = data.shares.map((s) => [
    h('strong', {}, s.name),
    h('code', {}, s.path),
    s.read_only ? 'Chỉ đọc' : 'Đọc/ghi',
    s.guest_ok ? tag('Khách', 'warn') : s.valid_users.length ? s.valid_users.join(', ') : 'Mọi người dùng SMB',
    h('code', {}, `\\\\${ip}\\${s.name}`),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: () => shareDialog(s, data.users, mounts, reload) }, 'Sửa'),
      h('button', {
        class: 'btn small danger',
        onclick: () => confirmDelete('Xóa chia sẻ', `Xóa chia sẻ "${s.name}"? Dữ liệu trong ${s.path} vẫn được giữ nguyên.`, `/api/shares/${encodeURIComponent(s.name)}`, reload),
      }, 'Xóa')),
  ]);
  const userRows = data.users.map((u) => [
    h('strong', {}, u),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: () => userDialog(u, reload) }, 'Đổi mật khẩu'),
      h('button', { class: 'btn small danger', onclick: () => confirmDelete('Xóa người dùng', `Xóa người dùng SMB "${u}"?`, `/api/smb-users/${encodeURIComponent(u)}`, reload) }, 'Xóa')),
  ]);

  show(
    h('div', { class: 'page-head' },
      h('h1', {}, 'Chia sẻ (SMB)'),
      h('button', { class: 'btn primary', onclick: () => shareDialog(null, data.users, mounts, reload) }, '+ Thêm chia sẻ')),
    !mounts.length && h('div', { class: 'alert' }, 'Chưa có ổ dữ liệu nào được mount. Vào ', h('a', { href: '#/storage' }, 'Ổ đĩa'), ' để mount trước.'),
    panel('Thư mục chia sẻ', table(['Tên', 'Thư mục', 'Quyền', 'Truy cập', 'Địa chỉ', ''], shareRows, 'Chưa có thư mục chia sẻ nào.')),
    panel('Người dùng SMB', table(['Tên', ''], userRows, 'Chưa có người dùng SMB nào.'),
      h('button', { class: 'btn small primary', onclick: () => userDialog(null, reload) }, '+ Thêm người dùng')),
    panel('Cách kết nối', h('ul', { class: 'help' },
      h('li', {}, 'Windows: mở File Explorer, gõ ', h('code', {}, `\\\\${ip}`), ' vào thanh địa chỉ.'),
      h('li', {}, 'macOS: Finder → Go → Connect to Server → ', h('code', {}, `smb://${ip}`)),
      h('li', {}, 'Linux: trình quản lý file → ', h('code', {}, `smb://${ip}/<tên chia sẻ>`)),
      h('li', {}, 'Android/iOS: dùng ứng dụng quản lý file có hỗ trợ SMB.'))),
  );
}

// ---- Network --------------------------------------------------------------
async function pageNetwork({ show }) {
  const n = await api('/api/network');
  const hostForm = h('form', { class: 'inline-form' },
    h('input', { name: 'hostname', value: n.hostname, required: true, maxlength: 63, pattern: '[A-Za-z0-9]+(-[A-Za-z0-9]+)*', 'aria-label': 'Tên máy' }),
    h('button', { class: 'btn primary', type: 'submit' }, 'Lưu'));
  hostForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const hostname = new FormData(hostForm).get('hostname');
    if (await act(hostForm.querySelector('button'), () => api('/api/network/hostname', { method: 'POST', body: { hostname } }), 'Đã đổi tên máy')) route();
  });
  show(
    h('h1', {}, 'Mạng'),
    panel('Tên máy', h('div', {}, hostForm, h('p', { class: 'muted' }, 'Tên hiển thị trong mạng Windows (NetBIOS dùng tối đa 15 ký tự đầu).'))),
    panel('Card mạng', table(['Tên', 'Trạng thái', 'IPv4', 'MAC', 'Tốc độ', 'Lưu lượng'], n.interfaces.map((i) => [
      h('strong', {}, i.name),
      i.state === 'up' ? tag('up', 'ok') : tag(i.state || 'unknown'),
      i.ipv4.length ? i.ipv4.map((a) => h('div', {}, h('code', {}, a))) : '—',
      h('code', {}, i.mac || '—'),
      i.speed_mbps ? `${i.speed_mbps} Mb/s` : '—',
      `↓ ${fmtBytes(i.rx_bytes)} · ↑ ${fmtBytes(i.tx_bytes)}`,
    ]))),
    panel('Định tuyến và DNS', kvList([
      ['Gateway mặc định', n.gateway || '—'],
      ['DNS', n.dns.join(', ') || '—'],
      ['Cấp IP', 'DHCP (đặt IP tĩnh: dự kiến ở v0.5; tạm thời hãy gán IP cố định trên router)'],
    ])),
  );
}

// ---- System ---------------------------------------------------------------
async function power(action) {
  const label = action === 'reboot' ? 'Khởi động lại' : 'Tắt máy';
  const ok = await modal(label, [h('p', {}, action === 'reboot'
    ? 'NAS sẽ khởi động lại, kết nối SMB bị ngắt khoảng 1 phút.'
    : 'NAS sẽ tắt hẳn. Muốn bật lại phải rút điện rồi cắm lại.')],
  { ok: label, danger: true, submit: () => api('/api/system/power', { method: 'POST', body: { action } }) });
  if (!ok) return;
  seq++;
  timers.forEach(clearInterval);
  timers = [];
  view.replaceChildren(h('div', { class: 'alert' }, action === 'reboot'
    ? 'Đang khởi động lại… Trang sẽ tự tải lại khi NAS sẵn sàng.'
    : 'Đã gửi lệnh tắt máy. Rút điện khi đèn ACT xanh ngừng nhấp nháy.'));
  if (action !== 'reboot') return;
  await sleep(20000);
  for (;;) {
    try { await fetch('/api/auth/status'); location.reload(); return; } catch { await sleep(3000); }
  }
}

async function pageSystem({ show }) {
  const [svcs, logs] = await Promise.all([api('/api/system/services'), api('/api/system/logs')]);
  const reload = () => route();
  const labels = { start: 'Chạy', restart: 'Khởi động lại', stop: 'Dừng' };
  const svcRows = svcs.map((s) => [
    s.name,
    s.running ? tag('đang chạy', 'ok') : tag('đã dừng', 'err'),
    s.running ? fmtBytes(s.rss_bytes) : '—',
    s.pids.join(', ') || '—',
    s.controllable ? h('div', { class: 'row-actions' }, Object.keys(labels).map((a) => h('button', {
      class: 'btn small',
      onclick: (e) => act(e.currentTarget, () => api(`/api/system/services/${s.id}/${a}`, { method: 'POST' }), `${s.name}: ${labels[a]}`).then((ok) => ok && setTimeout(reload, 600)),
    }, labels[a]))) : '',
  ]);

  const pw = h('form', {},
    field('Mật khẩu hiện tại', h('input', { name: 'old', type: 'password', required: true, autocomplete: 'current-password' })),
    field('Mật khẩu mới', h('input', { name: 'new', type: 'password', required: true, minlength: 8, autocomplete: 'new-password' })),
    field('Nhập lại mật khẩu mới', h('input', { name: 'new2', type: 'password', required: true, autocomplete: 'new-password' })),
    h('button', { class: 'btn primary', type: 'submit' }, 'Đổi mật khẩu'));
  pw.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(pw);
    if (fd.get('new') !== fd.get('new2')) { toast('Mật khẩu nhập lại không khớp', 'err'); return; }
    const done = await act(pw.querySelector('button'), () => api('/api/auth/password', { method: 'POST', body: { old_password: fd.get('old'), new_password: fd.get('new') } }), 'Đã đổi mật khẩu, hãy đăng nhập lại');
    if (done) showAuth(false);
  });

  const pre = h('pre', { class: 'logs' }, logs.lines.join('\n') || '(trống)');
  const refreshLogs = (e) => act(e.currentTarget, async () => {
    const r = await api('/api/system/logs');
    pre.textContent = r.lines.join('\n') || '(trống)';
    pre.scrollTop = pre.scrollHeight;
  });

  show(
    h('h1', {}, 'Hệ thống'),
    panel('Dịch vụ', table(['Dịch vụ', 'Trạng thái', 'RAM', 'PID', ''], svcRows)),
    h('div', { class: 'grid2' },
      panel('Đổi mật khẩu quản trị', pw),
      panel('Nguồn', h('div', {},
        h('p', { class: 'muted' }, 'Hãy tắt máy từ đây thay vì rút điện để tránh hỏng dữ liệu trên ổ USB.'),
        h('div', { class: 'row-actions' },
          h('button', { class: 'btn', onclick: () => power('reboot') }, 'Khởi động lại'),
          h('button', { class: 'btn danger', onclick: () => power('poweroff') }, 'Tắt máy'))))),
    panel(`Nhật ký: ${logs.file}`, pre, h('button', { class: 'btn small', onclick: refreshLogs }, 'Làm mới')),
  );
  pre.scrollTop = pre.scrollHeight;
}

// ---- boot -----------------------------------------------------------------
$('#logout').addEventListener('click', async () => {
  try { await api('/api/auth/logout', { method: 'POST' }); } catch { /* ignore */ }
  showAuth(false);
});

(async function boot() {
  try {
    const s = await api('/api/auth/status');
    $('#node-name').textContent = s.hostname;
    $('#version').textContent = `v${s.version}`;
    document.title = `${s.hostname} · RustNAS`;
    if (s.setup_required) showAuth(true);
    else if (!s.user) showAuth(false);
    else showApp(s.user);
  } catch (e) {
    const box = $('#auth');
    box.hidden = false;
    box.replaceChildren(h('div', { class: 'auth-card' }, h('h1', {}, 'Không kết nối được rustnasd'), h('p', { class: 'muted' }, e.message)));
  }
})();
