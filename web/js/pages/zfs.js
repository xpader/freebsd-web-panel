// ZFS management — pools, datasets, snapshots.

import { api } from '../api.js';
import { renderLayout } from '../ui/layout.js';
import { toast } from '../ui/toast.js';
import { confirmDialog } from '../ui/confirm.js';
import { formModal } from '../ui/formModal.js';
// ===== Zpool list page =====

export async function renderZfsPools(app) {
  renderLayout(app, '/zfs/pools', `
    <div class="page-header">
      <h1>Zpool 管理</h1>
      <p>ZFS 存储池状态，点击池名查看详情</p>
    </div>
    <div id="zfs-pools"><div class="empty"><span class="spinner"></span> 加载中…</div></div>
  `);
  const el = document.getElementById('zfs-pools');
  try {
    const pools = await api.get('/api/zfs/pools');
    if (!pools.length) { el.innerHTML = '<div class="empty">无 ZFS 存储池</div>'; return; }
    el.innerHTML = pools.map(p => poolCard(p)).join('');
  } catch (e) {
    el.innerHTML = `<div class="empty">加载失败：${esc(e.message || '')}</div>`;
  }
}

function poolCard(p) {
  const pct = p.capacity_pct;
  const healthCls = p.health === 'ONLINE' ? 'badge-success' : 'badge-danger';
  return `
    <div class="card pool-card" style="cursor:pointer;" onclick="location.hash='#/zfs/pools/${esc(p.name)}'">
      <div class="flex" style="justify-content:space-between;">
        <div>
          <span class="badge ${healthCls}">${esc(p.health)}</span>
          <strong style="font-size:18px;margin-left:8px;">${esc(p.name)}</strong>
        </div>
        <span class="text-dim" style="font-size:13px;">${pct.toFixed(0)}% 已用 →</span>
      </div>
      <div class="stat-row" style="margin-top:12px;">
        <span>容量: <strong>${fmtBytes(p.size)}</strong></span>
        <span>已用: ${fmtBytes(p.allocated)}</span>
        <span>空闲: ${fmtBytes(p.free)}</span>
        <span>碎片率: ${p.fragmentation_pct.toFixed(0)}%</span>
        <span>去重: ${p.dedup.toFixed(2)}x</span>
      </div>
      <div class="bar-wrap" style="margin-top:10px;">
        <div class="bar ${pct > 80 ? 'bar-swap' : 'bar-mem'}" style="width:${pct}%"></div>
      </div>
    </div>`;
}

// ===== Zpool detail page =====

export async function renderZfsPoolDetail(app, hashPath) {
  // Extract pool name from path: /zfs/pools/{name}
  const name = hashPath.replace(/^\/zfs\/pools\//, '');

  renderLayout(app, '/zfs/pools', `
    <div class="page-header">
      <div class="flex">
        <a href="#/zfs/pools" class="btn-secondary btn-sm">← 返回</a>
        <h1 id="pool-title">存储池: ${esc(name)}</h1>
      </div>
      <p>阵列结构、磁盘状态与维护操作</p>
    </div>
    <div id="pool-detail"><div class="empty"><span class="spinner"></span> 加载中…</div></div>
  `);

  const el = document.getElementById('pool-detail');
  let info;
  try {
    info = await api.get(`/api/zfs/pools/${name}`);
  } catch (e) {
    el.innerHTML = `<div class="empty">加载失败：${esc(e.message || '')}</div>`;
    return;
  }

  const pct = info.capacity_pct || 0;
  const healthCls = healthBadge(info.health);
  const fragCls = info.fragmentation_pct > 70 ? 'badge-danger' : info.fragmentation_pct > 50 ? 'badge-warn' : 'badge-success';

  el.innerHTML = `
    <!-- Summary cards -->
    <div class="stat-grid">
      <div class="card"><div class="card-title">状态</div><div class="card-value sm"><span class="badge ${healthCls}">${esc(info.health)}</span></div></div>
      <div class="card"><div class="card-title">总容量</div><div class="card-value sm">${fmtBytes(info.size)}</div></div>
      <div class="card"><div class="card-title">已分配</div><div class="card-value sm">${fmtBytes(info.allocated)} (${pct.toFixed(0)}%)</div></div>
      <div class="card"><div class="card-title">空闲</div><div class="card-value sm">${fmtBytes(info.free)}</div></div>
      <div class="card"><div class="card-title">碎片率</div><div class="card-value sm"><span class="badge ${fragCls}">${info.fragmentation_pct.toFixed(0)}%</span></div></div>
      <div class="card"><div class="card-title">去重比</div><div class="card-value sm">${info.dedup.toFixed(2)}x</div></div>
    </div>

    <!-- Capacity bar -->
    <div class="card">
      <div class="card-title">容量使用</div>
      <div class="bar-wrap" style="height:16px;">
        <div class="bar ${pct > 80 ? 'bar-swap' : 'bar-mem'}" style="width:${pct}%"></div>
      </div>
      <div class="text-dim" style="font-size:12px;margin-top:6px;">${fmtBytes(info.allocated)} / ${fmtBytes(info.size)} (${pct.toFixed(1)}%)</div>
    </div>

    <!-- Scrub info -->
    ${info.scan ? `
    <div class="card">
      <div class="card-title">Scrub 状态</div>
      <p style="font-size:13px;">${esc(info.scan)}</p>
    </div>` : ''}

    <!-- VDEV / Disk topology -->
    <div class="card">
      <div class="card-title">阵列结构 (VDEV)</div>
      ${renderVdevTree(info.vdevs, 0)}
    </div>

    <!-- Errors -->
    ${info.error_text && !info.error_text.includes('No known') ? `
    <div class="card" style="border-color:var(--danger);">
      <div class="card-title" style="color:var(--danger);">错误信息</div>
      <p style="font-size:13px;color:var(--danger);">${esc(info.error_text)}</p>
    </div>` : ''}

    <!-- Actions -->
    <div class="card">
      <div class="card-title">维护操作</div>
      <div class="flex" style="gap:12px;">
        <button class="btn-secondary" id="btn-scrub">启动 Scrub</button>
        <button class="btn-secondary" id="btn-scrub-stop">停止 Scrub</button>
      </div>
      <p class="text-dim" style="font-size:12px;margin-top:10px;">
        Scrub 会校验池中所有数据完整性。建议每 1-3 个月执行一次。运行期间会有少量 I/O 开销。
      </p>
    </div>`;

  // Attach action buttons.
  document.getElementById('btn-scrub').onclick = async () => {
    try {
      await api.post(`/api/zfs/pools/${name}/scrub`);
      toast(`Scrub 已启动: ${name}`);
      renderZfsPoolDetail(document.getElementById('app'), hashPath);
    } catch (e) { toast(e.message || '操作失败', 'error'); }
  };
  document.getElementById('btn-scrub-stop').onclick = async () => {
    try {
      await api.post(`/api/zfs/pools/${name}/scrub/stop`);
      toast(`Scrub 已停止: ${name}`);
      renderZfsPoolDetail(document.getElementById('app'), hashPath);
    } catch (e) { toast(e.message || '操作失败', 'error'); }
  };
}

function healthBadge(health) {
  if (health === 'ONLINE') return 'badge-success';
  if (health === 'DEGRADED') return 'badge-warn';
  return 'badge-danger';
}

function renderVdevTree(vdevs, depth) {
  if (!vdevs || !vdevs.length) return '<div class="empty">无 VDEV 数据</div>';
  return vdevs.map(v => {
    const isLeaf = !v.children.length;
    const isMirror = v.name.startsWith('mirror');
    const isRaidz = v.name.startsWith('raidz');
    const vdevType = isMirror ? '镜像' : isRaidz ? 'RAID-Z' : isLeaf ? '磁盘' : 'VDEV';
    const stateCls = v.state === 'ONLINE' ? 'badge-success' : v.state === 'DEGRADED' ? 'badge-warn' : 'badge-danger';
    const hasErrors = v.read_errors > 0 || v.write_errors > 0 || v.checksum_errors > 0;
    return `
      <div class="vdev-node" style="margin-left:${depth * 24}px;">
        <div class="vdev-node-row flex">
          <span class="mono" style="font-size:13px;font-weight:${depth === 0 ? '600' : '400'};">${esc(v.name)}</span>
          <span class="badge badge-dim" style="margin:0 8px;font-size:10px;">${vdevType}</span>
          <span class="badge ${stateCls}" style="font-size:10px;">${esc(v.state)}</span>
          ${hasErrors ? `<span class="badge badge-danger" style="font-size:10px;">R:${v.read_errors} W:${v.write_errors} C:${v.checksum_errors}</span>` : ''}
        </div>
        ${v.children.length ? renderVdevTree(v.children, depth + 1) : ''}
      </div>`;
  }).join('');
}

// ===== Dataset management =====

export async function renderZfsDatasets(app) {
  renderLayout(app, '/zfs/datasets', `
    <div class="page-header">
      <h1>数据集管理</h1>
      <p>ZFS Dataset 树形视图</p>
    </div>
    <div class="toolbar">
      <div></div>
      <button onclick="window.__fwpCreateDataset()">+ 创建数据集</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>名称</th><th>类型</th><th>已用</th><th>可用</th><th>挂载点</th><th>压缩</th><th>操作</th></tr></thead>
        <tbody id="ds-tbody"><tr><td colspan="7" class="empty"><span class="spinner"></span> 加载中…</td></tr></tbody>
      </table>
    </div>
  `);
  await loadDatasets();
}

async function loadDatasets() {
  const tbody = document.getElementById('ds-tbody');
  try {
    const tree = await api.get('/api/zfs/datasets');
    const rows = [];
    function walk(ds, depth) {
      const originHtml = ds.origin
        ? `<div class="text-dim" style="font-size:11px;margin-top:2px;">⤷ 克隆自 <span class="mono" style="color:var(--accent);">${esc(ds.origin)}</span></div>`
        : '';
      rows.push(`
        <tr>
          <td class="mono" style="padding-left:${depth * 20 + 12}px;">
            ${depth > 0 ? '└ ' : ''}<strong>${esc(ds.name)}</strong>
            ${originHtml}
          </td>
          <td><span class="badge badge-dim">${esc(ds.typ)}</span></td>
          <td class="mono">${fmtBytes(ds.used)}</td>
          <td class="mono">${fmtBytes(ds.available)}</td>
          <td class="mono">${esc(ds.mountpoint)}</td>
          <td class="mono">${esc(ds.compression)}</td>
          <td>
            <button class="btn-secondary btn-sm" onclick="window.__fwpDsSnap('${esc(ds.name)}')">快照</button>
            <button class="btn-secondary btn-sm" onclick="window.__fwpDsProps('${esc(ds.name)}')">属性</button>
            ${ds.name.includes('/') ? `<button class="btn-danger btn-sm" onclick="window.__fwpDelDs('${esc(ds.name)}')">删除</button>` : ''}
          </td>
        </tr>`);
      ds.children.forEach(c => walk(c, depth + 1));
    }
    tree.forEach(ds => walk(ds, 0));
    tbody.innerHTML = rows.join('') || '<tr><td colspan="7" class="empty">无数据集</td></tr>';
  } catch (e) {
    tbody.innerHTML = `<tr><td colspan="7" class="empty">加载失败：${esc(e.message || '')}</td></tr>`;
  }
}

window.__fwpCreateDataset = async () => {
  const result = await formModal('创建数据集', [
    { key: 'name', label: '数据集名称', placeholder: '如 zroot/newds', required: true },
  ]);
  if (!result) return;
  api.post('/api/zfs/datasets', { name: result.name }).then(() => {
    toast('数据集已创建');
    loadDatasets();
  }).catch(e => toast(e.message || '创建失败', 'error'));
};

window.__fwpDsSnap = async (name) => {
  const result = await formModal(`创建快照: ${name}`, [
    { key: 'name', label: '快照名称', placeholder: '如 backup-20260626', required: true },
  ]);
  if (!result) return;
  api.post('/api/zfs/snapshots', { dataset: name, name: result.name }).then(() => {
    toast(`快照已创建: ${name}@${result.name}`);
  }).catch(e => toast(e.message || '创建快照失败', 'error'));
};

window.__fwpDelDs = async (name) => {
  if (!await confirmDialog('删除数据集', `确定删除 "${name}" 及其所有子项？此操作不可撤销。`)) return;
  api.del(`/api/zfs/dataset/destroy?name=${encodeURIComponent(name)}`).then(() => {
    toast('数据集已删除');
    loadDatasets();
  }).catch(e => toast(e.message || '删除失败', 'error'));
};

window.__fwpDsProps = async (name) => {
  let props;
  try { props = await api.get(`/api/zfs/dataset/properties?name=${encodeURIComponent(name)}`); }
  catch (e) { toast(e.message || '加载属性失败', 'error'); return; }

  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal" style="max-width:600px;">
      <h3>属性: ${esc(name)}</h3>
      <div style="max-height:400px;overflow-y:auto;">
        <table style="font-size:12px;">
          <thead><tr><th>属性</th><th>值</th><th>来源</th></tr></thead>
          <tbody>
            ${props.map(p => `<tr><td class="mono">${esc(p.name)}</td><td class="mono">${esc(p.value)}</td><td class="text-dim mono">${esc(p.source)}</td></tr>`).join('')}
          </tbody>
        </table>
      </div>
      <div class="modal-actions">
        <button class="btn-secondary" data-act="close">关闭</button>
      </div>
    </div>`;
  document.body.appendChild(overlay);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay || e.target.dataset.act === 'close') overlay.remove();
  });
};

// ===== Snapshot management =====

export async function renderZfsSnapshots(app) {
  renderLayout(app, '/zfs/snapshots', `
    <div class="page-header">
      <h1>快照管理</h1>
      <p>ZFS 快照列表与操作</p>
    </div>
    <div class="toolbar">
      <input type="text" id="snap-filter" class="search" placeholder="过滤数据集名称…" oninput="window.__fwpSnapFilter()" />
      <button onclick="window.__fwpCreateSnap()">+ 创建快照</button>
    </div>
    <div class="card" style="padding:0;">
      <table>
        <thead><tr><th>数据集</th><th>快照</th><th>已用</th><th>引用大小</th><th>创建时间</th><th>操作</th></tr></thead>
        <tbody id="snap-tbody"><tr><td colspan="6" class="empty"><span class="spinner"></span> 加载中…</td></tr></tbody>
      </table>
    </div>
  `);
  await loadSnapshots();
}

let _allSnaps = [];

async function loadSnapshots() {
  const tbody = document.getElementById('snap-tbody');
  try {
    _allSnaps = await api.get('/api/zfs/snapshots');
    renderSnapRows(_allSnaps);
  } catch (e) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">加载失败：${esc(e.message || '')}</td></tr>`;
  }
}

function renderSnapRows(snaps) {
  const tbody = document.getElementById('snap-tbody');
  if (!snaps.length) { tbody.innerHTML = '<tr><td colspan="6" class="empty">无快照</td></tr>'; return; }
  tbody.innerHTML = snaps.map(s => `
    <tr>
      <td class="mono">${esc(s.dataset)}</td>
      <td class="mono">${esc(s.snap_name)}</td>
      <td class="mono">${fmtBytes(s.used)}</td>
      <td class="mono">${fmtBytes(s.referenced)}</td>
      <td class="text-dim mono">${fmtTime(s.creation)}</td>
      <td>
        <button class="btn-secondary btn-sm" onclick="window.__fwpCloneSnap('${esc(s.name)}')">克隆</button>
        <button class="btn-secondary btn-sm" onclick="window.__fwpRollback('${esc(s.name)}')">回滚</button>
        <button class="btn-danger btn-sm" onclick="window.__fwpDelSnap('${esc(s.name)}')">删除</button>
      </td>
    </tr>`).join('');
}

window.__fwpSnapFilter = () => {
  const q = (document.getElementById('snap-filter')?.value || '').toLowerCase();
  renderSnapRows(_allSnaps.filter(s => s.dataset.toLowerCase().includes(q) || s.snap_name.toLowerCase().includes(q)));
};


window.__fwpCreateSnap = async () => {
  const result = await formModal('创建快照', [
    { key: 'dataset', label: '数据集', placeholder: '如 zroot/data', required: true },
    { key: 'name', label: '快照名称', placeholder: '如 backup-20260626', required: true },
  ]);
  if (!result) return;
  api.post('/api/zfs/snapshots', { dataset: result.dataset, name: result.name }).then(() => {
    toast('快照已创建'); loadSnapshots();
  }).catch(e => toast(e.message || '创建失败', 'error'));
};

window.__fwpCloneSnap = async (source) => {
  const result = await formModal(`克隆快照: ${source}`, [
    { key: 'target', label: '目标数据集名称', placeholder: '如 zroot/new-clone', required: true },
  ]);
  if (!result) return;
  api.post('/api/zfs/snapshot/clone', { source, target: result.target }).then(() => {
    toast(`克隆成功: ${result.target}`); loadSnapshots();
  }).catch(e => toast(e.message || '克隆失败', 'error'));
};

window.__fwpDelSnap = async (full) => {
  if (!await confirmDialog('删除快照', `确定删除 "${full}" 吗？`)) return;
  api.del(`/api/zfs/snapshot/destroy?name=${encodeURIComponent(full)}`).then(() => {
    toast('快照已删除'); loadSnapshots();
  }).catch(e => toast(e.message || '删除失败', 'error'));
};

window.__fwpRollback = async (full) => {
  if (!await confirmDialog('回滚快照', `确定回滚到 "${full}" 吗？\n\n警告：此操作将销毁该快照之后的所有数据和新快照！`)) return;
  api.post(`/api/zfs/snapshot/rollback?name=${encodeURIComponent(full)}`, { confirm: true }).then(() => {
    toast('回滚成功'); loadSnapshots();
  }).catch(e => toast(e.message || '回滚失败', 'error'));
};

function fmtBytes(b) {
  if (!b) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i < 2 ? 0 : 1)} ${u[i]}`;
}
function fmtTime(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleString('zh-CN');
}
function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}
