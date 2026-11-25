'use strict';
/* RustNAS dashboard — vanilla JS, no build step. Text is always inserted as text, never as HTML. */

const $ = (sel, root = document) => root.querySelector(sel);
const view = $('#view');
const SVGNS = 'http://www.w3.org/2000/svg';
let timers = [];
let seq = 0; // bumps on every navigation so stale renders are dropped

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
  cpu: 'M9 3v2M15 3v2M9 19v2M15 19v2M3 9h2M3 15h2M19 9h2M19 15h2M7 5h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2zM9 9h6v6H9z',
  mem: 'M2 7a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2zM6 19v-3M10 19v-3M14 19v-3M18 19v-3M7 9v3M12 9v3M17 9v3',
  temp: 'M14 4v10.54a4 4 0 1 1-4 0V4a2 2 0 0 1 4 0z',
  net: 'M22 12h-4l-3 9L9 3l-3 9H2',
  disk: 'M22 12H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11zM6 16h.01M10 16h.01',
};
function icon(name, cls = 'ico') {
  const el = svg('svg', { class: cls, viewBox: '0 0 24 24', 'aria-hidden': 'true' });
  el.append(svg('path', { d: ICONS[name] || '' }));
  return el;
}
const tone = (p) => (p >= 90 ? 'crit' : p >= 75 ? 'warn' : 'ok');
/** Ring gauge (0-100). */
function gauge(p, label) {
  p = Math.min(100, Math.max(0, p || 0));
  const r = 26, c = 2 * Math.PI * r;
  const g = svg('svg', { class: `gauge ${tone(p)}`, viewBox: '0 0 64 64', role: 'img', 'aria-label': `${Math.round(p)}%` });
  g.append(svg('circle', { class: 'track', cx: 32, cy: 32, r }));
  g.append(svg('circle', { class: 'bar', cx: 32, cy: 32, r, 'stroke-dasharray': `${(p / 100) * c} ${c}`, transform: 'rotate(-90 32 32)' }));
  const t = svg('text', { x: 32, y: 37, 'text-anchor': 'middle' });
  t.textContent = label ?? `${Math.round(p)}%`;
  g.append(t);
  return g;
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
  if (method !== 'GET') opt.headers['X-RustNAS'] = '1';
  if (raw !== undefined) opt.body = raw;
  else if (body !== undefined) {
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

/** PUT a file with progress (fetch has no upload progress). */
function uploadFile(url, file, onProgress) {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open('PUT', url);
    x.setRequestHeader('X-RustNAS', '1');
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

/** Modal form. `submit(formData)` may throw: the error is shown and the dialog stays open. */
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
const confirmDialog = (title, text, okLabel, fn) =>
  modal(title, [h('p', {}, text)], { ok: okLabel, danger: true, submit: fn });

const field = (label, input, hint) =>
  h('label', { class: 'field' }, h('span', {}, label), input, hint && h('small', {}, hint));
const check = (name, label, checked, value) =>
  h('label', { class: 'check' }, h('input', { type: 'checkbox', name, checked, value: value ?? 'on' }), h('span', {}, label));
function select(name, options, current) {
  const s = h('select', { name }, options.map(([v, l]) => h('option', { value: v, selected: v === current }, l)));
  return s;
}
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

function sparkChart(series, max, fmt, names) {
  const W = 300, H = 100;
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
  return h('div', {}, s, h('div', { class: 'legend' },
    series.map((pts, i) => h('span', {}, h('i', { class: `s${i}` }), `${names ? names[i] + ': ' : ''}${fmt(pts[n - 1])}`)),
    h('span', {}, `đỉnh trục: ${fmt(top)}`)));
}

function alertBox(a) {
  return h('div', { class: `alert ${a.level}` },
    h('strong', {}, a.title), ' ', h('span', {}, a.detail),
    a.page && a.page !== 'overview' ? [' ', h('a', { href: `#/${a.page}` }, 'Xem')] : null);
}

// ---- auth -----------------------------------------------------------------
let status = {};

function showAuth(setup) {
  seq++;
  timers.forEach(clearInterval);
  timers = [];
  $('#app').hidden = true;
  const box = $('#auth');
  box.hidden = false;
  const err = h('p', { class: 'form-err', role: 'alert' });
  const form = h('form', { class: 'auth-card' },
    h('div', { class: 'logo' }, h('img', { src: '/favicon.svg', alt: '', width: 28, height: 28 }), h('span', {}, 'RustNAS')),
    h('h1', {}, setup ? 'Thiết lập lần đầu' : 'Đăng nhập'),
    setup && h('p', { class: 'muted' }, 'Tạo tài khoản quản trị. Người đầu tiên mở trang này sẽ thành quản trị viên, nên hãy làm ngay sau khi bật NAS.'),
    !status.secure && !status.demo && h('p', { class: 'alert warn' }, 'Kết nối chưa mã hóa. Hãy dùng địa chỉ https://… nếu có thể.'),
    field('Tên đăng nhập', h('input', { name: 'username', autocomplete: 'username', required: true, value: setup ? 'admin' : '' })),
    field('Mật khẩu', h('input', { name: 'password', type: 'password', required: true, minlength: setup ? 8 : null, autocomplete: setup ? 'new-password' : 'current-password' })),
    setup && field('Nhập lại mật khẩu', h('input', { name: 'password2', type: 'password', required: true, autocomplete: 'new-password' })),
    err,
    h('button', { class: 'btn primary block', type: 'submit' }, setup ? 'Tạo tài khoản' : 'Đăng nhập'),
    status.demo && h('p', { class: 'muted center' }, 'Bản xem thử: tài khoản admin / mật khẩu rustnas123'));
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
  refreshAlertPill();
}

async function refreshAlertPill() {
  try {
    const list = await api('/api/alerts');
    const bad = list.filter((a) => a.level !== 'info');
    const pill = $('#alert-pill');
    pill.hidden = !bad.length;
    pill.className = `alert-pill ${bad.some((a) => a.level === 'crit') ? 'crit' : 'warn'}`;
    pill.textContent = `${bad.length} cảnh báo`;
  } catch { /* ignore */ }
}

// ---- routing --------------------------------------------------------------
const pages = {
  overview: pageOverview, storage: pageStorage, files: pageFiles, shares: pageShares,
  network: pageNetwork, system: pageSystem,
};

function route() {
  const my = ++seq;
  timers.forEach(clearInterval);
  timers = [];
  const [id, query = ''] = location.hash.replace(/^#\/?/, '').split('?');
  const page = pages[id] ? id : 'overview';
  document.querySelectorAll('.nav a').forEach((a) => a.classList.toggle('active', a.dataset.page === page));
  view.replaceChildren(h('p', { class: 'muted pad' }, 'Đang tải…'));
  const ctx = {
    params: new URLSearchParams(query),
    show: (...nodes) => { if (my === seq) view.replaceChildren(...nodes.flat().filter(Boolean)); },
    every: (fn, ms) => { if (my === seq) timers.push(setInterval(() => fn().catch(() => {}), ms)); },
    alive: () => my === seq,
  };
  pages[page](ctx).catch((e) => {
    if (my === seq && !$('#app').hidden) view.replaceChildren(h('div', { class: 'alert crit' }, e.message));
  });
}
window.addEventListener('hashchange', () => { if (!location.hash.startsWith('#/confirm')) route(); });

// ---- Overview -------------------------------------------------------------
function card({ ico, title, value, sub, pct: p, label }) {
  return h('div', { class: `card stat ${p != null ? tone(p) : ''}` },
    h('div', { class: 'stat-main' },
      h('div', { class: 'card-title' }, h('span', { class: 'chip' }, icon(ico)), title),
      h('div', { class: 'card-value' }, value),
      h('div', { class: 'card-sub' }, sub)),
    p != null && gauge(p, label));
}

function diskSummary(disks) {
  const parts = disks.flatMap((d) => d.partitions.map((p) => ({ d, p }))).filter(({ p }) => p.mountpoint && p.total_bytes);
  if (!parts.length) return h('p', { class: 'muted' }, 'Chưa có ổ nào được mount.');
  return h('div', { class: 'disk-list' }, parts.map(({ d, p }) => {
    const u = pct(p.used_bytes, p.total_bytes);
    return h('div', { class: 'disk-row' },
      h('span', { class: `chip${d.system ? '' : ' accent'}` }, icon('disk')),
      h('div', { class: 'disk-info' },
        h('div', { class: 'disk-head' }, h('strong', {}, p.label || p.name), h('code', {}, p.mountpoint), d.system && tag('hệ thống'), h('span', { class: `disk-pct ${tone(u)}` }, `${u}%`)),
        meter(u),
        h('small', {}, `${fmtBytes(p.used_bytes)} / ${fmtBytes(p.total_bytes)} · ${p.fstype || ''} · còn ${fmtBytes(p.total_bytes - p.used_bytes)}`)));
  }));
}

async function pageOverview({ show, every }) {
  const cards = h('div', { class: 'cards' });
  const cpuChart = h('div', { class: 'chart' });
  const netChart = h('div', { class: 'chart' });
  const info = h('div');
  const alertsBox = h('div');
  const draw = (o, hist) => {
    $('#node-name').textContent = o.hostname;
    $('#version').textContent = `v${o.version}`;
    const memUsed = o.mem_total - o.mem_available;
    const t = o.temp_c;
    const netPct = (o.net_rx_bps + o.net_tx_bps) / 125000; // % of 100 Mbit/s
    cards.replaceChildren(
      card({ ico: 'cpu', title: 'CPU', value: `${o.cpu_percent.toFixed(0)}%`, pct: o.cpu_percent, sub: `${o.cpu_cores} nhân · tải ${o.load.map((x) => x.toFixed(2)).join(' / ')}` }),
      card({ ico: 'mem', title: 'Bộ nhớ', value: fmtBytes(memUsed), pct: pct(memUsed, o.mem_total), sub: `trên ${fmtBytes(o.mem_total)} · cache ${fmtBytes(o.mem_cached)}` }),
      card({ ico: 'temp', title: 'Nhiệt độ', value: t != null ? `${t.toFixed(1)} °C` : '—', pct: t != null ? (t / 85) * 100 : null, label: t != null ? `${Math.round(t)}°` : '', sub: t >= 80 ? 'Quá nóng: CPU đang bị hạ xung' : 'Pi 3B hạ xung ở 80 °C' }),
      card({ ico: 'net', title: 'Mạng', value: `↓ ${fmtRate(o.net_rx_bps)}`, pct: Math.min(100, netPct), label: `${Math.round(netPct)}%`, sub: `↑ ${fmtRate(o.net_tx_bps)} · vòng = % của 100 Mb/s` }),
    );
    cpuChart.replaceChildren(sparkChart([hist.map((p) => p.cpu)], 100, (v) => `${v.toFixed(0)}%`));
    netChart.replaceChildren(sparkChart([hist.map((p) => p.rx), hist.map((p) => p.tx)], null, fmtRate, ['Nhận', 'Gửi']));
    info.replaceChildren(kvList([
      ['Tên máy', o.hostname], ['Phần cứng', o.model || '—'], ['Kernel', o.kernel], ['RustNAS', `v${o.version}`],
      ['Thời gian chạy', fmtUptime(o.uptime_secs)],
      ['Swap (zram)', o.swap_total ? `${fmtBytes(o.swap_total - o.swap_free)} / ${fmtBytes(o.swap_total)}` : 'tắt'],
      ['RAM của rustnasd', fmtBytes(o.daemon_rss)],
    ]));
  };
  const [o, hist, d, al] = await Promise.all([api('/api/system/overview'), api('/api/system/history'), api('/api/storage/disks'), api('/api/alerts')]);
  draw(o, hist);
  alertsBox.replaceChildren(...al.map(alertBox));
  const area = (cls, el) => { el.classList.add(cls); return el; };
  show(
    h('h1', {}, 'Tổng quan'), alertsBox, cards,
    h('div', { class: 'ov-grid' },
      area('ov-cpu', panel('Tải CPU (10 phút)', cpuChart)),
      area('ov-net', panel('Lưu lượng mạng', netChart)),
      area('ov-disk', panel('Ổ đĩa', diskSummary(d.disks), h('a', { href: '#/storage', class: 'btn small' }, 'Quản lý'))),
      area('ov-sys', panel('Hệ thống', info))),
  );
  every(async () => {
    const [o2, h2] = await Promise.all([api('/api/system/overview'), api('/api/system/history')]);
    draw(o2, h2);
  }, 3000);
  every(async () => {
    const a = await api('/api/alerts');
    alertsBox.replaceChildren(...a.map(alertBox));
    refreshAlertPill();
  }, 30000);
}

// ---- Storage --------------------------------------------------------------
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
      ['Sector đã thay (5)', s.reallocated ?? '—'], ['Sector chờ (197)', s.pending ?? '—'],
      ['Không sửa được (198)', s.uncorrectable ?? '—'],
      s.percentage_used != null && ['Mức hao mòn (NVMe)', `${s.percentage_used}%`],
    ].filter(Boolean)) : h('p', { class: 'alert warn' }, s.message || 'Không đọc được SMART.'));
  } catch (e) {
    box.replaceChildren(h('p', { class: 'alert crit' }, e.message));
  }
  return p;
}

function wipeDialog(kind, target, size, reload) {
  const isInit = kind === 'initialize';
  return modal(isInit ? `Khởi tạo ổ ${target}` : `Format ${target} thành ext4`, [
    h('div', { class: 'alert crit' }, isInit
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
      h('button', { class: 'btn small', title: 'Giao toàn bộ file cho tài khoản chia sẻ (dùng cho ổ mang từ máy khác sang)', onclick: post('/api/storage/fix-owner', { path: p.mountpoint }, 'Đã sửa quyền') }, 'Sửa quyền'),
      h('button', { class: 'btn small', onclick: post('/api/storage/unmount', { device: p.name }, `Đã tháo ${p.name}`) }, 'Tháo'));
  }
  return h('div', { class: 'row-actions' },
    p.fstype && h('button', { class: 'btn small primary', onclick: post('/api/storage/mount', { device: p.name }, `Đã mount ${p.name}`) }, 'Mount'),
    h('button', { class: 'btn small danger', onclick: () => wipeDialog('format', p.name, p.size_bytes, reload) }, 'Format'));
}

async function pageStorage({ show }) {
  const { disks, smart } = await api('/api/storage/disks');
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
    ]), 'Ổ chưa có phân vùng nào. Bấm "Khởi tạo" để chuẩn bị ổ.'),
    [
      h('span', { class: 'tags' },
        tag(d.transport.toUpperCase()), tag(fmtBytes(d.size_bytes)), tag(d.rotational ? 'HDD' : 'SSD/Flash'),
        d.system ? tag('Thẻ hệ thống', 'warn') : smartTag(smart[d.name])),
      !d.system && h('button', { class: 'btn small', onclick: () => smartDialog(d.name) }, 'SMART'),
      !d.system && h('button', { class: 'btn small danger', onclick: () => wipeDialog('initialize', d.name, d.size_bytes, reload) }, 'Khởi tạo'),
    ],
  ));
  show(
    h('div', { class: 'page-head' }, h('h1', {}, 'Ổ đĩa'), h('button', { class: 'btn', onclick: reload }, 'Làm mới')),
    h('p', { class: 'muted' }, 'Dữ liệu NAS nên nằm trên ổ USB, thẻ nhớ chỉ chứa hệ điều hành. Ổ HDD 2.5" nên cắm qua hub USB có nguồn riêng. Ổ đã mount sẽ tự mount lại sau khi khởi động (có kiểm tra lỗi filesystem trước).'),
    blocks.length ? h('div', { class: 'grid-wide' }, blocks) : h('div', { class: 'alert' }, 'Không tìm thấy ổ đĩa nào.'),
  );
}

// ---- Files ----------------------------------------------------------------
async function pageFiles({ show, params }) {
  const path = params.get('path') || '';
  const go = (p) => { location.hash = p ? `#/files?path=${encodeURIComponent(p)}` : '#/files'; };
  const reload = () => route();

  if (params.get('trash')) return pageTrash({ show });

  const data = await api(`/api/files?path=${encodeURIComponent(path)}`);
  if (data.roots) {
    show(
      h('div', { class: 'page-head' }, h('h1', {}, 'Tệp'), h('a', { class: 'btn', href: '#/files?trash=1' }, 'Thùng rác')),
      data.roots.length
        ? h('div', { class: 'cards' }, data.roots.map((r) => h('a', { class: 'card link', href: `#/files?path=${encodeURIComponent(r.path)}` },
          h('div', { class: 'card-title' }, 'Ổ dữ liệu'), h('div', { class: 'card-value' }, r.name), meter(pct(r.used, r.total)),
          h('div', { class: 'card-sub' }, `Còn trống ${fmtBytes(r.avail)} / ${fmtBytes(r.total)}`))))
        : h('div', { class: 'alert' }, 'Chưa có ổ dữ liệu nào được mount. Vào ', h('a', { href: '#/storage' }, 'Ổ đĩa'), ' để mount.'),
    );
    return;
  }

  const crumbs = [h('a', { href: '#/files' }, 'Ổ dữ liệu')];
  const rel = data.path.slice(data.mount.length).split('/').filter(Boolean);
  let acc = data.mount;
  crumbs.push(' / ', h('a', { href: `#/files?path=${encodeURIComponent(acc)}` }, baseName(data.mount)));
  for (const seg of rel) {
    acc = joinPath(acc, seg);
    crumbs.push(' / ', h('a', { href: `#/files?path=${encodeURIComponent(acc)}` }, seg));
  }

  const selected = new Set();
  const delBtn = h('button', { class: 'btn danger', disabled: true }, 'Xóa');
  const updateSel = () => { delBtn.disabled = !selected.size; delBtn.textContent = selected.size ? `Xóa (${selected.size})` : 'Xóa'; };
  delBtn.addEventListener('click', () => confirmDialog('Chuyển vào thùng rác', `Chuyển ${selected.size} mục vào thùng rác? Có thể khôi phục trong 30 ngày.`, 'Xóa',
    () => api('/api/files/delete', { method: 'POST', body: { paths: [...selected] } })).then((ok) => { if (ok) { toast('Đã chuyển vào thùng rác'); reload(); } }));

  const fileInput = h('input', { type: 'file', multiple: true, hidden: true });
  const progress = h('div', { class: 'upload-progress' });
  fileInput.addEventListener('change', async () => {
    const files = [...fileInput.files];
    fileInput.value = '';
    for (const f of files) {
      const row = h('div', {}, h('small', {}, f.name), meter(0));
      progress.append(row);
      const url = `/api/files/upload?dir=${encodeURIComponent(data.path)}&name=${encodeURIComponent(f.name)}`;
      try {
        await uploadFile(url, f, (x) => row.replaceChildren(h('small', {}, `${f.name} — ${Math.round(x * 100)}%`), meter(x * 100)));
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
  }).then((ok) => ok && reload()) }, 'Thư mục mới');

  const rows = data.entries.map((e) => {
    const full = joinPath(data.path, e.name);
    const cb = h('input', { type: 'checkbox', 'aria-label': `Chọn ${e.name}`, onchange: (ev) => { ev.target.checked ? selected.add(full) : selected.delete(full); updateSel(); } });
    const name = e.dir
      ? h('a', { href: `#/files?path=${encodeURIComponent(full)}`, class: 'file-name' }, '📁 ', e.name)
      : h('a', { href: `/api/files/download?path=${encodeURIComponent(full)}`, class: 'file-name', download: e.name }, '📄 ', e.name);
    return [cb, h('span', {}, name, e.symlink && ' ', e.symlink && tag('link')), e.dir ? '—' : fmtBytes(e.size), fmtDate(e.mtime),
      h('div', { class: 'row-actions' },
        h('button', { class: 'btn small', onclick: () => modal(`Đổi tên ${e.name}`, [field('Tên mới', h('input', { name: 'name', value: e.name, required: true, maxlength: 255 }))], {
          ok: 'Đổi tên', submit: (fd) => api('/api/files/rename', { method: 'POST', body: { path: full, name: fd.get('name') } }),
        }).then((ok) => ok && reload()) }, 'Đổi tên'))];
  });

  show(
    h('div', { class: 'page-head' }, h('h1', {}, 'Tệp'),
      h('div', { class: 'tools' }, mkdirBtn, h('button', { class: 'btn primary', onclick: () => fileInput.click() }, 'Tải lên'), delBtn,
        h('a', { class: 'btn', href: '#/files?trash=1' }, 'Thùng rác'))),
    h('p', { class: 'crumbs' }, crumbs), fileInput, progress,
    panel(data.path, table(['', 'Tên', 'Kích thước', 'Sửa đổi', ''], [
      data.path !== data.mount && [h('span'), h('a', { href: '#', onclick: (ev) => { ev.preventDefault(); go(parentOf(data.path)); } }, '⬆ Lên thư mục cha'), '', '', ''],
      ...rows,
    ].filter(Boolean), 'Thư mục trống')),
  );
}

async function pageTrash({ show }) {
  const items = await api('/api/trash');
  const reload = () => route();
  const rows = items.map((it) => [
    h('span', {}, it.dir ? '📁 ' : '📄 ', it.name), h('code', {}, it.original), it.dir ? '—' : fmtBytes(it.size), fmtDate(it.deleted_at),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: (e) => act(e.currentTarget, () => api('/api/trash/restore', { method: 'POST', body: { mount: it.mount, id: it.id } }), 'Đã khôi phục').then((ok) => ok && reload()) }, 'Khôi phục'),
      h('button', { class: 'btn small danger', onclick: () => confirmDialog('Xóa vĩnh viễn', `Xóa vĩnh viễn "${it.name}"? Không thể hoàn tác.`, 'Xóa vĩnh viễn',
        () => api('/api/trash/purge', { method: 'POST', body: { mount: it.mount, id: it.id } })).then((ok) => ok && reload()) }, 'Xóa hẳn')),
  ]);
  show(
    h('div', { class: 'page-head' }, h('h1', {}, 'Thùng rác'),
      h('div', { class: 'tools' }, h('a', { class: 'btn', href: '#/files' }, '← Tệp'),
        items.length > 0 && h('button', { class: 'btn danger', onclick: () => confirmDialog('Dọn thùng rác', `Xóa vĩnh viễn ${items.length} mục?`, 'Dọn sạch',
          () => api('/api/trash/purge', { method: 'POST', body: {} })).then((ok) => ok && reload()) }, 'Dọn sạch'))),
    h('p', { class: 'muted' }, 'File xóa từ trang Tệp được giữ ở đây 30 ngày. (ksmbd không hỗ trợ thùng rác cho SMB: file xóa qua Windows/macOS sẽ mất ngay.)'),
    panel('Đã xóa', table(['Tên', 'Vị trí cũ', 'Kích thước', 'Xóa lúc', ''], rows, 'Thùng rác trống')),
  );
}

// ---- Shares ---------------------------------------------------------------
async function shareDialog(s, data, mounts, reload) {
  const editing = !!s;
  s = s || { name: '', path: mounts[0] ? `${mounts[0]}/share` : '/mnt/', comment: '', read_only: false, guest_ok: false, users: [], nfs: false, nfs_clients: '192.168.1.0/24' };
  const access = Object.fromEntries(s.users.map((u) => [u.name, u.access]));
  const ok = await modal(editing ? `Sửa chia sẻ ${s.name}` : 'Thêm chia sẻ', [
    field('Tên chia sẻ', h('input', { name: 'name', value: s.name, required: true, maxlength: 32, pattern: '[A-Za-z0-9_]+(-[A-Za-z0-9_]+)*', readonly: editing })),
    field('Thư mục', h('input', { name: 'path', value: s.path, required: true, list: 'mount-list' }), 'Nằm trên ổ đã mount, ví dụ /mnt/data/phim. Thư mục chưa có sẽ được tạo.'),
    h('datalist', { id: 'mount-list' }, mounts.map((m) => h('option', { value: m }))),
    field('Mô tả', h('input', { name: 'comment', value: s.comment, maxlength: 80 })),
    check('read_only', 'Mặc định chỉ đọc', s.read_only),
    check('guest_ok', 'Cho khách truy cập (không cần mật khẩu)', s.guest_ok),
    data.users.length
      ? h('fieldset', {}, h('legend', {}, 'Quyền theo người dùng (để "—" hết = mọi người dùng SMB, theo mặc định)'),
        data.users.map((u) => h('div', { class: 'acl-row' }, h('span', {}, u),
          select(`acl_${u}`, [['', '—'], ['rw', 'Đọc + ghi'], ['ro', 'Chỉ đọc']], access[u] || ''))))
      : h('p', { class: 'muted' }, 'Chưa có người dùng SMB. Hãy tạo ở bảng "Người dùng SMB" hoặc bật quyền khách.'),
    h('fieldset', {}, h('legend', {}, 'NFS (Linux, TV box, máy ảo)'),
      check('nfs', 'Chia sẻ qua NFS', s.nfs),
      field('Mạng được phép', h('input', { name: 'nfs_clients', value: s.nfs_clients || '192.168.1.0/24' }), 'IP hoặc dải mạng, cách nhau bằng dấu cách.'),
      !data.nfs_available && h('p', { class: 'alert warn' }, 'Image này chưa có NFS server.')),
  ], {
    ok: editing ? 'Lưu' : 'Tạo',
    wide: true,
    submit: (fd) => api('/api/shares', {
      method: 'POST',
      body: {
        name: fd.get('name'), path: fd.get('path'), comment: fd.get('comment') || '',
        read_only: fd.has('read_only'), guest_ok: fd.has('guest_ok'),
        users: data.users.filter((u) => fd.get(`acl_${u}`)).map((u) => ({ name: u, access: fd.get(`acl_${u}`) })),
        nfs: fd.has('nfs'), nfs_clients: fd.get('nfs_clients') || '',
      },
    }),
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
    h('strong', {}, s.name), h('code', {}, s.path), accessText(s),
    h('span', { class: 'tags' }, tag('SMB', 'ok'), s.nfs && tag('NFS', data.nfs_enabled ? 'ok' : '')),
    h('div', {}, h('code', {}, `\\\\${ip}\\${s.name}`), s.nfs && h('div', {}, h('code', {}, `${ip}:${s.path}`))),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: () => shareDialog(s, data, mounts, reload) }, 'Sửa'),
      h('button', { class: 'btn small danger', onclick: () => confirmDialog('Xóa chia sẻ', `Xóa chia sẻ "${s.name}"? Dữ liệu trong ${s.path} vẫn được giữ nguyên.`, 'Xóa',
        () => api(`/api/shares/${encodeURIComponent(s.name)}`, { method: 'DELETE' })).then((ok) => ok && reload()) }, 'Xóa')),
  ]);
  const userRows = data.users.map((u) => [
    h('strong', {}, u),
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn small', onclick: () => userDialog(u, reload) }, 'Đổi mật khẩu'),
      h('button', { class: 'btn small danger', onclick: () => confirmDialog('Xóa người dùng', `Xóa người dùng SMB "${u}"?`, 'Xóa',
        () => api(`/api/smb-users/${encodeURIComponent(u)}`, { method: 'DELETE' })).then((ok) => ok && reload()) }, 'Xóa')),
  ]);
  const nfsToggle = h('label', { class: 'check' },
    h('input', { type: 'checkbox', checked: data.nfs_enabled, disabled: !data.nfs_available,
      onchange: (e) => act(null, () => api('/api/nfs', { method: 'POST', body: { enabled: e.target.checked } }), e.target.checked ? 'Đã bật NFS' : 'Đã tắt NFS').then((ok) => ok ? reload() : (e.target.checked = !e.target.checked)) }),
    h('span', {}, 'Bật NFS server'));

  show(
    h('div', { class: 'page-head' }, h('h1', {}, 'Chia sẻ'),
      h('button', { class: 'btn primary', onclick: () => shareDialog(null, data, mounts, reload) }, '+ Thêm chia sẻ')),
    !mounts.length && h('div', { class: 'alert' }, 'Chưa có ổ dữ liệu nào được mount. Vào ', h('a', { href: '#/storage' }, 'Ổ đĩa'), ' để mount trước.'),
    panel('Thư mục chia sẻ', table(['Tên', 'Thư mục', 'Quyền', 'Giao thức', 'Địa chỉ', ''], shareRows, 'Chưa có thư mục chia sẻ nào.'), nfsToggle),
    h('div', { class: 'grid2' },
      panel('Người dùng SMB', table(['Tên', ''], userRows, 'Chưa có người dùng SMB nào.'),
        h('button', { class: 'btn small primary', onclick: () => userDialog(null, reload) }, '+ Thêm người dùng')),
      panel('Cách kết nối', h('ul', { class: 'help' },
      h('li', {}, 'Windows: File Explorer → gõ ', h('code', {}, `\\\\${ip}`)),
      h('li', {}, 'macOS: Finder → Go → Connect to Server → ', h('code', {}, `smb://${ip}`)),
      h('li', {}, 'Linux: ', h('code', {}, `smb://${ip}/<tên>`), ' hoặc NFS: ', h('code', {}, `mount -t nfs ${ip}:/mnt/<ổ>/<thư mục> /mnt/x`)),
      h('li', {}, 'Android/iOS: ứng dụng quản lý file có hỗ trợ SMB.')))),
  );
}

// ---- Network --------------------------------------------------------------
function pendingBox(p, reload) {
  if (!p) return null;
  const left = Math.max(0, p.deadline - Math.floor(Date.now() / 1000));
  return h('div', { class: 'alert warn' },
    h('strong', {}, `Thay đổi ${p.kind === 'network' ? 'mạng' : 'firewall'} đang chờ xác nhận`), ` — tự hoàn tác sau ${left} giây. `,
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
    h('p', {}, h('a', { href: url, class: 'btn primary' }, url)))));
}

async function pageNetwork({ show }) {
  const [n, fw] = await Promise.all([api('/api/network'), api('/api/firewall')]);
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

  const fwForm = h('form', {},
    check('enabled', 'Bật firewall (chặn mọi kết nối không nằm trong danh sách)', fw.enabled),
    field('Mạng được phép truy cập', h('input', { name: 'allowed', value: fw.allowed.join(' ') }), 'IP hoặc dải mạng, cách nhau bằng dấu cách. Mặc định là các dải mạng nội bộ.'),
    check('smb', 'Cho phép SMB (cổng 445)', fw.smb),
    check('nfs', 'Cho phép NFS (111, 2049, 20048)', fw.nfs),
    h('p', { class: 'muted' }, 'Giao diện web luôn được mở cho các mạng trên. Nếu bạn tự khóa mình, NAS tự hoàn tác sau 2 phút.'),
    h('button', { class: 'btn primary', type: 'submit' }, 'Áp dụng'));
  fwForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(fwForm);
    const body = { enabled: fd.has('enabled'), allowed: (fd.get('allowed') || '').split(/[\s,]+/).filter(Boolean), smb: fd.has('smb'), nfs: fd.has('nfs') };
    // If this page can still reach the NAS after the change, confirm automatically.
    const done = await act(fwForm.querySelector('button[type=submit]'), async () => {
      const r = await api('/api/firewall', { method: 'POST', body });
      await sleep(1500);
      await api('/api/network/confirm', { method: 'POST', body: { token: r.token } });
    }, 'Đã áp dụng firewall');
    if (done) reload();
  });

  show(
    h('h1', {}, 'Mạng'),
    pendingBox(n.pending, reload),
    h('div', { class: 'grid2' },
      panel('Địa chỉ IP', ipForm),
      panel('Firewall', fwForm)),
    panel('Tên máy', h('div', {}, hostForm, h('p', { class: 'muted' }, 'Tên hiển thị trong mạng Windows (NetBIOS dùng tối đa 15 ký tự đầu).'))),
    panel('Card mạng', table(['Tên', 'Trạng thái', 'IPv4', 'MAC', 'Tốc độ', 'Lưu lượng'], n.info.interfaces.map((i) => [
      h('strong', {}, i.name), i.state === 'up' ? tag('up', 'ok') : tag(i.state || 'unknown'),
      i.ipv4.length ? i.ipv4.map((a) => h('div', {}, h('code', {}, a))) : '—',
      h('code', {}, i.mac || '—'), i.speed_mbps ? `${i.speed_mbps} Mb/s` : '—',
      `↓ ${fmtBytes(i.rx_bytes)} · ↑ ${fmtBytes(i.tx_bytes)}`,
    ]))),
    panel('Định tuyến và DNS', kvList([['Gateway mặc định', n.info.gateway || '—'], ['DNS', n.info.dns.join(', ') || '—']])),
  );
}

/** Opened on the NAS's new IP after a network change (no session needed). */
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

// ---- System ---------------------------------------------------------------
async function power(action) {
  const label = action === 'reboot' ? 'Khởi động lại' : 'Tắt máy';
  const ok = await modal(label, [h('p', {}, action === 'reboot'
    ? 'NAS sẽ khởi động lại, kết nối SMB bị ngắt khoảng 1 phút.'
    : 'NAS sẽ tắt hẳn. Muốn bật lại phải rút điện rồi cắm lại.')],
  { ok: label, danger: true, submit: () => api('/api/system/power', { method: 'POST', body: { action } }) });
  if (!ok) return;
  waitForRestart(action === 'reboot' ? 'Đang khởi động lại… Trang sẽ tự tải lại khi NAS sẵn sàng.' : 'Đã gửi lệnh tắt máy. Rút điện khi đèn ACT xanh ngừng nhấp nháy.', action === 'reboot');
}

async function waitForRestart(text, poll = true, delay = 4000) {
  seq++;
  timers.forEach(clearInterval);
  timers = [];
  view.replaceChildren(h('div', { class: 'alert' }, text));
  if (!poll) return;
  await sleep(delay);
  for (;;) {
    try { await fetch('/api/auth/status'); location.reload(); return; } catch { await sleep(3000); }
  }
}

async function pageSystem({ show }) {
  const [svcs, logs, settings, upd] = await Promise.all([
    api('/api/system/services'), api('/api/system/logs'), api('/api/settings'), api('/api/system/update'),
  ]);
  const reload = () => route();
  const labels = { start: 'Chạy', restart: 'Khởi động lại', stop: 'Dừng' };
  const svcRows = svcs.map((s) => [
    s.name, s.running ? tag('đang chạy', 'ok') : tag('đã dừng', 'err'),
    s.running ? fmtBytes(s.rss_bytes) : '—', s.pids.join(', ') || '—',
    s.controllable ? h('div', { class: 'row-actions' }, Object.keys(labels).map((a) => h('button', {
      class: 'btn small',
      onclick: (e) => act(e.currentTarget, () => api(`/api/system/services/${s.id}/${a}`, { method: 'POST' }), `${s.name}: ${labels[a]}`).then((ok) => ok && setTimeout(reload, 600)),
    }, labels[a]))) : '',
  ]);

  // Admin password
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

  // HTTPS
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
      view.replaceChildren(h('div', { class: 'alert' }, 'Dịch vụ web đang khởi động lại. Mở lại tại ', h('a', { href: target }, target)));
    }
  });

  // Backup / restore
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
  const backupBox = h('div', {},
    h('p', { class: 'muted' }, 'Sao lưu chia sẻ, ổ đĩa, mạng, firewall và cài đặt. Bản "kèm mật khẩu" chứa cả mã băm mật khẩu admin và SMB: hãy cất giữ cẩn thận.'),
    h('div', { class: 'row-actions left' },
      h('a', { class: 'btn primary', href: '/api/system/backup', download: true }, 'Tải bản sao lưu'),
      h('a', { class: 'btn', href: '/api/system/backup?secrets=1', download: true }, 'Kèm mật khẩu'),
      h('button', { class: 'btn', onclick: () => restoreInput.click() }, 'Khôi phục từ file…')),
    restoreInput);

  // Update
  const updInput = h('input', { type: 'file', accept: '.tar', hidden: true });
  const updProgress = h('div');
  updInput.addEventListener('change', async () => {
    const f = updInput.files[0];
    updInput.value = '';
    if (!f) return;
    updProgress.replaceChildren(meter(0));
    try {
      const r = await uploadFile('/api/system/update', f, (x) => updProgress.replaceChildren(h('small', {}, x < 1 ? `Đang tải lên ${Math.round(x * 100)}%` : 'Đang kiểm tra và ghi vào slot dự phòng…'), meter(x * 100)));
      updProgress.replaceChildren(h('div', { class: 'alert' }, `Đã cài v${r.version} vào slot dự phòng. Khởi động lại để chạy thử; nếu lỗi, hệ thống tự quay về bản cũ.`, ' ',
        h('button', { class: 'btn small primary', onclick: () => power('reboot') }, 'Khởi động lại')));
    } catch (e) {
      updProgress.replaceChildren(h('div', { class: 'alert crit' }, e.message));
    }
  });
  const slotText = (s) => (upd.slots[s] ? `v${upd.slots[s]}` : 'chưa rõ');
  const updBox = upd.supported ? h('div', {},
    kvList([
      ['Đang chạy', `v${upd.version} (slot ${upd.current_slot?.toUpperCase()})`],
      ['Slot A', slotText('a')], ['Slot B', slotText('b')],
      ['Trạng thái', upd.trying ? tag(`đang chạy thử slot ${upd.trying.toUpperCase()}`, 'warn') : tag('ổn định', 'ok')],
    ]),
    h('div', { class: 'row-actions left' },
      h('button', { class: 'btn primary', onclick: () => updInput.click() }, 'Tải lên gói cập nhật (.tar)'),
      h('button', { class: 'btn', onclick: () => confirmDialog('Quay về bản trước', 'Lần khởi động tới sẽ dùng slot còn lại. Tiếp tục?', 'Quay về',
        () => api('/api/system/update/rollback', { method: 'POST' })).then((ok) => ok && toast('Khởi động lại để quay về bản trước')) }, 'Quay về bản trước')),
    updInput, updProgress)
    : h('p', { class: 'muted' }, `Bản đang chạy: v${upd.version}. Image này không hỗ trợ cập nhật A/B (chỉ có trên image thẻ nhớ Raspberry Pi).`);

  // Logs
  const pre = h('pre', { class: 'logs' }, logs.lines.join('\n') || '(trống)');
  const logSource = select('source', [['all', 'Tất cả'], ['rustnasd', 'RustNAS'], ['smb', 'SMB'], ['nfs', 'NFS'], ['network', 'Mạng'], ['kernel', 'Kernel']], 'all');
  const logQuery = h('input', { name: 'q', placeholder: 'Tìm…', 'aria-label': 'Tìm trong log' });
  const refreshLogs = async () => {
    const r = await api(`/api/system/logs?source=${logSource.value}&q=${encodeURIComponent(logQuery.value)}`);
    pre.textContent = r.lines.join('\n') || '(không có dòng nào)';
    pre.scrollTop = pre.scrollHeight;
  };
  logSource.addEventListener('change', refreshLogs);
  logQuery.addEventListener('keydown', (e) => { if (e.key === 'Enter') refreshLogs(); });

  show(
    h('h1', {}, 'Hệ thống'),
    panel('Dịch vụ', table(['Dịch vụ', 'Trạng thái', 'RAM', 'PID', ''], svcRows)),
    h('div', { class: 'grid2' },
      panel('Bảo mật truy cập web', sec),
      panel('Đổi mật khẩu quản trị', pw)),
    h('div', { class: 'grid2' },
      panel('Cập nhật hệ thống (A/B)', updBox),
      panel('Sao lưu và khôi phục', backupBox)),
    h('div', { class: 'grid2' },
      panel('Nguồn', h('div', {},
        h('p', { class: 'muted' }, 'Hãy tắt máy từ đây thay vì rút điện để tránh hỏng dữ liệu trên ổ USB.'),
        h('div', { class: 'row-actions left' },
          h('button', { class: 'btn', onclick: () => power('reboot') }, 'Khởi động lại'),
          h('button', { class: 'btn danger', onclick: () => power('poweroff') }, 'Tắt máy')))),
      panel('Khôi phục khi quên mật khẩu', h('ul', { class: 'help' },
        h('li', {}, 'Tắt NAS, cắm thẻ nhớ vào máy tính, mở phân vùng FAT "boot".'),
        h('li', {}, 'Tạo file trống ', h('code', {}, 'rustnas-reset-admin.txt'), ' để đặt lại tài khoản admin, hoặc ', h('code', {}, 'rustnas-factory-reset.txt'), ' để xóa toàn bộ cấu hình.'),
        h('li', {}, 'Cắm thẻ lại vào Pi và bật nguồn. Dữ liệu trên ổ USB không bị ảnh hưởng.')))),
    panel(`Nhật ký: ${logs.file}`, pre, [logSource, logQuery, h('button', { class: 'btn small', onclick: (e) => act(e.currentTarget, refreshLogs) }, 'Làm mới')]),
  );
  pre.scrollTop = pre.scrollHeight;
}

// ---- boot -----------------------------------------------------------------
$('#logout').addEventListener('click', async () => {
  try { await api('/api/auth/logout', { method: 'POST' }); } catch { /* ignore */ }
  showAuth(false);
});

async function boot() {
  const m = location.hash.match(/^#\/confirm\?token=([0-9a-f]+)/);
  try {
    status = await api('/api/auth/status');
    $('#demo-banner').hidden = !status.demo;
    if (m) return pageConfirm(m[1]);
    $('#node-name').textContent = status.hostname;
    $('#version').textContent = `v${status.version}`;
    document.title = `${status.hostname} · RustNAS`;
    if (status.setup_required) showAuth(true);
    else if (!status.user) showAuth(false);
    else showApp(status.user);
  } catch (e) {
    const box = $('#auth');
    box.hidden = false;
    box.replaceChildren(h('div', { class: 'auth-card' }, h('h1', {}, 'Không kết nối được rustnasd'), h('p', { class: 'muted' }, e.message)));
  }
}
boot();
