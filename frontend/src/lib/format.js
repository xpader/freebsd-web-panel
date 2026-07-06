// Shared formatting utilities used across pages.

import i18n from '../i18n/index.js';

export function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let i = 0;
  let v = b;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}

export function fmtRate(bps) {
  if (!bps || bps < 1) return '0 B/s';
  const u = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  let i = 0;
  let v = bps;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}

export function fmtUptime(s) {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const t = i18n.global.t.bind(i18n.global);
  return d > 0
    ? t('dash.uptimeFmtDHM', { d, h, m })
    : t('dash.uptimeFmtHM', { h, m });
}

export function fmtTime(ts) {
  if (!ts) return '—';
  const locale = i18n.global.locale.value === 'zh' ? 'zh-CN' : 'en-US';
  return new Date(ts * 1000).toLocaleString(locale);
}

export function fmtDate(ts) {
  if (!ts) return '—';
  const d = new Date(ts * 1000);
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function formatBytesTick(v) {
  if (v >= 1e9) return (v / 1e9).toFixed(0) + 'GB';
  if (v >= 1e6) return (v / 1e6).toFixed(0) + 'MB';
  if (v >= 1e3) return (v / 1e3).toFixed(0) + 'KB';
  return v + 'B';
}

export function formatRateTick(v) {
  if (v >= 1e9) return (v / 1e9).toFixed(1) + 'GB/s';
  if (v >= 1e6) return (v / 1e6).toFixed(0) + 'MB/s';
  if (v >= 1e3) return (v / 1e3).toFixed(0) + 'KB/s';
  return v + 'B/s';
}

export function fmtTooltipTime(x) {
  const d = new Date(x);
  const MM = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  const HH = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${MM}/${dd} ${HH}:${mm}`;
}

export function fmtSpeed(baudrate) {
  if (!baudrate) return '—';
  if (baudrate >= 1e9) return `${(baudrate / 1e9).toFixed(baudrate % 1e9 ? 1 : 0)} Gbps`;
  if (baudrate >= 1e6) return `${(baudrate / 1e6).toFixed(0)} Mbps`;
  if (baudrate >= 1e3) return `${(baudrate / 1e3).toFixed(0)} Kbps`;
  return `${baudrate} bps`;
}

export function fmtExpire(expire) {
  if (!expire) return null;
  const now = Math.floor(Date.now() / 1000);
  const remain = expire - now;
  if (remain <= 0) return null;
  const m = Math.floor(remain / 60);
  const s = remain % 60;
  return m > 0 ? `${m}m${s}s` : `${s}s`;
}

// Build a 9-char ls-style permission string from a numeric mode.
export function permStringFull(mode) {
  let s = '';
  s += mode & 0o400 ? 'r' : '-';
  s += mode & 0o200 ? 'w' : '-';
  s += mode & 0o4000 ? (mode & 0o100 ? 's' : 'S') : (mode & 0o100 ? 'x' : '-');
  s += mode & 0o040 ? 'r' : '-';
  s += mode & 0o020 ? 'w' : '-';
  s += mode & 0o2000 ? (mode & 0o010 ? 's' : 'S') : (mode & 0o010 ? 'x' : '-');
  s += mode & 0o004 ? 'r' : '-';
  s += mode & 0o002 ? 'w' : '-';
  s += mode & 0o1000 ? (mode & 0o001 ? 't' : 'T') : (mode & 0o001 ? 'x' : '-');
  return s;
}

export function octStr(mode) {
  return mode.toString(8).padStart(4, '0');
}
