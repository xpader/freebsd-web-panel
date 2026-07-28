// Chart.js setup and shared monitor utilities.

import { Chart, registerables } from 'chart.js';
import 'chartjs-adapter-date-fns';

Chart.register(...registerables);

export { Chart };

export const CHART_COLORS = {
  blue: '#3b82f6',
  purple: '#8b5cf6',
  amber: '#f59e0b',
  green: '#22c55e',
  red: '#ef4444',
  cyan: '#06b6d4',
  pink: '#ec4899',
  indigo: '#6366f1',
  gray: '#374151',
};

export function palette(n) {
  const base = Object.values(CHART_COLORS);
  return Array.from({ length: n }, (_, i) => base[i % base.length]);
}

export function dataIsEmpty(datasets) {
  return datasets.every((d) => !d.data || d.data.length === 0);
}

export function gridColor() {
  return getComputedStyle(document.documentElement).getPropertyValue('--border').trim() || '#2a2f3a';
}
export function tickColor() {
  return getComputedStyle(document.documentElement).getPropertyValue('--text-dim').trim() || '#8b94a5';
}
export function labelColor() {
  return getComputedStyle(document.documentElement).getPropertyValue('--text').trim() || '#e4e7eb';
}

export function timeScale() {
  return {
    type: 'time',
    time: { displayFormats: { minute: 'HH:mm', hour: 'MM/dd HH:mm', day: 'MM/dd' } },
    ticks: { color: tickColor(), maxRotation: 0, autoSkip: true, maxTicksLimit: 8 },
    grid: { color: gridColor() },
  };
}

export function yScale(opts = {}) {
  return {
    min: 0,
    max: opts.yMax || undefined,
    ticks: { color: tickColor(), callback: opts.tickCb || ((v) => v + (opts.yUnit || '')) },
    grid: { color: gridColor() },
  };
}

export function chartPlugins(opts) {
  const fmtVal = opts.byteFormat ? fmtBytesRaw
    : opts.byteRateFormat ? fmtRateRaw
    : (v) => v.toFixed(1) + (opts.yUnit || '');
  return {
    legend: { labels: { color: labelColor(), font: { size: 12 } } },
    tooltip: {
      callbacks: {
        title: (items) => fmtTooltipTimeRaw(items[0].parsed.x),
        label: (c) => `${c.dataset.label}: ${fmtVal(c.parsed.y)}`,
      },
    },
  };
}

export function baseOptions(opts = {}) {
  const tickCb = opts.byteFormat ? formatBytesTick
    : opts.byteRateFormat ? formatRateTick
    : (v) => v + (opts.yUnit || '');
  return {
    responsive: true,
    maintainAspectRatio: false,
    scales: { x: timeScale(), y: yScale({ ...opts, tickCb }) },
    plugins: chartPlugins(opts),
    interaction: { mode: 'nearest', axis: 'x', intersect: false },
  };
}

function fmtTooltipTimeRaw(x) {
  const d = new Date(x);
  const MM = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  const HH = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${MM}/${dd} ${HH}:${mm}`;
}

function fmtBytesRaw(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}

function fmtRateRaw(bps) {
  if (!bps || bps < 1) return '0 B/s';
  const u = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  let i = 0;
  while (bps >= 1024 && i < u.length - 1) { bps /= 1024; i++; }
  return `${bps.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}

function formatBytesTick(v) {
  if (v >= 1e9) return (v / 1e9).toFixed(0) + 'GB';
  if (v >= 1e6) return (v / 1e6).toFixed(0) + 'MB';
  if (v >= 1e3) return (v / 1e3).toFixed(0) + 'KB';
  return v + 'B';
}

function formatRateTick(v) {
  if (v >= 1e9) return (v / 1e9).toFixed(1) + 'GB/s';
  if (v >= 1e6) return (v / 1e6).toFixed(0) + 'MB/s';
  if (v >= 1e3) return (v / 1e3).toFixed(0) + 'KB/s';
  return v + 'B/s';
}

export const NOISE_IFACE_PREFIXES = [
  'lo', 'pflog', 'pfsync', 'ipfw', 'enc', 'disc', 'edsc',
];

export function isNotNoiseIface(name) {
  return !NOISE_IFACE_PREFIXES.some((p) => name.startsWith(p));
}

// Back-compat alias — older callers still import `isPhysicalIface`.
// Semantically it's now "visible in the monitor chart", not "silicon NIC".
export const isPhysicalIface = isNotNoiseIface;
