'use strict';
/* HeheNAS OS dashboard — vanilla JS, no build step. Text is always inserted as text, never as HTML. */

const $ = (sel, root = document) => root.querySelector(sel);
const view = $('#view');
const SVGNS = 'http://www.w3.org/2000/svg';
let timers = [];
let seq = 0;
let status = {};

// ---- theme ----------------------------------------------------------------
function storedTheme() { try { return localStorage.getItem('hehenas-theme'); } catch { return null; } }
function applyTheme(t) {
  const theme = t || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  document.documentElement.dataset.theme = theme;
  return theme;
}
applyTheme(storedTheme());

// ---- DOM helpers ----------------------------------------------------------
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

const ICONS = {
  dashboard: 'M3 3h7v9H3zM14 3h7v5h-7zM14 12h7v9h-7zM3 16h7v5H3z',
  monitor: 'M22 12h-4l-3 9L9 3l-3 9H2',
  storage: 'M22 12H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11zM6 16h.01M10 16h.01',
  folder: 'M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z',
  share: 'M18 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.6 13.5l6.8 4M15.4 6.5l-6.8 4',
  network: 'M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z',
  shield: 'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z',
  shieldok: 'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10zM9 12l2 2 4-4',
  update: 'M21 12a9 9 0 1 1-3-6.7L21 8M21 3v5h-5',
  backup: 'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3',
  services: 'M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6',
  logs: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6M16 13H8M16 17H8M10 9H8',
  cpu: 'M9 3v2M15 3v2M9 19v2M15 19v2M3 9h2M3 15h2M19 9h2M19 15h2M7 5h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2zM9 9h6v6H9z',
  mem: 'M2 7a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2zM6 19v-3M10 19v-3M14 19v-3M18 19v-3M7 9v3M12 9v3M17 9v3',
  temp: 'M14 4v10.54a4 4 0 1 1-4 0V4a2 2 0 0 1 4 0z',
  net: 'M5 12.55a11 11 0 0 1 14.08 0M1.42 9a16 16 0 0 1 21.16 0M8.53 16.11a6 6 0 0 1 6.95 0M12 20h.01',
  bell: 'M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9M13.73 21a2 2 0 0 1-3.46 0',
  sun: 'M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42',
  moon: 'M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z',
  menu: 'M3 12h18M3 6h18M3 18h18',
  logout: 'M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9',
  key: 'M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.78 7.78 5.5 5.5 0 0 1 7.78-7.78zM15.5 7.5l3 3L22 7l-3-3',
  clock: 'M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2',
  file: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6',
  trash: 'M3 6h18M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6M10 11v6M14 11v6M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2',
  upload: 'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12',
  plus: 'M12 5v14M5 12h14',
  refresh: 'M23 4v6h-6M1 20v-6h6M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15',
  power: 'M18.36 6.64a9 9 0 1 1-12.73 0M12 2v10',
  lock: 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2zM7 11V7a5 5 0 0 1 10 0v4',
  info: 'M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01',
  alert: 'M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0zM12 9v4M12 17h.01',
};
function icon(name, cls = 'ico') {
  const el = svg('svg', { class: cls, viewBox: '0 0 24 24', 'aria-hidden': 'true' });
  el.append(svg('path', { d: ICONS[name] || '' }));
  return el;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pct = (a, b) => (b ? Math.round((a / b) * 100) : 0);
const tone = (p) => (p >= 90 ? 'crit' : p >= 75 ? 'warn' : 'ok');
const tag = (text, kind = '') => h('span', { class: `tag ${kind}` }, text);
function fmtBytes(n, digits = 1) {
  if (n == null || Number.isNaN(n)) return '—';
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
const fmtDate = (s) => (s ? new Date(s * 1000).toLocaleString('vi-VN') : '—');
const joinPath = (a, b) => `${a.replace(/\/+$/, '')}/${b}`;
const parentOf = (p) => p.replace(/\/[^/]+\/?$/, '') || '/';
const baseName = (p) => p.replace(/\/+$/, '').split('/').pop();

function toast(msg, kind = 'ok') {
  const t = h('div', { class: `toast ${kind}`, role: 'status' }, msg);
  $('#toasts').append(t);
  setTimeout(() => t.remove(), kind === 'err' ? 6000 : 3000);
}

async function api(path, { method = 'GET', body, raw } = {}) {
  const opt = { method, headers: {}, credentials: 'same-origin' };
  if (method !== 'GET') opt.headers['X-HeheNAS'] = '1';
  if (raw !== undefined) opt.body = raw;
  else if (body !== undefined) {
    opt.headers['Content-Type'] = 'application/json';
    opt.body = JSON.stringify(body);
  }
  const res = await fetch(path, opt);
  let data = null;
  try { data = await res.json(); } catch { /* empty body */ }
  if (res.status === 401 && !path.startsWith('/api/auth/') && !path.startsWith('/api/network/confirm')) {
    showAuth(false);
    throw new Error('Phiên đăng nhập đã hết hạn');
  }
  if (!res.ok) throw new Error((data && data.error) || `Lỗi HTTP ${res.status}`);
  return data;
}

function uploadFile(url, file, onProgress) {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open('PUT', url);
    x.setRequestHeader('X-HeheNAS', '1');
    x.setRequestHeader('Content-Type', 'application/octet-stream');
    x.upload.onprogress = (e) => e.lengthComputable && onProgress && onProgress(e.loaded / e.total);
    x.onload = () => {
      let d = null;
      try { d = JSON.parse(x.responseText); } catch { /* ignore */ }
      if (x.status >= 200 && x.status < 300) resolve(d);
      else reject(new Error((d && d.error) || `Lỗi HTTP ${x.status}`));
    };
    x.onerror = () => reject(new Error('Mất kết nối khi tải lên'));
    x.send(file);
  });
}

async function act(btn, fn, okMsg) {
  if (btn) btn.disabled = true;
  try {
    const r = await fn();
    if (okMsg) toast(okMsg);
    return r ?? true;
  } catch (e) {
    toast(e.message, 'err');
    return false;
  } finally {
    if (btn) btn.disabled = false;
  }
}

function modal(title, body, { ok = 'OK', danger = false, submit, wide = false }) {
  return new Promise((resolve) => {
    const err = h('p', { class: 'form-err', role: 'alert' });
    const okBtn = h('button', { type: 'submit', class: danger ? 'btn danger' : 'btn primary' }, ok);
    const dlg = h('dialog', { class: `modal${wide ? ' wide' : ''}` });
    const form = h('form', {},
      h('h3', {}, title), body, err,
      h('div', { class: 'actions' },
        h('button', { type: 'button', class: 'btn', onclick: () => dlg.close() }, submit ? 'Hủy' : 'Đóng'),
        submit && okBtn));
    let result = false;
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      okBtn.disabled = true;
      err.textContent = '';
      try {
        result = (await submit(new FormData(form))) ?? true;
        dlg.close();
      } catch (ex) {
        err.textContent = ex.message;
      } finally {
        okBtn.disabled = false;
      }
    });
    dlg.addEventListener('close', () => { dlg.remove(); resolve(result); });
    dlg.append(form);
    document.body.append(dlg);
    dlg.showModal();
    const first = form.querySelector('input:not([readonly]):not([type=checkbox]), select');
    if (first) first.focus();
  });
}
const confirmDialog = (title, text, okLabel, fn) => modal(title, [h('p', {}, text)], { ok: okLabel, danger: true, submit: fn });

const field = (label, input, hint) => h('label', { class: 'field' }, h('span', {}, label), input, hint && h('small', {}, hint));
const check = (name, label, checked, value) =>
  h('label', { class: 'check' }, h('input', { type: 'checkbox', name, checked, value: value ?? 'on' }), h('span', {}, label));
const select = (name, options, current) =>
  h('select', { name }, options.map(([v, l]) => h('option', { value: v, selected: String(v) === String(current) }, l)));
function panel(title, body, tools, ico) {
  return h('section', { class: 'panel' },
    h('header', {}, h('h2', {}, ico && icon(ico), title), tools && h('div', { class: 'tools' }, tools)),
    h('div', { class: 'body' }, body));
}
const kvList = (pairs) => h('dl', { class: 'kv' }, pairs.filter(Boolean).map(([k, v]) => [h('dt', {}, k), h('dd', {}, v)]));
function pageHead(title, sub, tools) {
  return h('div', { class: 'page-head' }, h('div', {}, h('h1', {}, title), sub && h('p', {}, sub)), tools && h('div', { class: 'tools' }, tools));
}
function meter(p, cls = '') {
  p = Math.min(100, Math.max(0, p || 0));
  const bar = h('span');
  bar.style.width = `${p}%`;
  return h('div', { class: `meter ${cls || (p >= 90 ? 'crit' : p >= 75 ? 'warn' : '')}` }, bar);
}
function gauge(p, label, cls = 'gauge') {
  p = Math.min(100, Math.max(0, p || 0));
  const r = 26, c = 2 * Math.PI * r;
  const g = svg('svg', { class: `${cls} ${tone(p)}`, viewBox: '0 0 64 64', role: 'img', 'aria-label': `${Math.round(p)}%` });
  g.append(svg('circle', { class: 'track', cx: 32, cy: 32, r }));
  g.append(svg('circle', { class: 'bar', cx: 32, cy: 32, r, 'stroke-dasharray': `${(p / 100) * c} ${c}`, transform: 'rotate(-90 32 32)' }));
  const t = svg('text', { x: 32, y: 37, 'text-anchor': 'middle' });
  t.textContent = label ?? `${Math.round(p)}%`;
  g.append(t);
  return g;
}
function table(headers, rows, empty = 'Không có dữ liệu') {
  if (!rows.length) return h('p', { class: 'muted' }, empty);
  return h('div', { class: 'table-wrap' },
    h('table', {},
      h('thead', {}, h('tr', {}, headers.map((x) => h('th', {}, x)))),
      h('tbody', {}, rows.map((r) => h('tr', {}, r.map((c) => h('td', {}, c)))))));
}
function sparkChart(series, max, fmt, names, tall = false) {
  const W = 300, H = tall ? 140 : 100;
  const n = series[0].length;
  if (n < 2) return h('p', { class: 'muted' }, 'Đang thu thập dữ liệu…');
  const top = max ?? Math.max(1, ...series.flat());
  const s = svg('svg', { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: 'none', role: 'img' });
  for (const y of [0.25, 0.5, 0.75]) s.append(svg('line', { class: 'grid-line', x1: 0, x2: W, y1: H * y, y2: H * y }));
  series.forEach((pts, i) => {
    const points = pts.map((v, j) => `${((j / (n - 1)) * W).toFixed(1)},${(H - 2 - (v / top) * (H - 4)).toFixed(1)}`).join(' ');
    s.append(svg('polygon', { class: `a${i}`, points: `0,${H} ${points} ${W},${H}` }));
    s.append(svg('polyline', { class: `s${i}`, points }));
  });
  return h('div', { class: `chart${tall ? ' tall' : ''}` }, s, h('div', { class: 'legend' },
    series.map((pts, i) => h('span', {}, h('i', { class: `s${i}` }), names ? `${names[i]}: ` : '', h('b', {}, fmt(pts[n - 1])))),
    h('span', {}, `đỉnh trục: ${fmt(top)}`)));
}
function stat({ ico, color = '', label, value, sub, gaugePct, gaugeLabel, href }) {
  return h(href ? 'a' : 'div', { class: 'stat', href },
    h('span', { class: `chip ${color}` }, icon(ico, 'ico lg')),
    h('div', { class: 'stat-main' }, h('div', { class: 'stat-label' }, label), h('div', { class: 'stat-value' }, value), h('div', { class: 'stat-sub' }, sub)),
    gaugePct != null && gauge(gaugePct, gaugeLabel));
}
const alertChip = (a) => h('span', { class: `chip ${a.level === 'crit' ? 'red' : a.level === 'warn' ? 'orange' : 'gray'}` }, icon(a.level === 'info' ? 'info' : 'alert'));

// ---- shell: navigation, notifications, user menu ---------------------------
const NAV = [
  { items: [['overview', 'Bảng điều khiển', 'dashboard'], ['monitor', 'Giám sát tài nguyên', 'monitor']] },
  { title: 'Lưu trữ', items: [['storage', 'Quản lý ổ đĩa', 'storage'], ['files', 'Trình quản lý tệp', 'folder'], ['shares', 'Thư mục chia sẻ', 'share']] },
  { title: 'Mạng & bảo mật', items: [['network', 'Mạng', 'network'], ['security', 'Bảo mật', 'shield']] },
  { title: 'Hệ thống', items: [['update', 'Cập nhật', 'update'], ['backup', 'Sao lưu & khôi phục', 'backup'], ['services', 'Dịch vụ & nguồn', 'services'], ['logs', 'Nhật ký', 'logs']] },
];

function buildShell() {
  const side = $('#sidebar');
  side.replaceChildren(...NAV.map((g) => h('div', { class: 'nav-group' },
    g.title && h('div', { class: 'nav-title' }, g.title),
    g.items.map(([id, label, ico]) => h('a', { href: `#/${id}`, 'data-page': id, onclick: () => side.classList.remove('open') }, icon(ico), label)))));
  $('#menu-toggle').replaceChildren(icon('menu', 'ico lg'));
  $('#menu-toggle').onclick = () => side.classList.toggle('open');
  const themeBtn = $('#theme-toggle');
  const paintTheme = () => themeBtn.replaceChildren(icon(document.documentElement.dataset.theme === 'dark' ? 'sun' : 'moon', 'ico lg'));
  paintTheme();
  themeBtn.onclick = () => {
    const next = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
    try { localStorage.setItem('hehenas-theme', next); } catch { /* private mode */ }
    applyTheme(next);
    paintTheme();
  };
  $('#notif-btn').prepend(icon('bell', 'ico lg'));
  $('#notif-btn').onclick = (e) => { e.stopPropagation(); toggleMenu('#notif-menu'); };
  $('#user-btn').onclick = (e) => { e.stopPropagation(); toggleMenu('#user-menu'); };
  $('#user-menu').replaceChildren(
    h('div', { class: 'menu-head' }, 'Tài khoản'),
    h('a', { class: 'menu-item', href: '#/security' }, icon('key'), 'Đổi mật khẩu'),
    h('a', { class: 'menu-item', href: '#/services' }, icon('power'), 'Khởi động lại / tắt máy'),
    h('button', { class: 'menu-item', type: 'button', onclick: logout }, icon('logout'), 'Đăng xuất'));
  document.addEventListener('click', () => { $('#notif-menu').hidden = true; $('#user-menu').hidden = true; });
}
function toggleMenu(sel) {
  const m = $(sel);
  const open = m.hidden;
  $('#notif-menu').hidden = true;
  $('#user-menu').hidden = true;
  m.hidden = !open;
}

async function refreshNotifications() {
  try {
    const list = await api('/api/alerts');
    const bad = list.filter((a) => a.level !== 'info');
    const badge = $('#notif-badge');
    badge.hidden = !bad.length;
    badge.textContent = bad.length;
    $('#notif-menu').replaceChildren(
      h('div', { class: 'menu-head' }, 'Thông báo'),
      list.length ? list.map((a) => h('a', { class: 'notif-item', href: `#/${a.page || 'overview'}` }, alertChip(a), h('div', {}, h('strong', {}, a.title), h('small', {}, a.detail))))
        : h('p', { class: 'muted pad center' }, 'Không có thông báo nào. Mọi thứ đều ổn.'));
    return list;
  } catch { return []; }
}

async function logout() {
  try { await api('/api/auth/logout', { method: 'POST' }); } catch { /* ignore */ }
  showAuth(false);
}

// ---- auth -----------------------------------------------------------------
function showAuth(setup) {
  seq++;
  timers.forEach(clearInterval);
  timers = [];
  $('#app').hidden = true;
  const box = $('#auth');
  box.hidden = false;
  const err = h('p', { class: 'form-err', role: 'alert' });
  const form = h('form', { class: 'auth-card' },
    h('div', { class: 'logo' }, h('img', { src: '/favicon.svg', alt: '', width: 34, height: 34 }), h('span', {}, 'HeheNAS'), h('span', { class: 'brand-os' }, 'OS')),
    h('h1', {}, setup ? 'Chào mừng!' : 'Đăng nhập'),
    h('p', { class: 'muted' }, setup ? 'Tạo tài khoản quản trị để bắt đầu. Người đầu tiên mở trang này sẽ thành quản trị viên.' : `Đăng nhập vào ${status.hostname || 'NAS'}`),
    !status.secure && !status.demo && h('div', { class: 'alert warn' }, icon('alert'), 'Kết nối chưa mã hóa. Hãy dùng https://… nếu có thể.'),
    field('Tên đăng nhập', h('input', { name: 'username', autocomplete: 'username', required: true, value: setup ? 'admin' : '' })),
    field('Mật khẩu', h('input', { name: 'password', type: 'password', required: true, minlength: setup ? 8 : null, autocomplete: setup ? 'new-password' : 'current-password' })),
    setup && field('Nhập lại mật khẩu', h('input', { name: 'password2', type: 'password', required: true, autocomplete: 'new-password' })),
    err,
    h('button', { class: 'btn primary block', type: 'submit' }, setup ? 'Tạo tài khoản' : 'Đăng nhập'),
    status.demo && h('p', { class: 'muted center' }, 'Bản xem thử: admin / hehenas123'));
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(form);
    if (setup && fd.get('password') !== fd.get('password2')) { err.textContent = 'Mật khẩu nhập lại không khớp'; return; }
    const btn = form.querySelector('button[type=submit]');
    btn.disabled = true;
    err.textContent = '';
    try {
      const r = await api(setup ? '/api/auth/setup' : '/api/auth/login', { method: 'POST', body: { username: fd.get('username'), password: fd.get('password') } });
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
  $('#user-avatar').textContent = (user || 'A').slice(0, 1).toUpperCase();
  route();
  refreshNotifications();
}

// ---- routing --------------------------------------------------------------
const pages = {
  overview: pageOverview, monitor: pageMonitor, storage: pageStorage, files: pageFiles, shares: pageShares,
  network: pageNetwork, security: pageSecurity, update: pageUpdate, backup: pageBackup, services: pageServices, logs: pageLogs,
};

function route() {
  const my = ++seq;
  timers.forEach(clearInterval);
  timers = [];
  $('#notif-menu').hidden = true;
  $('#user-menu').hidden = true;
  const [id, query = ''] = location.hash.replace(/^#\/?/, '').split('?');
  const page = pages[id] ? id : 'overview';
  document.querySelectorAll('.sidebar a').forEach((a) => a.classList.toggle('active', a.dataset.page === page));
  view.replaceChildren(h('p', { class: 'muted pad' }, 'Đang tải…'));
  const ctx = {
    params: new URLSearchParams(query),
    show: (...nodes) => { if (my === seq) view.replaceChildren(...nodes.flat().filter(Boolean)); },
    every: (fn, ms) => { if (my === seq) timers.push(setInterval(() => fn().catch(() => {}), ms)); },
  };
  pages[page](ctx).catch((e) => {
    if (my === seq && !$('#app').hidden) view.replaceChildren(h('div', { class: 'alert crit' }, e.message));
  });
}
window.addEventListener('hashchange', () => { if (!location.hash.startsWith('#/confirm')) route(); });

// ---- Bảng điều khiển ----------------------------------------------------------
function volumeList(disks) {
  const data = disks.filter((d) => !d.system).flatMap((d) => d.partitions.map((p) => ({ d, p }))).filter(({ p }) => p.mountpoint && p.total_bytes);
  if (!data.length) return h('div', {}, h('p', { class: 'muted' }, 'Chưa có ổ dữ liệu nào được mount.'), h('a', { class: 'btn small primary', href: '#/storage' }, 'Mở Quản lý ổ đĩa'));
  return h('div', {}, data.map(({ d, p }) => {
    const u = pct(p.used_bytes, p.total_bytes);
    return h('div', { class: 'volume' },
      gauge(u, `${u}%`, 'gauge donut'),
      h('div', { class: 'volume-info' },
        h('strong', {}, p.label || p.name), ' ', tag(d.transport.toUpperCase(), 'accent'), ' ', tag(p.fstype || '?'),
        h('div', { class: 'meta' }, h('code', {}, p.mountpoint), ` · ${d.model || d.name}`),
        meter(u),
        h('div', { class: 'meta' }, `Đã dùng ${fmtBytes(p.used_bytes)} · còn trống ${fmtBytes(p.total_bytes - p.used_bytes)} · tổng ${fmtBytes(p.total_bytes)}`)));
  }));
}

async function pageOverview({ show, every }) {
  const [o, hist, d, al, svcs, logs, upd, net] = await Promise.all([
    api('/api/system/overview'), api('/api/system/history'), api('/api/storage/disks'), api('/api/alerts'),
    api('/api/system/services'), api('/api/system/logs?source=hehenasd').catch(() => ({ lines: [] })),
    api('/api/system/update').catch(() => ({})), api('/api/network').catch(() => ({ info: { interfaces: [] } })),
  ]);
  const level = al.some((a) => a.level === 'crit') ? 'crit' : al.some((a) => a.level === 'warn') ? 'warn' : 'ok';
  const heroText = {
    ok: ['Hệ thống hoạt động tốt', 'Không có cảnh báo nào cần xử lý.'],
    warn: ['Cần chú ý', `${al.filter((a) => a.level === 'warn').length} cảnh báo đang chờ bạn xem.`],
    crit: ['Có sự cố nghiêm trọng', 'Hãy xem thông báo ngay.'],
  }[level];
  const ip = net.info.interfaces.flatMap((i) => i.ipv4)[0];
  const hero = h('div', { class: `hero ${level === 'ok' ? '' : level}` },
    h('div', { class: 'hero-icon' }, icon(level === 'ok' ? 'shieldok' : 'alert', 'ico xl')),
    h('div', {}, h('h2', {}, heroText[0]), h('p', {}, heroText[1])),
    h('div', { class: 'hero-stats' },
      h('div', {}, h('small', {}, 'Tên máy'), h('strong', {}, o.hostname)),
      h('div', {}, h('small', {}, 'Địa chỉ IP'), h('strong', {}, ip ? ip.split('/')[0] : '—')),
      h('div', {}, h('small', {}, 'Thời gian chạy'), h('strong', {}, fmtUptime(o.uptime_secs))),
      h('div', {}, h('small', {}, 'Phiên bản'), h('strong', {}, `HeheNAS ${o.version}`))));

  const stats = h('div', { class: 'grid cols-4' });
  const cpuChart = h('div'), netChart = h('div');
  const draw = (s, hh) => {
    const memUsed = s.mem_total - s.mem_available;
    const netPct = (s.net_rx_bps + s.net_tx_bps) / 125000;
    stats.replaceChildren(
      stat({ ico: 'cpu', label: 'CPU', value: `${s.cpu_percent.toFixed(0)}%`, sub: `${s.cpu_cores} nhân · tải ${s.load[0].toFixed(2)}`, gaugePct: s.cpu_percent, href: '#/monitor' }),
      stat({ ico: 'mem', color: 'teal', label: 'Bộ nhớ', value: fmtBytes(memUsed), sub: `trên ${fmtBytes(s.mem_total)}`, gaugePct: pct(memUsed, s.mem_total), href: '#/monitor' }),
      stat({ ico: 'temp', color: 'orange', label: 'Nhiệt độ CPU', value: s.temp_c != null ? `${s.temp_c.toFixed(1)} °C` : '—', sub: 'hạ xung ở 80 °C', gaugePct: s.temp_c != null ? (s.temp_c / 85) * 100 : null, gaugeLabel: s.temp_c != null ? `${Math.round(s.temp_c)}°` : '' }),
      stat({ ico: 'net', color: 'green', label: 'Mạng', value: `↓ ${fmtRate(s.net_rx_bps)}`, sub: `↑ ${fmtRate(s.net_tx_bps)}`, gaugePct: Math.min(100, netPct), gaugeLabel: `${Math.round(netPct)}%` }));
    cpuChart.replaceChildren(sparkChart([hh.map((p) => p.cpu)], 100, (v) => `${v.toFixed(0)}%`, ['CPU']));
    netChart.replaceChildren(sparkChart([hh.map((p) => p.rx), hh.map((p) => p.tx)], null, fmtRate, ['Nhận', 'Gửi']));
  };
  draw(o, hist);

  const services = h('div', { class: 'list' }, svcs.map((s) => h('div', { class: 'list-row' },
    h('span', { class: `status-dot ${s.running ? 'ok' : 'err'}` }),
    h('div', { class: 'grow' }, s.name, h('small', {}, s.running ? `RAM ${fmtBytes(s.rss_bytes)}` : 'đã dừng')))));
  const alertsBox = al.length
    ? h('div', { class: 'list' }, al.map((a) => h('a', { class: 'list-row notif-item', href: `#/${a.page || 'overview'}` }, alertChip(a), h('div', { class: 'grow' }, h('strong', {}, a.title), h('small', {}, a.detail)))))
    : h('div', { class: 'list-row' }, h('span', { class: 'chip green' }, icon('shieldok')), h('div', { class: 'grow' }, 'Không có thông báo', h('small', {}, 'Mọi thứ đều ổn.')));
  const on = upd.online || {};
  const updBox = h('div', {},
    kvList([
      ['Đang chạy', `v${o.version}`],
      ['Bản mới nhất', on.latest ? `v${on.latest}` : 'chưa kiểm tra'],
      ['Tự động', { install: 'Tự cài + khởi động lại', notify: 'Chỉ thông báo', off: 'Tắt' }[upd.auto_update] || '—'],
    ]),
    h('div', { class: 'row-actions left' }, on.available ? h('a', { class: 'btn small primary', href: '#/update' }, `Cập nhật lên v${on.latest}`) : h('a', { class: 'btn small', href: '#/update' }, 'Quản lý cập nhật')));
  const logText = (logs.lines || []).slice(-8).map((l) => l.replace(/^\S+\s+\S+\s+\S+\s+\S+\s+\S+\s+/, '')).join('\n') || '(trống)';

  show(
    pageHead('Bảng điều khiển', `Chào mừng trở lại, ${$('#user-name').textContent || 'admin'}.`),
    h('div', { class: 'dash' },
      h('div', { class: 'w-8' }, hero, h('div', { class: 'mb' }), stats),
      h('section', { class: 'panel w-4' }, h('header', {}, h('h2', {}, icon('bell'), 'Thông báo')), h('div', { class: 'body' }, alertsBox)),
      h('div', { class: 'w-6' }, panel('Tải CPU (10 phút)', cpuChart, h('a', { class: 'btn small', href: '#/monitor' }, 'Chi tiết'), 'cpu')),
      h('div', { class: 'w-6' }, panel('Lưu lượng mạng', netChart, null, 'net')),
      h('div', { class: 'w-6' }, panel('Ổ lưu trữ', volumeList(d.disks), h('a', { class: 'btn small', href: '#/storage' }, 'Quản lý'), 'storage')),
      h('div', { class: 'w-3' }, panel('Dịch vụ', services, null, 'services')),
      h('div', { class: 'w-3' }, panel('Cập nhật', updBox, null, 'update')),
      h('div', { class: 'w-6' }, panel('Thông tin hệ thống', kvList([
        ['Phần cứng', o.model || '—'], ['Kernel', o.kernel], ['Bộ nhớ đệm', fmtBytes(o.mem_cached)],
        ['Swap (zram)', o.swap_total ? `${fmtBytes(o.swap_total - o.swap_free)} / ${fmtBytes(o.swap_total)}` : 'tắt'],
        ['RAM của hehenasd', fmtBytes(o.daemon_rss)],
      ]), null, 'info')),
      h('div', { class: 'w-6' }, panel('Hoạt động gần đây', h('pre', { class: 'log-mini' }, logText), h('a', { class: 'btn small', href: '#/logs' }, 'Xem nhật ký'), 'logs'))),
  );
  every(async () => {
    const [s, hh] = await Promise.all([api('/api/system/overview'), api('/api/system/history')]);
    draw(s, hh);
  }, 3000);
  every(refreshNotifications, 30000);
}

// ---- Giám sát tài nguyên -----------------------------------------------------
async function pageMonitor({ show, every }) {
  const body = h('div');
  const draw = (s, hist) => {
    const memUsed = s.mem_total - s.mem_available;
    const swapUsed = s.swap_total - s.swap_free;
    const cores = s.cores || [];
    const disks = s.disks || [];
    const segbar = h('div', { class: 'segbar' });
    const a = h('span', { class: 'seg-used' });
    a.style.width = `${pct(memUsed, s.mem_total)}%`;
    const b = h('span', { class: 'seg-cache' });
    b.style.width = `${pct(Math.min(s.mem_cached, s.mem_available), s.mem_total)}%`;
    segbar.append(a, b);
    body.replaceChildren(
      h('div', { class: 'grid cols-4 mb' },
        stat({ ico: 'cpu', label: 'CPU', value: `${s.cpu_percent.toFixed(1)}%`, sub: `tải ${s.load.map((x) => x.toFixed(2)).join(' · ')}`, gaugePct: s.cpu_percent }),
        stat({ ico: 'mem', color: 'teal', label: 'Bộ nhớ', value: fmtBytes(memUsed), sub: `${pct(memUsed, s.mem_total)}% của ${fmtBytes(s.mem_total)}`, gaugePct: pct(memUsed, s.mem_total) }),
        stat({ ico: 'temp', color: 'orange', label: 'Nhiệt độ', value: s.temp_c != null ? `${s.temp_c.toFixed(1)} °C` : '—', sub: 'Pi 3B hạ xung ở 80 °C', gaugePct: s.temp_c != null ? (s.temp_c / 85) * 100 : null, gaugeLabel: s.temp_c != null ? `${Math.round(s.temp_c)}°` : '' }),
        stat({ ico: 'storage', color: 'green', label: 'Ổ đĩa (đọc)', value: fmtRate(disks.reduce((x, y) => x + y.read_bps, 0)), sub: `ghi ${fmtRate(disks.reduce((x, y) => x + y.write_bps, 0))}` })),
      h('div', { class: 'grid cols-2 mb' },
        panel('CPU', [
          sparkChart([hist.map((p) => p.cpu)], 100, (v) => `${v.toFixed(0)}%`, ['Tổng'], true),
          cores.length > 0 && h('div', { class: 'bars' }, h('div', { class: 'muted pad' }, 'Từng nhân'),
            cores.map((c, i) => h('div', { class: 'bar-row' }, h('span', {}, `Nhân ${i}`), meter(c, 'accent'), h('span', {}, `${c.toFixed(0)}%`)))),
        ], null, 'cpu'),
        panel('Bộ nhớ', [
          sparkChart([hist.map((p) => p.mem)], s.mem_total, fmtBytes, ['Đang dùng'], true),
          h('div', { class: 'muted pad' }, 'Phân bổ RAM'),
          segbar,
          h('div', { class: 'legend' },
            h('span', {}, h('i', { class: 's0' }), 'Đang dùng ', h('b', {}, fmtBytes(memUsed))),
            h('span', {}, h('i', { class: 's1' }), 'Bộ đệm ', h('b', {}, fmtBytes(s.mem_cached))),
            h('span', {}, h('i', { class: 's2' }), 'Swap zram ', h('b', {}, `${fmtBytes(swapUsed)} / ${fmtBytes(s.swap_total)}`))),
        ], null, 'mem')),
      h('div', { class: 'grid cols-2' },
        panel('Mạng', sparkChart([hist.map((p) => p.rx), hist.map((p) => p.tx)], null, fmtRate, ['Nhận', 'Gửi'], true), null, 'net'),
        panel('Ổ đĩa', [
          sparkChart([hist.map((p) => p.dr || 0), hist.map((p) => p.dw || 0)], null, fmtRate, ['Đọc', 'Ghi'], true),
          table(['Ổ', 'Đọc', 'Ghi'], disks.map((x) => [h('code', {}, x.name), fmtRate(x.read_bps), fmtRate(x.write_bps)]), 'Không có ổ nào'),
        ], null, 'storage')));
  };
  const [s, hist] = await Promise.all([api('/api/system/overview'), api('/api/system/history')]);
  draw(s, hist);
  show(pageHead('Giám sát tài nguyên', 'Cập nhật mỗi 3 giây · lịch sử 10 phút'), body);
  every(async () => {
    const [s2, h2] = await Promise.all([api('/api/system/overview'), api('/api/system/history')]);
    draw(s2, h2);
  }, 3000);
}

// ---- Quản lý ổ đĩa -----------------------------------------------------------
function smartTag(s) {
  if (!s) return tag('SMART: chưa đọc');
  if (!s.available) return tag('SMART: không hỗ trợ');
  if (s.passed === false) return tag('SMART: LỖI', 'err');
  const bad = (s.reallocated || 0) + (s.pending || 0) + (s.uncorrectable || 0);
  return bad ? tag(`SMART: ${bad} sector lỗi`, 'warn') : tag('SMART: tốt', 'ok');
}

async function smartDialog(disk) {
  const box = h('div', {}, h('p', { class: 'muted' }, 'Đang đọc SMART…'));
  const p = modal(`SMART — ${disk}`, [box], {});
  try {
    const s = await api(`/api/storage/smart/${encodeURIComponent(disk)}`);
    box.replaceChildren(s.available ? kvList([
      ['Đánh giá', s.passed ? tag('PASSED', 'ok') : tag('FAILED', 'err')],
      ['Model', s.model || '—'], ['Serial', s.serial || '—'],
      ['Nhiệt độ', s.temperature_c != null ? `${s.temperature_c} °C` : '—'],
      ['Giờ hoạt động', s.power_on_hours != null ? `${s.power_on_hours} giờ (${(s.power_on_hours / 24).toFixed(0)} ngày)` : '—'],
      ['Sector đã thay (5)', s.reallocated ?? '—'], ['Sector chờ (197)', s.pending ?? '—'], ['Không sửa được (198)', s.uncorrectable ?? '—'],
      s.percentage_used != null && ['Mức hao mòn (NVMe)', `${s.percentage_used}%`],
    ]) : h('div', { class: 'alert warn' }, s.message || 'Không đọc được SMART.'));
  } catch (e) {
    box.replaceChildren(h('div', { class: 'alert crit' }, e.message));
  }
  return p;
}

function wipeDialog(kind, target, size, reload) {
  const isInit = kind === 'initialize';
  return modal(isInit ? `Khởi tạo ổ ${target}` : `Format ${target} thành ext4`, [
    h('div', { class: 'alert crit' }, icon('alert'), isInit
      ? `Toàn bộ ổ /dev/${target} (${fmtBytes(size)}) sẽ bị xóa: tạo bảng phân vùng GPT mới với một phân vùng ext4.`
      : `Toàn bộ dữ liệu trên /dev/${target} (${fmtBytes(size)}) sẽ bị xóa vĩnh viễn.`),
    field('Nhãn ổ', h('input', { name: 'label', maxlength: 16, pattern: '[A-Za-z0-9_]*', value: 'data' }), 'Ổ sẽ được mount tại /mnt/<nhãn>.'),
    field(`Gõ "${target}" để xác nhận`, h('input', { name: 'confirm', required: true, autocomplete: 'off' })),
  ], {
    ok: isInit ? 'Khởi tạo' : 'Format', danger: true,
    submit: (fd) => api(`/api/storage/${kind}`, { method: 'POST', body: { device: target, label: fd.get('label') || '', confirm: fd.get('confirm') } }),
  }).then((ok) => { if (ok) { toast(`Đã ${isInit ? 'khởi tạo' : 'format'} ${target}`); reload(); } });
}

function partActions(p, reload) {
  const post = (path, body, msg) => (e) => act(e.currentTarget, () => api(path, { method: 'POST', body }), msg).then((ok) => ok && reload());
  if (p.mountpoint) {
    return h('div', { class: 'row-actions' },
      h('a', { class: 'btn small', href: `#/files?path=${encodeURIComponent(p.mountpoint)}` }, 'Mở'),
      h('button', { class: 'btn small', title: 'Giao toàn bộ file cho tài khoản chia sẻ (ổ mang từ máy khác sang)', onclick: post('/api/storage/fix-owner', { path: p.mountpoint }, 'Đã sửa quyền') }, 'Sửa quyền'),
      h('button', { class: 'btn small', onclick: post('/api/storage/unmount', { device: p.name }, `Đã tháo ${p.name}`) }, 'Tháo'));
  }
  return h('div', { class: 'row-actions' },
    p.fstype && p.fstype !== 'swap' && h('button', { class: 'btn small primary', onclick: post('/api/storage/mount', { device: p.name }, `Đã mount ${p.name}`) }, 'Mount'),
    h('button', { class: 'btn small danger', onclick: () => wipeDialog('format', p.name, p.size_bytes, reload) }, 'Format'));
}

async function pageStorage({ show, every }) {
  const { disks, smart } = await api('/api/storage/disks');
  const reload = () => route();
  const dataDisks = disks.filter((d) => !d.system);
  const total = dataDisks.reduce((x, d) => x + d.size_bytes, 0);
  const mounted = dataDisks.flatMap((d) => d.partitions).filter((p) => p.mountpoint && p.total_bytes);
  const usedAll = mounted.reduce((x, p) => x + p.used_bytes, 0), capAll = mounted.reduce((x, p) => x + p.total_bytes, 0);
  const blocks = disks.map((d) => panel(
    `${d.name} · ${d.model || 'Không rõ model'}`,
    table(['Phân vùng', 'Filesystem', 'Nhãn', 'Dung lượng', 'Đã dùng', 'Mount', ''], d.partitions.map((p) => [
      h('code', {}, p.name),
      p.fstype ? tag(p.fstype, 'accent') : h('span', { class: 'muted' }, 'không rõ'),
      p.label || '—',
      fmtBytes(p.size_bytes),
      p.total_bytes ? h('div', { class: 'usage' }, meter(pct(p.used_bytes, p.total_bytes)), h('small', {}, `${fmtBytes(p.used_bytes)} / ${fmtBytes(p.total_bytes)}`)) : '—',
      p.mountpoint ? h('code', {}, p.mountpoint) : h('span', { class: 'muted' }, 'chưa mount'),
      d.system ? tag('hệ thống') : partActions(p, reload),
    ]), 'Ổ chưa có phân vùng nào. Bấm "Khởi tạo" để chuẩn bị ổ.'),
    [
      h('span', { class: 'tags' }, tag(d.transport.toUpperCase()), tag(fmtBytes(d.size_bytes)), tag(d.rotational ? 'HDD' : 'SSD/Flash'),
        d.system ? tag('Thẻ hệ thống', 'warn') : smartTag(smart[d.name])),
      !d.system && h('button', { class: 'btn small', onclick: () => smartDialog(d.name) }, 'SMART'),
      !d.system && h('button', { class: 'btn small danger', onclick: () => wipeDialog('initialize', d.name, d.size_bytes, reload) }, 'Khởi tạo'),
    ], d.system ? 'lock' : 'storage'));
  const smartList = Object.values(smart);
  show(
    pageHead('Quản lý ổ đĩa', 'Ổ USB chứa dữ liệu; thẻ nhớ chỉ chứa hệ điều hành. Ổ đã mount sẽ tự mount lại sau khi khởi động.', h('button', { class: 'btn', onclick: reload }, icon('refresh'), 'Làm mới')),
    h('div', { class: 'grid cols-4 mb' },
      stat({ ico: 'storage', label: 'Ổ dữ liệu', value: String(dataDisks.length), sub: `tổng ${fmtBytes(total)}` }),
      stat({ ico: 'folder', color: 'teal', label: 'Đang mount', value: String(mounted.length), sub: mounted.map((p) => p.mountpoint).join(', ') || 'chưa có' }),
      stat({ ico: 'monitor', color: 'orange', label: 'Đã dùng', value: fmtBytes(usedAll), sub: `trên ${fmtBytes(capAll)}`, gaugePct: capAll ? pct(usedAll, capAll) : null }),
      stat({ ico: 'shieldok', color: smartList.some((s) => s.passed === false) ? 'red' : 'green', label: 'Sức khỏe SMART', value: smartList.some((s) => s.passed === false) ? 'Có lỗi' : 'Tốt', sub: `${smartList.filter((s) => s.available).length} ổ hỗ trợ SMART` })),
    blocks.length ? h('div', { class: 'grid disks' }, blocks) : h('div', { class: 'alert' }, 'Không tìm thấy ổ đĩa nào.'),
  );
  const sig = (ds) => JSON.stringify(ds.map((x) => [x.name, x.partitions.map((p) => [p.name, p.fstype, p.mountpoint])]));
  const before = sig(disks);
  every(async () => {
    const r = await api('/api/storage/disks');
    if (sig(r.disks) !== before && !document.querySelector('dialog[open]')) reload();
  }, 8000);
}

// ---- Trình quản lý tệp --------------------------------------------------------
async function pageFiles({ show, params }) {
  const path = params.get('path') || '';
  const go = (p) => { location.hash = p ? `#/files?path=${encodeURIComponent(p)}` : '#/files'; };
  const reload = () => route();
  if (params.get('trash')) return pageTrash({ show });

  const data = await api(`/api/files?path=${encodeURIComponent(path)}`);
  if (data.roots) {
    show(
      pageHead('Trình quản lý tệp', 'Duyệt, tải lên, tải xuống và quản lý file trên các ổ dữ liệu.', h('a', { class: 'btn', href: '#/files?trash=1' }, icon('trash'), 'Thùng rác')),
      data.roots.length
        ? h('div', { class: 'grid cols-3' }, data.roots.map((r) => h('a', { class: 'stat', href: `#/files?path=${encodeURIComponent(r.path)}` },
          h('span', { class: 'chip' }, icon('storage', 'ico lg')),
          h('div', { class: 'stat-main' }, h('div', { class: 'stat-label' }, 'Ổ dữ liệu'), h('div', { class: 'stat-value' }, r.name), meter(pct(r.used, r.total)),
            h('div', { class: 'stat-sub' }, `Còn trống ${fmtBytes(r.avail)} / ${fmtBytes(r.total)}`)))))
        : h('div', { class: 'alert' }, 'Chưa có ổ dữ liệu nào được mount. Vào ', h('a', { href: '#/storage' }, 'Quản lý ổ đĩa'), ' để mount.'),
    );
    return;
  }

  const crumbs = [h('a', { href: '#/files' }, 'Ổ dữ liệu')];
  let acc = data.mount;
  crumbs.push('›', h('a', { href: `#/files?path=${encodeURIComponent(acc)}` }, baseName(data.mount)));
  for (const seg of data.path.slice(data.mount.length).split('/').filter(Boolean)) {
    acc = joinPath(acc, seg);
    crumbs.push('›', h('a', { href: `#/files?path=${encodeURIComponent(acc)}` }, seg));
  }

  const selected = new Set();
  const delLabel = h('span', {}, 'Xóa');
  const delBtn = h('button', { class: 'btn danger', disabled: true }, icon('trash'), delLabel);
  const updateSel = () => { delBtn.disabled = !selected.size; delLabel.textContent = selected.size ? `Xóa (${selected.size})` : 'Xóa'; };
  delBtn.addEventListener('click', () => confirmDialog('Chuyển vào thùng rác', `Chuyển ${selected.size} mục vào thùng rác? Có thể khôi phục trong 30 ngày.`, 'Xóa',
    () => api('/api/files/delete', { method: 'POST', body: { paths: [...selected] } })).then((ok) => { if (ok) { toast('Đã chuyển vào thùng rác'); reload(); } }));

  const fileInput = h('input', { type: 'file', multiple: true, hidden: true });
  const progress = h('div', { class: 'upload-progress' });
  fileInput.addEventListener('change', async () => {
    const files = [...fileInput.files];
    fileInput.value = '';
    for (const f of files) {
      const row = h('div', {}, h('small', {}, f.name), meter(0, 'accent'));
      progress.append(row);
      const url = `/api/files/upload?dir=${encodeURIComponent(data.path)}&name=${encodeURIComponent(f.name)}`;
      try {
        await uploadFile(url, f, (x) => row.replaceChildren(h('small', {}, `${f.name} — ${Math.round(x * 100)}%`), meter(x * 100, 'accent')));
        row.remove();
      } catch (e) {
        if (/đã tồn tại/.test(e.message) && confirm(`"${f.name}" đã tồn tại. Ghi đè?`)) {
          await uploadFile(`${url}&overwrite=1`, f).then(() => row.remove()).catch((e2) => { toast(e2.message, 'err'); row.remove(); });
        } else {
          toast(`${f.name}: ${e.message}`, 'err');
          row.remove();
        }
      }
    }
    toast('Tải lên xong');
    reload();
  });
  const mkdirBtn = h('button', { class: 'btn', onclick: () => modal('Thư mục mới', [field('Tên', h('input', { name: 'name', required: true, maxlength: 255 }))], {
    ok: 'Tạo', submit: (fd) => api('/api/files/mkdir', { method: 'POST', body: { path: data.path, name: fd.get('name') } }),
  }).then((ok) => ok && reload()) }, icon('plus'), 'Thư mục mới');

  const rows = data.entries.map((e) => {
    const full = joinPath(data.path, e.name);
    const cb = h('input', { type: 'checkbox', 'aria-label': `Chọn ${e.name}`, onchange: (ev) => { ev.target.checked ? selected.add(full) : selected.delete(full); updateSel(); } });
    const name = e.dir
      ? h('a', { href: `#/files?path=${encodeURIComponent(full)}`, class: 'file-name' }, h('span', { class: 'chip' }, icon('folder')), e.name)
      : h('a', { href: `/api/files/download?path=${encodeURIComponent(full)}`, class: 'file-name', download: e.name }, h('span', { class: 'chip gray' }, icon('file')), e.name);
    return [cb, h('span', {}, name, e.symlink && ' ', e.symlink && tag('link')), e.dir ? '—' : fmtBytes(e.size), fmtDate(e.mtime),
      h('div', { class: 'row-actions' },
        h('button', { class: 'btn small', onclick: () => modal(`Đổi tên ${e.name}`, [field('Tên mới', h('input', { name: 'name', value: e.name, required: true, maxlength: 255 }))], {
          ok: 'Đổi tên', submit: (fd) => api('/api/files/rename', { method: 'POST', body: { path: full, name: fd.get('name') } }),
        }).then((ok) => ok && reload()) }, 'Đổi tên'))];
  });

  show(
    pageHead('Trình quản lý tệp', null, [mkdirBtn, h('button', { class: 'btn primary', onclick: () => fileInput.click() }, icon('upload'), 'Tải lên'), delBtn,
      h('a', { class: 'btn', href: '#/files?trash=1' }, icon('trash'), 'Thùng rác')]),
    h('div', { class: 'crumbs' }, crumbs), fileInput, progress,
    panel(data.path, table(['', 'Tên', 'Kích thước', 'Sửa đổi', ''], [
      data.path !== data.mount && [h('span'), h('a', { href: '#', class: 'file-name', onclick: (ev) => { ev.preventDefault(); go(parentOf(data.path)); } }, '⬆ Lên thư mục cha'), '', '', ''],
      ...rows,
    ].filter(Boolean), 'Thư mục trống'), null, 'folder'),
  );
}

async function pageTrash({ show }) {
  const items = await api('/api/trash');
  const reload = () => route();
  const rows = items.map((it) => [
    h('span', { class: 'file-name' }, h('span', { class: 'chip gray' }, icon(it.dir ? 'folder' : 'file')), it.name), h('code', {}, it.original), it.dir ? '—' : fmtBytes(it.size), fmtDate(it.deleted_at),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: (e) => act(e.currentTarget, () => api('/api/trash/restore', { method: 'POST', body: { mount: it.mount, id: it.id } }), 'Đã khôi phục').then((ok) => ok && reload()) }, 'Khôi phục'),
      h('button', { class: 'btn small danger', onclick: () => confirmDialog('Xóa vĩnh viễn', `Xóa vĩnh viễn "${it.name}"? Không thể hoàn tác.`, 'Xóa vĩnh viễn',
        () => api('/api/trash/purge', { method: 'POST', body: { mount: it.mount, id: it.id } })).then((ok) => ok && reload()) }, 'Xóa hẳn')),
  ]);
  show(
    pageHead('Thùng rác', 'File xóa từ trình quản lý tệp được giữ 30 ngày. (File xóa qua SMB sẽ mất ngay — ksmbd không có thùng rác.)', [
      h('a', { class: 'btn', href: '#/files' }, '← Tệp'),
      items.length > 0 && h('button', { class: 'btn danger', onclick: () => confirmDialog('Dọn thùng rác', `Xóa vĩnh viễn ${items.length} mục?`, 'Dọn sạch',
        () => api('/api/trash/purge', { method: 'POST', body: {} })).then((ok) => ok && reload()) }, icon('trash'), 'Dọn sạch')]),
    panel('Đã xóa', table(['Tên', 'Vị trí cũ', 'Kích thước', 'Xóa lúc', ''], rows, 'Thùng rác trống'), null, 'trash'),
  );
}

// ---- Thư mục chia sẻ ---------------------------------------------------------
async function shareDialog(s, data, mounts, reload) {
  const editing = !!s;
  s = s || { name: '', path: mounts[0] ? `${mounts[0]}/share` : '/mnt/', comment: '', read_only: false, guest_ok: false, users: [], nfs: false, nfs_clients: '192.168.1.0/24' };
  const access = Object.fromEntries(s.users.map((u) => [u.name, u.access]));
  const ok = await modal(editing ? `Sửa chia sẻ ${s.name}` : 'Tạo thư mục chia sẻ', [
    field('Tên chia sẻ', h('input', { name: 'name', value: s.name, required: true, maxlength: 32, pattern: '[A-Za-z0-9_]+(-[A-Za-z0-9_]+)*', readonly: editing })),
    field('Thư mục', h('input', { name: 'path', value: s.path, required: true, list: 'mount-list' }), 'Nằm trên ổ đã mount, ví dụ /mnt/data/phim. Thư mục chưa có sẽ được tạo.'),
    h('datalist', { id: 'mount-list' }, mounts.map((m) => h('option', { value: m }))),
    field('Mô tả', h('input', { name: 'comment', value: s.comment, maxlength: 80 })),
    check('read_only', 'Mặc định chỉ đọc', s.read_only),
    check('guest_ok', 'Cho khách truy cập (không cần mật khẩu)', s.guest_ok),
    data.users.length
      ? h('fieldset', {}, h('legend', {}, 'Quyền theo người dùng (để "—" hết = mọi người dùng SMB)'),
        data.users.map((u) => h('div', { class: 'acl-row' }, h('span', {}, u), select(`acl_${u}`, [['', '—'], ['rw', 'Đọc + ghi'], ['ro', 'Chỉ đọc']], access[u] || ''))))
      : h('p', { class: 'muted' }, 'Chưa có người dùng SMB. Hãy tạo ở bảng "Người dùng SMB" hoặc bật quyền khách.'),
    h('fieldset', {}, h('legend', {}, 'NFS (Linux, TV box, máy ảo)'),
      check('nfs', 'Chia sẻ qua NFS', s.nfs),
      field('Mạng được phép', h('input', { name: 'nfs_clients', value: s.nfs_clients || '192.168.1.0/24' }), 'IP hoặc dải mạng, cách nhau bằng dấu cách.'),
      !data.nfs_available && h('div', { class: 'alert warn' }, 'Image này chưa có NFS server.')),
  ], {
    ok: editing ? 'Lưu' : 'Tạo', wide: true,
    submit: (fd) => api('/api/shares', { method: 'POST', body: {
      name: fd.get('name'), path: fd.get('path'), comment: fd.get('comment') || '', read_only: fd.has('read_only'), guest_ok: fd.has('guest_ok'),
      users: data.users.filter((u) => fd.get(`acl_${u}`)).map((u) => ({ name: u, access: fd.get(`acl_${u}`) })),
      nfs: fd.has('nfs'), nfs_clients: fd.get('nfs_clients') || '',
    } }),
  });
  if (ok) { toast('Đã lưu chia sẻ'); reload(); }
}

async function userDialog(user, reload) {
  const ok = await modal(user ? `Đổi mật khẩu SMB của ${user}` : 'Thêm người dùng SMB', [
    field('Tên người dùng', h('input', { name: 'username', value: user || '', required: true, maxlength: 32, pattern: '[a-z_][a-z0-9_]*(-[a-z0-9_]+)*', readonly: !!user, autocomplete: 'off' }), !user && 'Chữ thường, số, _ và -.'),
    field('Mật khẩu', h('input', { name: 'password', type: 'password', required: true, minlength: 8, autocomplete: 'new-password' })),
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

function accessText(s) {
  if (s.guest_ok) return tag('Khách', 'warn');
  if (!s.users.length) return `Mọi người dùng (${s.read_only ? 'chỉ đọc' : 'đọc/ghi'})`;
  return h('span', { class: 'tags' }, s.users.map((u) => tag(`${u.name}: ${u.access === 'rw' ? 'ghi' : 'đọc'}`, u.access === 'rw' ? 'ok' : '')));
}

async function pageShares({ show }) {
  const [data, d, net] = await Promise.all([api('/api/shares'), api('/api/storage/disks'), api('/api/network')]);
  const ip = (net.info.interfaces.flatMap((i) => i.ipv4)[0] || location.hostname).split('/')[0];
  const mounts = d.disks.filter((x) => !x.system).flatMap((x) => x.partitions).filter((p) => p.mountpoint).map((p) => p.mountpoint);
  const reload = () => route();
  const shareRows = data.shares.map((s) => [
    h('span', { class: 'file-name' }, h('span', { class: 'chip' }, icon('share')), h('strong', {}, s.name)), h('code', {}, s.path), accessText(s),
    h('span', { class: 'tags' }, tag('SMB', 'ok'), s.nfs && tag('NFS', data.nfs_enabled ? 'ok' : '')),
    h('div', {}, h('code', {}, `\\\\${ip}\\${s.name}`), s.nfs && h('div', {}, h('code', {}, `${ip}:${s.path}`))),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: () => shareDialog(s, data, mounts, reload) }, 'Sửa'),
      h('button', { class: 'btn small danger', onclick: () => confirmDialog('Xóa chia sẻ', `Xóa chia sẻ "${s.name}"? Dữ liệu trong ${s.path} vẫn được giữ nguyên.`, 'Xóa',
        () => api(`/api/shares/${encodeURIComponent(s.name)}`, { method: 'DELETE' })).then((ok) => ok && reload()) }, 'Xóa')),
  ]);
  const userRows = data.users.map((u) => [
    h('span', { class: 'file-name' }, h('span', { class: 'avatar' }, u.slice(0, 1).toUpperCase()), h('strong', {}, u)),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: () => userDialog(u, reload) }, 'Đổi mật khẩu'),
      h('button', { class: 'btn small danger', onclick: () => confirmDialog('Xóa người dùng', `Xóa người dùng SMB "${u}"?`, 'Xóa',
        () => api(`/api/smb-users/${encodeURIComponent(u)}`, { method: 'DELETE' })).then((ok) => ok && reload()) }, 'Xóa')),
  ]);
  const nfsToggle = h('label', { class: 'check' },
    h('input', { type: 'checkbox', checked: data.nfs_enabled, disabled: !data.nfs_available,
      onchange: (e) => act(null, () => api('/api/nfs', { method: 'POST', body: { enabled: e.target.checked } }), e.target.checked ? 'Đã bật NFS' : 'Đã tắt NFS').then((ok) => ok ? reload() : (e.target.checked = !e.target.checked)) }),
    h('span', {}, 'NFS server'));
  show(
    pageHead('Thư mục chia sẻ', 'Chia sẻ qua SMB (Windows, macOS, điện thoại) và NFS (Linux).', h('button', { class: 'btn primary', onclick: () => shareDialog(null, data, mounts, reload) }, icon('plus'), 'Tạo chia sẻ')),
    !mounts.length && h('div', { class: 'alert warn' }, icon('alert'), h('span', {}, 'Chưa có ổ dữ liệu nào được mount. Vào ', h('a', { href: '#/storage' }, 'Quản lý ổ đĩa'), ' để mount trước.')),
    h('div', { class: 'grid cols-4 mb' },
      stat({ ico: 'share', label: 'Thư mục chia sẻ', value: String(data.shares.length), sub: `${data.shares.filter((s) => s.nfs).length} có NFS` }),
      stat({ ico: 'key', color: 'teal', label: 'Người dùng SMB', value: String(data.users.length), sub: 'đăng nhập bằng mật khẩu riêng' }),
      stat({ ico: 'network', color: 'orange', label: 'Workgroup', value: data.workgroup, sub: `\\\\${ip}` }),
      stat({ ico: 'services', color: 'green', label: 'NFS', value: data.nfs_enabled ? 'Đang bật' : 'Tắt', sub: data.nfs_available ? 'có sẵn' : 'không có trong image' })),
    panel('Thư mục chia sẻ', table(['Tên', 'Thư mục', 'Quyền', 'Giao thức', 'Địa chỉ', ''], shareRows, 'Chưa có thư mục chia sẻ nào.'), nfsToggle, 'share'),
    h('div', { class: 'grid cols-2' },
      panel('Người dùng SMB', table(['Tên', ''], userRows, 'Chưa có người dùng SMB nào.'), h('button', { class: 'btn small primary', onclick: () => userDialog(null, reload) }, icon('plus'), 'Thêm'), 'key'),
      panel('Cách kết nối', h('ul', { class: 'help' },
        h('li', {}, 'Windows: File Explorer → gõ ', h('code', {}, `\\\\${ip}`)),
        h('li', {}, 'macOS: Finder → Go → Connect to Server → ', h('code', {}, `smb://${ip}`)),
        h('li', {}, 'Linux: ', h('code', {}, `smb://${ip}/<tên>`), ' hoặc ', h('code', {}, `mount -t nfs ${ip}:/mnt/<ổ>/<thư mục> /mnt/x`)),
        h('li', {}, 'Android/iOS: ứng dụng quản lý file có hỗ trợ SMB.')), null, 'info')),
  );
}

// ---- Mạng ----------------------------------------------------------------------
function pendingBox(p, reload) {
  if (!p) return null;
  const left = Math.max(0, p.deadline - Math.floor(Date.now() / 1000));
  return h('div', { class: 'alert warn' }, icon('clock'),
    h('span', {}, h('strong', {}, `Thay đổi ${p.kind === 'network' ? 'mạng' : 'firewall'} đang chờ xác nhận`), ` — tự hoàn tác sau ${left} giây. `),
    h('button', { class: 'btn small', onclick: (e) => act(e.currentTarget, () => api('/api/network/revert', { method: 'POST' }), 'Đã hoàn tác').then((ok) => ok && reload()) }, 'Hoàn tác ngay'));
}

async function applyNetwork(body) {
  const r = await api('/api/network/config', { method: 'POST', body });
  const newIp = body.mode === 'static' ? body.address.split('/')[0] : null;
  if (!newIp || newIp === location.hostname) {
    await api('/api/network/confirm', { method: 'POST', body: { token: r.token } });
    toast('Đã áp dụng và xác nhận cấu hình mạng');
    return;
  }
  const url = `${location.protocol}//${newIp}${location.port ? ':' + location.port : ''}/#/confirm?token=${r.token}`;
  view.replaceChildren(panel('Đang chuyển sang IP mới', h('div', {},
    h('p', {}, 'NAS đang đổi sang địa chỉ mới. Mở liên kết dưới đây và bấm Xác nhận trong vòng 2 phút, nếu không NAS sẽ tự quay về cấu hình cũ.'),
    h('p', {}, h('a', { href: url, class: 'btn primary' }, url))), null, 'network'));
}

async function pageNetwork({ show }) {
  const n = await api('/api/network');
  const reload = () => route();
  const c = n.config;
  const hostForm = h('form', { class: 'inline-form' },
    h('input', { name: 'hostname', value: n.info.hostname, required: true, maxlength: 63, pattern: '[A-Za-z0-9]+(-[A-Za-z0-9]+)*', 'aria-label': 'Tên máy' }),
    h('button', { class: 'btn primary', type: 'submit' }, 'Lưu'));
  hostForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const hostname = new FormData(hostForm).get('hostname');
    if (await act(hostForm.querySelector('button'), () => api('/api/network/hostname', { method: 'POST', body: { hostname } }), 'Đã đổi tên máy')) reload();
  });
  const ipForm = h('form', {},
    field('Cấp địa chỉ', select('mode', [['dhcp', 'Tự động (DHCP)'], ['static', 'IP tĩnh']], c.mode)),
    h('div', { class: 'static-fields' },
      field('Địa chỉ IP / prefix', h('input', { name: 'address', value: c.address, placeholder: '192.168.1.50/24' })),
      field('Gateway', h('input', { name: 'gateway', value: c.gateway, placeholder: '192.168.1.1' })),
      field('DNS', h('input', { name: 'dns', value: (c.dns || []).join(' '), placeholder: '1.1.1.1 8.8.8.8' }))),
    h('p', { class: 'muted' }, 'Nếu cấu hình mới làm mất kết nối, NAS tự quay về cấu hình cũ sau 2 phút.'),
    h('button', { class: 'btn primary', type: 'submit' }, 'Áp dụng'));
  const staticBox = $('.static-fields', ipForm);
  const syncMode = () => { staticBox.hidden = ipForm.mode.value !== 'static'; };
  ipForm.mode.addEventListener('change', syncMode);
  syncMode();
  ipForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(ipForm);
    const body = { mode: fd.get('mode'), address: fd.get('address') || '', gateway: fd.get('gateway') || '', dns: (fd.get('dns') || '').split(/[\s,]+/).filter(Boolean) };
    await act(ipForm.querySelector('button[type=submit]'), () => applyNetwork(body));
  });
  const eth = n.info.interfaces[0];
  show(
    pageHead('Mạng', 'Địa chỉ IP, card mạng, DNS và tên máy.'),
    pendingBox(n.pending, reload),
    h('div', { class: 'grid cols-4 mb' },
      stat({ ico: 'network', label: 'Địa chỉ IP', value: eth && eth.ipv4[0] ? eth.ipv4[0].split('/')[0] : '—', sub: c.mode === 'static' ? 'IP tĩnh' : 'DHCP' }),
      stat({ ico: 'net', color: 'teal', label: 'Kết nối', value: eth && eth.speed_mbps ? `${eth.speed_mbps} Mb/s` : '—', sub: eth ? `${eth.name} · ${eth.state}` : '' }),
      stat({ ico: 'monitor', color: 'orange', label: 'Gateway', value: n.info.gateway || '—', sub: `DNS ${n.info.dns.join(', ') || '—'}` }),
      stat({ ico: 'info', color: 'green', label: 'Tên máy', value: n.info.hostname, sub: 'NetBIOS dùng 15 ký tự đầu' })),
    h('div', { class: 'grid cols-2 mb' },
      panel('Cấu hình địa chỉ IP', ipForm, null, 'network'),
      panel('Tên máy', h('div', {}, hostForm, h('p', { class: 'muted' }, 'Tên hiển thị trong mạng Windows và trên console.')), null, 'info')),
    panel('Card mạng', table(['Tên', 'Trạng thái', 'IPv4', 'MAC', 'Tốc độ', 'Lưu lượng'], n.info.interfaces.map((i) => [
      h('strong', {}, i.name), i.state === 'up' ? tag('up', 'ok') : tag(i.state || 'unknown'),
      i.ipv4.length ? i.ipv4.map((a) => h('div', {}, h('code', {}, a))) : '—',
      h('code', {}, i.mac || '—'), i.speed_mbps ? `${i.speed_mbps} Mb/s` : '—',
      `↓ ${fmtBytes(i.rx_bytes)} · ↑ ${fmtBytes(i.tx_bytes)}`,
    ])), null, 'net'),
  );
}

function pageConfirm(token) {
  $('#app').hidden = true;
  const box = $('#auth');
  box.hidden = false;
  const msg = h('p', { class: 'muted' }, 'Bạn đang truy cập được NAS qua địa chỉ mới. Bấm xác nhận để giữ cấu hình này.');
  const btn = h('button', { class: 'btn primary block' }, 'Xác nhận cấu hình mạng');
  btn.addEventListener('click', () => act(btn, () => api('/api/network/confirm', { method: 'POST', body: { token } }), 'Đã xác nhận')
    .then((ok) => { if (ok) { msg.textContent = 'Đã lưu. Bạn có thể đăng nhập lại.'; btn.remove(); setTimeout(() => { location.hash = '#/overview'; boot(); }, 1500); } }));
  box.replaceChildren(h('div', { class: 'auth-card' }, h('h1', {}, 'Xác nhận mạng mới'), msg, btn));
}

// ---- Bảo mật ---------------------------------------------------------------------
async function waitForRestart(text, poll = true, delay = 4000) {
  seq++;
  timers.forEach(clearInterval);
  timers = [];
  view.replaceChildren(h('div', { class: 'alert info' }, icon('clock'), text));
  if (!poll) return;
  await sleep(delay);
  for (;;) {
    try { await fetch('/api/auth/status'); location.reload(); return; } catch { await sleep(3000); }
  }
}

async function pageSecurity({ show }) {
  const [settings, fw] = await Promise.all([api('/api/settings'), api('/api/firewall')]);
  const reload = () => route();
  const sec = h('form', {},
    check('https', `Bật HTTPS (cổng ${settings.https_port}, chứng chỉ tự ký)`, settings.https),
    check('http_redirect', `Chuyển hướng HTTP (cổng ${settings.http_port}) sang HTTPS`, settings.http_redirect),
    settings.fingerprint && field('Vân tay chứng chỉ (SHA-256)', h('code', { class: 'wrap' }, settings.fingerprint), 'So với vân tay trình duyệt hiển thị để chắc chắn không bị giả mạo.'),
    h('div', { class: 'row-actions left' },
      h('button', { class: 'btn primary', type: 'submit' }, 'Lưu và khởi động lại dịch vụ web'),
      h('button', { class: 'btn', type: 'button', onclick: (e) => act(e.currentTarget, () => api('/api/settings/regenerate-cert', { method: 'POST' }), 'Đã tạo chứng chỉ mới').then((ok) => ok && waitForRestart('Đang khởi động lại dịch vụ web…')) }, 'Tạo chứng chỉ mới')));
  sec.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(sec);
    const body = { https: fd.has('https'), http_redirect: fd.has('http_redirect') };
    if (await act(sec.querySelector('button[type=submit]'), () => api('/api/settings', { method: 'POST', body }), 'Đã lưu')) {
      const target = body.https ? `https://${location.hostname}:${settings.https_port}/` : `http://${location.hostname}:${settings.http_port}/`;
      view.replaceChildren(h('div', { class: 'alert info' }, 'Dịch vụ web đang khởi động lại. Mở lại tại ', h('a', { href: target }, target)));
    }
  });
  const fwForm = h('form', {},
    check('enabled', 'Bật firewall (chặn mọi kết nối không nằm trong danh sách)', fw.enabled),
    field('Mạng được phép truy cập', h('input', { name: 'allowed', value: fw.allowed.join(' ') }), 'IP hoặc dải mạng, cách nhau bằng dấu cách.'),
    check('smb', 'Cho phép SMB (cổng 445)', fw.smb),
    check('nfs', 'Cho phép NFS (111, 2049, 20048)', fw.nfs),
    h('p', { class: 'muted' }, 'Giao diện web luôn mở cho các mạng trên. Nếu bạn tự khóa mình, NAS tự hoàn tác sau 2 phút.'),
    h('button', { class: 'btn primary', type: 'submit' }, 'Áp dụng'));
  fwForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(fwForm);
    const body = { enabled: fd.has('enabled'), allowed: (fd.get('allowed') || '').split(/[\s,]+/).filter(Boolean), smb: fd.has('smb'), nfs: fd.has('nfs') };
    const done = await act(fwForm.querySelector('button[type=submit]'), async () => {
      const r = await api('/api/firewall', { method: 'POST', body });
      await sleep(1500);
      await api('/api/network/confirm', { method: 'POST', body: { token: r.token } });
    }, 'Đã áp dụng firewall');
    if (done) reload();
  });
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
  const score = [settings.https, settings.http_redirect, fw.enabled].filter(Boolean).length;
  show(
    pageHead('Bảo mật', 'HTTPS, firewall, tài khoản quản trị và các lớp bảo vệ của hệ thống.'),
    h('div', { class: 'grid cols-4 mb' },
      stat({ ico: 'shieldok', color: score === 3 ? 'green' : 'orange', label: 'Mức bảo vệ', value: score === 3 ? 'Cao' : score === 2 ? 'Trung bình' : 'Thấp', sub: `${score}/3 lớp đang bật` }),
      stat({ ico: 'lock', label: 'HTTPS', value: settings.https ? 'Bật' : 'Tắt', sub: settings.http_redirect ? 'HTTP tự chuyển sang HTTPS' : 'HTTP vẫn mở' }),
      stat({ ico: 'shield', color: 'teal', label: 'Firewall', value: fw.enabled ? 'Bật' : 'Tắt', sub: `${fw.allowed.length} mạng được phép` }),
      stat({ ico: 'key', color: 'orange', label: 'Đăng nhập', value: 'argon2id', sub: 'khóa IP 5 phút sau 5 lần sai' })),
    h('div', { class: 'grid cols-2 mb' },
      panel('Truy cập web (HTTPS)', sec, null, 'lock'),
      panel('Firewall', fwForm, null, 'shield')),
    h('div', { class: 'grid cols-2' },
      panel('Đổi mật khẩu quản trị', pw, null, 'key'),
      panel('Các lớp bảo vệ có sẵn', h('ul', { class: 'help' },
        h('li', {}, 'Tách quyền: giao diện web chạy bằng user thường, chỉ một helper nhỏ chạy root với danh sách thao tác cố định.'),
        h('li', {}, 'Bản cập nhật phải có chữ ký số ed25519 hợp lệ, nếu không sẽ bị từ chối.'),
        h('li', {}, 'Rootfs chỉ-đọc, không SSH/telnet, console không cho đăng nhập.'),
        h('li', {}, 'Chống CSRF (header riêng + cookie SameSite=Strict), CSP chặt, chặn đường dẫn ra ngoài ổ dữ liệu.'),
        h('li', {}, 'Quên mật khẩu: tạo file rỗng ', h('code', {}, 'hehenas-reset-admin.txt'), ' trên phân vùng boot của thẻ nhớ.')), null, 'info')),
  );
}

// ---- Cập nhật ----------------------------------------------------------------------
async function power(action) {
  const label = action === 'reboot' ? 'Khởi động lại' : 'Tắt máy';
  const ok = await modal(label, [h('p', {}, action === 'reboot' ? 'NAS sẽ khởi động lại, kết nối SMB bị ngắt khoảng 1 phút.' : 'NAS sẽ tắt hẳn. Muốn bật lại phải rút điện rồi cắm lại.')],
    { ok: label, danger: true, submit: () => api('/api/system/power', { method: 'POST', body: { action } }) });
  if (!ok) return;
  waitForRestart(action === 'reboot' ? 'Đang khởi động lại… Trang sẽ tự tải lại khi NAS sẵn sàng.' : 'Đã gửi lệnh tắt máy. Rút điện khi đèn ACT xanh ngừng nhấp nháy.', action === 'reboot', 20000);
}

async function pageUpdate({ show, every }) {
  const upd = await api('/api/system/update');
  const reload = () => route();
  const on = upd.online || {};
  const jobBox = h('div');
  const renderJob = (j) => {
    if (!j || !j.state || j.state === 'idle') { jobBox.replaceChildren(); return; }
    if (j.state === 'downloading') jobBox.replaceChildren(h('div', { class: 'muted pad' }, `Đang tải v${j.version}: ${fmtBytes(j.downloaded)} / ${fmtBytes(j.total)}`), meter(pct(j.downloaded, j.total), 'accent'));
    else if (j.state === 'installing') jobBox.replaceChildren(h('div', { class: 'muted pad' }, `Đang kiểm tra chữ ký và ghi v${j.version} vào slot dự phòng…`), meter(100, 'accent'));
    else if (j.state === 'staged') jobBox.replaceChildren(h('div', { class: 'alert info' }, icon('shieldok'), h('span', {}, `v${j.version} đã được xác minh và cài vào slot dự phòng. `,
      upd.auto_update === 'install' ? `NAS sẽ tự khởi động lại lúc ${upd.update_hour}:00. ` : ''), h('button', { class: 'btn small primary', type: 'button', onclick: () => power('reboot') }, 'Khởi động lại ngay')));
    else if (j.state === 'error') jobBox.replaceChildren(h('div', { class: 'alert crit' }, icon('alert'), j.error || 'Lỗi không rõ'));
  };
  renderJob(upd.job);
  every(async () => { const u = await api('/api/system/update'); renderJob(u.job); }, 2500);

  const onlineTag = upd.clock_ok === false ? tag('Đang chờ đồng bộ giờ (NTP)', 'warn')
    : !on.checked_at ? tag('Chưa kiểm tra')
      : on.error ? tag(on.error, 'err')
        : on.available ? tag(`Có bản mới v${on.latest}`, 'warn') : tag('Đang dùng bản mới nhất', 'ok');
  const autoForm = h('form', { class: 'inline-form' },
    select('auto_update', [['install', 'Tự động tải, cài và khởi động lại'], ['notify', 'Chỉ thông báo khi có bản mới'], ['off', 'Tắt']], upd.auto_update),
    select('update_hour', Array.from({ length: 24 }, (_, i) => [i, `lúc ${String(i).padStart(2, '0')}:00`]), upd.update_hour),
    h('button', { class: 'btn', type: 'submit' }, 'Lưu'));
  autoForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(autoForm);
    await act(autoForm.querySelector('button'), () => api('/api/system/update/settings', { method: 'POST', body: { auto_update: fd.get('auto_update'), update_hour: Number(fd.get('update_hour')) } }), 'Đã lưu cài đặt cập nhật');
  });

  const updInput = h('input', { type: 'file', accept: '.tar', hidden: true });
  const updProgress = h('div');
  updInput.addEventListener('change', async () => {
    const f = updInput.files[0];
    updInput.value = '';
    if (!f) return;
    updProgress.replaceChildren(meter(0, 'accent'));
    try {
      const r = await uploadFile('/api/system/update', f, (x) => updProgress.replaceChildren(h('small', {}, x < 1 ? `Đang tải lên ${Math.round(x * 100)}%` : 'Đang kiểm tra chữ ký và ghi vào slot dự phòng…'), meter(x * 100, 'accent')));
      updProgress.replaceChildren(h('div', { class: 'alert info' }, `Đã cài v${r.version} vào slot dự phòng. Khởi động lại để dùng bản mới.`, ' ', h('button', { class: 'btn small primary', onclick: () => power('reboot') }, 'Khởi động lại')));
    } catch (e) {
      updProgress.replaceChildren(h('div', { class: 'alert crit' }, e.message));
    }
  });
  const slotText = (s) => (upd.slots && upd.slots[s] ? `v${upd.slots[s]}` : 'chưa rõ');

  show(
    pageHead('Cập nhật hệ thống', 'Cập nhật qua mạng từ GitHub Releases. Bản mới được ghi vào slot dự phòng, lỗi thì tự quay về bản cũ.'),
    h('div', { class: 'grid cols-4 mb' },
      stat({ ico: 'info', label: 'Phiên bản đang chạy', value: `v${upd.version}`, sub: upd.supported ? `slot ${String(upd.current_slot || '').toUpperCase()}` : 'không hỗ trợ A/B' }),
      stat({ ico: 'update', color: on.available ? 'orange' : 'green', label: 'Bản mới nhất', value: on.latest ? `v${on.latest}` : '—', sub: on.checked_at ? `kiểm tra lúc ${fmtDate(on.checked_at)}` : 'chưa kiểm tra' }),
      stat({ ico: 'clock', color: 'teal', label: 'Tự động cập nhật', value: { install: 'Bật', notify: 'Thông báo', off: 'Tắt' }[upd.auto_update] || '—', sub: `khởi động lại lúc ${upd.update_hour}:00` }),
      stat({ ico: 'shieldok', color: 'green', label: 'Xác minh', value: 'ed25519', sub: 'chỉ cài gói có chữ ký hợp lệ' })),
    !upd.supported && h('div', { class: 'alert warn' }, icon('info'), 'Image này không hỗ trợ cập nhật A/B (chỉ có trên image thẻ nhớ Raspberry Pi).'),
    h('div', { class: 'grid cols-2' },
      panel('Cập nhật qua mạng', h('div', {},
        h('div', { class: 'tags mb' }, onlineTag),
        jobBox,
        h('div', { class: 'row-actions left' },
          h('button', { class: 'btn', disabled: !upd.supported, onclick: (e) => act(e.currentTarget, () => api('/api/system/update/check', { method: 'POST' })).then((ok) => ok && reload()) }, icon('refresh'), 'Kiểm tra ngay'),
          on.available && h('button', { class: 'btn primary', onclick: (e) => act(e.currentTarget, () => api('/api/system/update/install', { method: 'POST' }), 'Bắt đầu tải bản cập nhật') }, icon('update'), `Cập nhật lên v${on.latest}`),
          on.notes_url && h('a', { class: 'btn', href: on.notes_url, target: '_blank', rel: 'noopener noreferrer' }, 'Xem thay đổi')),
        h('div', { class: 'muted pad' }, 'Chế độ tự động'), autoForm), null, 'update'),
      panel('Slot hệ thống (A/B)', h('div', {},
        kvList([
          ['Slot A', [slotText('a'), upd.current_slot === 'a' ? [' ', tag('đang chạy', 'ok')] : null]],
          ['Slot B', [slotText('b'), upd.current_slot === 'b' ? [' ', tag('đang chạy', 'ok')] : null]],
          ['Trạng thái', upd.trying ? tag(`đang chạy thử slot ${upd.trying.toUpperCase()}`, 'warn') : tag('ổn định', 'ok')],
        ]),
        h('div', { class: 'row-actions left' },
          h('button', { class: 'btn', disabled: !upd.supported, onclick: () => updInput.click() }, icon('upload'), 'Cài từ file .tar'),
          h('button', { class: 'btn', disabled: !upd.supported, onclick: () => confirmDialog('Quay về bản trước', 'Lần khởi động tới sẽ dùng slot còn lại. Tiếp tục?', 'Quay về',
            () => api('/api/system/update/rollback', { method: 'POST' })).then((ok) => ok && toast('Khởi động lại để quay về bản trước')) }, 'Quay về bản trước')),
        updInput, updProgress), null, 'backup')),
  );
}

// ---- Sao lưu ------------------------------------------------------------------------
async function pageBackup({ show }) {
  const restoreInput = h('input', { type: 'file', accept: 'application/json,.json', hidden: true });
  restoreInput.addEventListener('change', async () => {
    const f = restoreInput.files[0];
    restoreInput.value = '';
    if (!f) return;
    let data;
    try { data = JSON.parse(await f.text()); } catch { toast('File không hợp lệ', 'err'); return; }
    const ok = await confirmDialog('Khôi phục cấu hình', `Khôi phục cấu hình từ "${data.hostname || '?'}" (${fmtDate(data.created)})? Cấu hình hiện tại sẽ bị thay thế${data.secrets ? ', kể cả mật khẩu' : ''}.`, 'Khôi phục',
      () => api('/api/system/restore', { method: 'POST', body: data }));
    if (ok) waitForRestart('Đã khôi phục. Dịch vụ đang khởi động lại…');
  });
  show(
    pageHead('Sao lưu & khôi phục', 'Lưu cấu hình NAS ra một file JSON để khôi phục khi cài lại hoặc chuyển máy.'),
    h('div', { class: 'grid cols-2' },
      panel('Sao lưu cấu hình', h('div', {},
        h('p', {}, 'Gồm: thư mục chia sẻ, ổ đĩa đã mount, mạng, firewall và cài đặt.'),
        h('div', { class: 'alert warn' }, icon('lock'), 'Bản "kèm mật khẩu" chứa mã băm mật khẩu admin và SMB, hãy cất giữ như mật khẩu.'),
        h('div', { class: 'row-actions left' },
          h('a', { class: 'btn primary', href: '/api/system/backup', download: true }, icon('backup'), 'Tải bản sao lưu'),
          h('a', { class: 'btn', href: '/api/system/backup?secrets=1', download: true }, 'Kèm mật khẩu'))), null, 'backup'),
      panel('Khôi phục', h('div', {},
        h('p', {}, 'Chọn file sao lưu .json. NAS sẽ kiểm tra file trước khi áp dụng, sau đó khởi động lại dịch vụ.'),
        h('div', { class: 'row-actions left' }, h('button', { class: 'btn', onclick: () => restoreInput.click() }, icon('upload'), 'Chọn file sao lưu…')),
        restoreInput,
        h('div', { class: 'muted pad' }, 'Khôi phục khẩn cấp khi không vào được web'),
        h('ul', { class: 'help' },
          h('li', {}, 'Tắt NAS, cắm thẻ nhớ vào máy tính, mở phân vùng "boot".'),
          h('li', {}, h('code', {}, 'hehenas-reset-admin.txt'), ': đặt lại tài khoản admin.'),
          h('li', {}, h('code', {}, 'hehenas-factory-reset.txt'), ': xóa toàn bộ cấu hình (dữ liệu trên ổ USB không bị ảnh hưởng).'))), null, 'refresh')),
  );
}

// ---- Dịch vụ & nguồn ----------------------------------------------------------------
async function pageServices({ show }) {
  const svcs = await api('/api/system/services');
  const reload = () => route();
  const labels = { start: 'Chạy', restart: 'Khởi động lại', stop: 'Dừng' };
  const rows = svcs.map((s) => [
    h('span', { class: 'file-name' }, h('span', { class: `status-dot ${s.running ? 'ok' : 'err'}` }), s.name),
    s.running ? tag('đang chạy', 'ok') : tag('đã dừng', 'err'),
    s.running ? fmtBytes(s.rss_bytes) : '—', s.pids.join(', ') || '—',
    s.controllable ? h('div', { class: 'row-actions' }, Object.keys(labels).map((a) => h('button', {
      class: 'btn small',
      onclick: (e) => act(e.currentTarget, () => api(`/api/system/services/${s.id}/${a}`, { method: 'POST' }), `${s.name}: ${labels[a]}`).then((ok) => ok && setTimeout(reload, 600)),
    }, labels[a]))) : h('span', { class: 'muted' }, 'tự quản lý'),
  ]);
  const totalRam = svcs.reduce((x, s) => x + (s.running ? s.rss_bytes : 0), 0);
  show(
    pageHead('Dịch vụ & nguồn', 'Trạng thái các dịch vụ hệ thống, RAM từng dịch vụ, khởi động lại hoặc tắt NAS.'),
    h('div', { class: 'grid cols-4 mb' },
      stat({ ico: 'services', label: 'Đang chạy', value: `${svcs.filter((s) => s.running).length}/${svcs.length}`, sub: 'dịch vụ' }),
      stat({ ico: 'mem', color: 'teal', label: 'RAM các dịch vụ', value: fmtBytes(totalRam), sub: 'tổng RSS' })),
    panel('Dịch vụ', table(['Dịch vụ', 'Trạng thái', 'RAM', 'PID', ''], rows), null, 'services'),
    panel('Nguồn', h('div', {},
      h('p', { class: 'muted' }, 'Hãy tắt máy từ đây thay vì rút điện để tránh hỏng dữ liệu trên ổ USB.'),
      h('div', { class: 'row-actions left' },
        h('button', { class: 'btn', onclick: () => power('reboot') }, icon('refresh'), 'Khởi động lại'),
        h('button', { class: 'btn danger', onclick: () => power('poweroff') }, icon('power'), 'Tắt máy'))), null, 'power'),
  );
}

// ---- Nhật ký -------------------------------------------------------------------------
async function pageLogs({ show, every }) {
  const logs = await api('/api/system/logs');
  const pre = h('pre', { class: 'logs' }, logs.lines.join('\n') || '(trống)');
  const src = select('source', [['all', 'Tất cả'], ['hehenasd', 'HeheNAS'], ['smb', 'SMB'], ['nfs', 'NFS'], ['network', 'Mạng'], ['kernel', 'Kernel']], 'all');
  const q = h('input', { name: 'q', placeholder: 'Tìm…', 'aria-label': 'Tìm trong nhật ký' });
  const auto = h('input', { type: 'checkbox', checked: true });
  const refresh = async () => {
    const atBottom = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 30;
    const r = await api(`/api/system/logs?source=${src.value}&q=${encodeURIComponent(q.value)}`);
    pre.textContent = r.lines.join('\n') || '(không có dòng nào)';
    if (atBottom) pre.scrollTop = pre.scrollHeight;
  };
  src.addEventListener('change', refresh);
  q.addEventListener('keydown', (e) => { if (e.key === 'Enter') refresh(); });
  show(
    pageHead('Nhật ký hệ thống', logs.file),
    panel('Nhật ký', pre, [src, q, h('label', { class: 'check' }, auto, h('span', {}, 'Tự làm mới')), h('button', { class: 'btn small', onclick: (e) => act(e.currentTarget, refresh) }, icon('refresh'), 'Làm mới')], 'logs'),
  );
  pre.scrollTop = pre.scrollHeight;
  every(async () => { if (auto.checked) await refresh(); }, 5000);
}

// ---- boot -----------------------------------------------------------------------------
async function boot() {
  const m = location.hash.match(/^#\/confirm\?token=([0-9a-f]+)/);
  try {
    status = await api('/api/auth/status');
    $('#demo-banner').hidden = !status.demo;
    if (m) return pageConfirm(m[1]);
    $('#node-name').textContent = status.hostname;
    $('#version').textContent = `v${status.version}`;
    document.title = `${status.hostname} · HeheNAS`;
    if (status.setup_required) showAuth(true);
    else if (!status.user) showAuth(false);
    else showApp(status.user);
  } catch (e) {
    const box = $('#auth');
    box.hidden = false;
    box.replaceChildren(h('div', { class: 'auth-card' }, h('h1', {}, 'Không kết nối được HeheNAS'), h('p', { class: 'muted' }, e.message)));
  }
}
buildShell();
boot();
