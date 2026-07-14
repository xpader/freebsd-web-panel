<script setup>
import { ref } from 'vue';
import { useToast, useAlert, useConfirm, useFormModal } from '../composables/useDialog.js';

const toast = useToast();
const alert = useAlert();
const confirm = useConfirm();
const formModal = useFormModal();

const logs = ref([]);
let logSeq = 0;

function log(message) {
  const now = new Date();
  const time = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}`;
  logs.value.unshift({ id: ++logSeq, time, message });
}

function fmtResult(r) {
  if (r === null || r === undefined) return 'null (已取消)';
  if (r === true) return 'true';
  if (r === false) return 'false';
  return JSON.stringify(r);
}

// ── Toast ──
function demoToastSuccess() {
  toast.toast('操作成功完成', 'success');
  log('toast → success');
}
function demoToastError() {
  toast.toast('出错了，请重试。', 'error');
  log('toast → error');
}
function demoToastInfo() {
  toast.toast('这是一条持续时间更长的提示消息', 'info', 5000);
  log('toast → info (5s)');
}

// ── Alert ──
async function demoAlert() {
  await alert('提示', '这是一个 Alert 弹窗，只有一个"确定"按钮，适合向用户展示重要信息。');
  log('alert → 已关闭 (OK)');
}

// ── Confirm ──
async function demoConfirm() {
  const r = await confirm('请确认', '确定要执行此操作吗？');
  log(`confirm → ${fmtResult(r)}`);
}

async function demoConfirmOptions() {
  const r = await confirm('请确认', '请确认并选择附加选项：', [
    { key: 'notif', label: '启用通知', checked: true },
    { key: 'email', label: '同时发送邮件通知', checked: false },
  ]);
  log(`confirm(带选项) → ${fmtResult(r)}`);
}

// ── Form: basic ──
async function demoFormBasic() {
  const r = await formModal('基础表单', [
    { key: 'name', label: '名称', type: 'text', placeholder: 'John Doe', required: true },
  ]);
  log(`form(基础) → ${fmtResult(r)}`);
}

// ── Form: all field types ──
async function demoFormAllTypes() {
  const r = await formModal('全部字段类型', [
    { key: 'username', label: '用户名', type: 'text', required: true, placeholder: 'jdoe' },
    { key: 'password', label: '密码', type: 'password', placeholder: '••••••••' },
    {
      key: 'role', label: '角色', type: 'select', required: true,
      options: [
        { value: 'admin', label: 'Admin' },
        { value: 'editor', label: 'Editor' },
        { value: 'viewer', label: 'Viewer' },
      ],
    },
    {
      key: 'gender', label: '性别', type: 'radio',
      options: [
        { value: 'male', label: '男' },
        { value: 'female', label: '女' },
        { value: 'other', label: '其他' },
      ],
    },
    { key: 'bio', label: '个人简介', type: 'textarea', rows: 4, placeholder: '介绍一下自己...' },
  ]);
  log(`form(全部类型) → ${fmtResult(r)}`);
}

// ── Form: conditional visibility (showIf) ──
async function demoFormConditional() {
  const r = await formModal('条件字段', [
    {
      key: 'serverType', label: '服务器类型', type: 'radio', required: true,
      options: [
        { value: 'web', label: 'Web 服务器' },
        { value: 'db', label: '数据库' },
        { value: 'cache', label: '缓存' },
      ],
    },
    { key: 'docRoot', label: '文档根目录', type: 'text', half: true, showIf: { serverType: 'web' }, placeholder: '/usr/local/www' },
    { key: 'phpVer', label: 'PHP 版本', type: 'select', half: true, showIf: { serverType: 'web' }, options: [
      { value: '82', label: '8.2' },
      { value: '83', label: '8.3' },
      { value: '84', label: '8.4' },
    ]},
    { key: 'dbEngine', label: '数据库引擎', type: 'select', showIf: { serverType: 'db' }, options: [
      { value: 'pgsql', label: 'PostgreSQL' },
      { value: 'mysql', label: 'MySQL' },
      { value: 'mariadb', label: 'MariaDB' },
    ]},
    { key: 'dbPort', label: '端口', type: 'text', half: true, showIf: { serverType: 'db' }, value: '5432' },
    { key: 'memSize', label: '内存 (MB)', type: 'text', half: true, showIf: { serverType: 'cache' }, value: '512' },
  ]);
  log(`form(条件字段) → ${fmtResult(r)}`);
}

// ── Form: half-width layout ──
async function demoFormHalfWidth() {
  const r = await formModal('半宽布局', [
    { key: 'firstname', label: '名', type: 'text', half: true, required: true },
    { key: 'lastname', label: '姓', type: 'text', half: true, required: true },
    { key: 'email', label: '邮箱', type: 'text', half: true, placeholder: 'user@example.com' },
    { key: 'phone', label: '电话', type: 'text', half: true, placeholder: '+1 555 0100' },
    { key: 'address', label: '地址', type: 'text' },
  ]);
  log(`form(半宽布局) → ${fmtResult(r)}`);
}

// ── Form: tooltips ──
async function demoFormTooltips() {
  const r = await formModal('带工具提示的表单', [
    { key: 'hostname', label: '主机名', type: 'text', required: true, tooltip: '服务器的完整域名或 IP 地址' },
    { key: 'port', label: '端口', type: 'text', half: true, tooltip: 'TCP 端口号 (1-65535)', value: '443' },
    { key: 'protocol', label: '协议', type: 'select', half: true, tooltip: '生产环境请使用 HTTPS', options: [
      { value: 'https', label: 'HTTPS' },
      { value: 'http', label: 'HTTP' },
    ]},
  ]);
  log(`form(工具提示) → ${fmtResult(r)}`);
}

// ── Form: async submit with inline error ──
async function demoFormAsync() {
  const r = await formModal(
    '异步提交',
    [
      { key: 'endpoint', label: 'API 端点', type: 'text', required: true, tooltip: '输入"fail"可模拟错误响应', placeholder: 'https://api.example.com' },
    ],
    {
      submitLabel: '应用',
      submitHandler: async (values) => {
        await new Promise(resolve => setTimeout(resolve, 1500));
        if (values.endpoint === 'fail') {
          throw new Error('连接失败：端点不可达');
        }
      },
    },
  );
  log(`form(异步提交) → ${fmtResult(r)}`);
}

// ── Form: initial values ──
async function demoFormInitialValues() {
  const r = await formModal('初始值', [
    { key: 'nickname', label: '昵称', type: 'text', value: 'Alice' },
    { key: 'age', label: '年龄', type: 'text', half: true, value: '30' },
    {
      key: 'beverage', label: '饮品', type: 'select', half: true, value: 'coffee',
      options: [
        { value: 'coffee', label: '咖啡' },
        { value: 'tea', label: '茶' },
        { value: 'water', label: '水' },
      ],
    },
    {
      key: 'size', label: '大小', type: 'radio', value: 'medium',
      options: [
        { value: 'small', label: '小杯' },
        { value: 'medium', label: '中杯' },
        { value: 'large', label: '大杯' },
      ],
    },
    { key: 'remarks', label: '备注', type: 'textarea', value: '无特殊备注' },
  ]);
  log(`form(初始值) → ${fmtResult(r)}`);
}

function clearLog() {
  logs.value = [];
}
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1>对话框演示</h1>
      <p>展示 DialogHost 的全部能力 — Toast、Alert、Confirm、表单</p>
    </div>

    <div class="demo-grid">
      <!-- Toast -->
      <div class="card demo-card">
        <div class="card-title">Toast 通知</div>
        <p class="text-dim demo-desc">自动消失的瞬时消息，点击可提前关闭。</p>
        <div class="btn-group">
          <button class="btn-secondary" @click="demoToastSuccess">Success Toast</button>
          <button class="btn-secondary" @click="demoToastError">Error Toast</button>
          <button class="btn-secondary" @click="demoToastInfo">Info Toast (5秒)</button>
        </div>
      </div>

      <!-- Alert -->
      <div class="card demo-card">
        <div class="card-title">Alert 弹窗</div>
        <p class="text-dim demo-desc">仅含一个"确定"按钮的模态弹窗，适合展示重要信息。</p>
        <div class="btn-group">
          <button @click="demoAlert">显示 Alert</button>
        </div>
      </div>

      <!-- Confirm -->
      <div class="card demo-card">
        <div class="card-title">Confirm 弹窗</div>
        <p class="text-dim demo-desc">确认/取消弹窗，返回 true 或 false。</p>
        <div class="btn-group">
          <button @click="demoConfirm">显示 Confirm</button>
        </div>
      </div>

      <!-- Confirm with options -->
      <div class="card demo-card">
        <div class="card-title">带选项的 Confirm</div>
        <p class="text-dim demo-desc">带额外复选框的确认弹窗，返回包含各选项状态的对象。</p>
        <div class="btn-group">
          <button @click="demoConfirmOptions">显示带选项的 Confirm</button>
        </div>
      </div>

      <!-- Form: basic -->
      <div class="card demo-card">
        <div class="card-title">基础表单</div>
        <p class="text-dim demo-desc">仅包含一个必填文本字段的简单表单。</p>
        <div class="btn-group">
          <button @click="demoFormBasic">打开基础表单</button>
        </div>
      </div>

      <!-- Form: all field types -->
      <div class="card demo-card">
        <div class="card-title">全部字段类型</div>
        <p class="text-dim demo-desc">演示所有受支持的字段类型：text、password、select、radio、textarea。</p>
        <div class="btn-group">
          <button @click="demoFormAllTypes">打开表单</button>
        </div>
      </div>

      <!-- Form: conditional visibility -->
      <div class="card demo-card">
        <div class="card-title">条件字段</div>
        <p class="text-dim demo-desc">字段根据 radio 选择显示或隐藏（showIf）。隐藏的字段不会提交。</p>
        <div class="btn-group">
          <button @click="demoFormConditional">打开条件表单</button>
        </div>
      </div>

      <!-- Form: half-width layout -->
      <div class="card demo-card">
        <div class="card-title">半宽布局</div>
        <p class="text-dim demo-desc">连续的半宽字段会并排渲染为两列。</p>
        <div class="btn-group">
          <button @click="demoFormHalfWidth">打开表单</button>
        </div>
      </div>

      <!-- Form: tooltips -->
      <div class="card demo-card">
        <div class="card-title">带工具提示的表单</div>
        <p class="text-dim demo-desc">字段可通过 FieldHelp 组件显示帮助提示。</p>
        <div class="btn-group">
          <button @click="demoFormTooltips">打开表单</button>
        </div>
      </div>

      <!-- Form: async submit -->
      <div class="card demo-card">
        <div class="card-title">异步提交</div>
        <p class="text-dim demo-desc">提交处理可以是异步的。提交时按钮显示加载指示器。出错时表单保持打开并显示行内错误。在端点输入"fail"可触发错误。</p>
        <div class="btn-group">
          <button @click="demoFormAsync">打开表单</button>
        </div>
      </div>

      <!-- Form: initial values -->
      <div class="card demo-card">
        <div class="card-title">初始值</div>
        <p class="text-dim demo-desc">表单字段可以预填初始值。</p>
        <div class="btn-group">
          <button @click="demoFormInitialValues">打开表单</button>
        </div>
      </div>

    </div>

    <!-- Interaction log -->
    <div class="card demo-log-card">
      <div class="demo-log-header">
        <div class="card-title">交互日志</div>
        <button class="btn-secondary demo-log-clear" @click="clearLog">清除</button>
      </div>
      <div v-if="logs.length === 0" class="text-dim demo-log-empty">点击上方按钮查看对话框返回结果</div>
      <div v-else class="demo-log-list">
        <div v-for="entry in logs" :key="entry.id" class="demo-log-entry">
          <span class="demo-log-time">{{ entry.time }}</span>
          <span class="demo-log-msg">{{ entry.message }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.demo-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 16px;
  margin-bottom: 24px;
}
.demo-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.demo-desc {
  font-size: 12px;
  line-height: 1.5;
  flex: 1;
}
.demo-card .btn-group {
  margin-top: 4px;
}
.demo-log-card {
  padding: 16px;
}
.demo-log-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}
.demo-log-header .card-title {
  margin-bottom: 0;
}
.demo-log-clear {
  font-size: 12px;
  padding: 4px 12px;
}
.demo-log-empty {
  font-size: 13px;
  padding: 8px 0;
}
.demo-log-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 320px;
  overflow-y: auto;
}
.demo-log-entry {
  display: flex;
  gap: 12px;
  font-size: 13px;
  font-family: monospace;
  padding: 4px 8px;
  border-radius: var(--radius);
  background: var(--bg-elev);
}
.demo-log-time {
  color: var(--text-dim);
  flex-shrink: 0;
}
.demo-log-msg {
  word-break: break-all;
}
</style>
