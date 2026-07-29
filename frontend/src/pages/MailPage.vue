<script setup>
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../lib/api.js';
import { fmtBytes } from '../lib/format.js';
import { useToast, useConfirm, useAlert } from '../composables/useDialog.js';

const { t } = useI18n();
const toast = useToast();
const confirm = useConfirm();
const alert = useAlert();

const mailboxes = ref([]);
const selectedUser = ref('');
const allMails = ref([]);
const total = ref(0);
const unread = ref(0);
const detail = ref(null);
const loading = ref(false);
const loadingMore = ref(false);
const checked = ref(new Set());
const page = ref(1);
const pageSize = 50;
const hasMore = computed(() => allMails.value.length < total.value);

const sentinel = ref(null);
let observer = null;

async function loadMailboxes() {
  try {
    const res = await api.get('/api/mail/boxes');
    mailboxes.value = res || [];
    if (!selectedUser.value && mailboxes.value.length) {
      selectedUser.value = mailboxes.value[0].user;
      await loadMails();
    }
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || '');
  }
}

async function loadMails() {
  if (!selectedUser.value) return;
  loading.value = true;
  allMails.value = [];
  checked.value = new Set();
  page.value = 1;
  try {
    const res = await api.get(`/api/mail/${selectedUser.value}?page=1&page_size=${pageSize}`);
    allMails.value = res.mails || [];
    total.value = res.total;
    unread.value = res.unread;
  } catch (e) {
    await alert(t('common.loadFailed', { msg: e.message || '' }), '');
  } finally {
    loading.value = false;
    await nextTick();
    setupObserver();
  }
}

async function loadMore() {
  if (loadingMore.value || !hasMore.value) return;
  loadingMore.value = true;
  const nextPage = page.value + 1;
  try {
    const res = await api.get(`/api/mail/${selectedUser.value}?page=${nextPage}&page_size=${pageSize}`);
    allMails.value.push(...(res.mails || []));
    total.value = res.total;
    unread.value = res.unread;
    page.value = nextPage;
  } catch (e) {
    await alert(t('common.loadFailed', { msg: e.message || '' }), '');
  } finally {
    loadingMore.value = false;
  }
}

function setupObserver() {
  if (observer) observer.disconnect();
  if (!sentinel.value) return;
  observer = new IntersectionObserver((entries) => {
    if (entries[0].isIntersecting && hasMore.value && !loadingMore.value) {
      loadMore();
    }
  }, { rootMargin: '200px' });
  observer.observe(sentinel.value);
}

function selectMailbox() {
  loadMails();
}

async function openMail(index) {
  try {
    const res = await api.get(`/api/mail/${selectedUser.value}/${index}`);
    detail.value = res;
    const m = allMails.value.find((x) => x.index === index);
    if (m && !m.read) {
      m.read = true;
      if (unread.value > 0) unread.value--;
    }
  } catch (e) {
    await alert(t('common.loadFailed', { msg: e.message || '' }), '');
  }
}

function closeDetail() {
  detail.value = null;
}

async function deleteMail(index) {
  const ok = await confirm(t('mail.deleteConfirm'), t('common.delete'));
  if (!ok) return;
  try {
    await api.del(`/api/mail/${selectedUser.value}/${index}`);
    toast.toast(t('common.delete') + ' OK', 'success');
    if (detail.value && detail.value.index === index) detail.value = null;
    allMails.value = allMails.value.filter((m) => m.index !== index);
    checked.value.delete(index);
    total.value = Math.max(0, total.value - 1);
    await loadMailboxes();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || '');
  }
}

async function batchDelete() {
  const indices = [...checked.value];
  if (!indices.length) return;
  const ok = await confirm(
    t('mail.batchDeleteConfirm', { n: indices.length }),
    t('common.delete'),
  );
  if (!ok) return;
  try {
    await api.post(`/api/mail/${selectedUser.value}/delete`, { indices });
    toast.toast(t('common.delete') + ' OK', 'success');
    allMails.value = allMails.value.filter((m) => !checked.value.has(m.index));
    checked.value = new Set();
    total.value = Math.max(0, total.value - indices.length);
    await loadMailboxes();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || '');
  }
}

async function clearMailbox() {
  const ok = await confirm(
    t('mail.clearConfirm', { user: selectedUser.value }),
    t('common.confirm'),
  );
  if (!ok) return;
  try {
    await api.del(`/api/mail/${selectedUser.value}`);
    toast.toast(t('mail.cleared'), 'success');
    detail.value = null;
    allMails.value = [];
    total.value = 0;
    unread.value = 0;
    await loadMailboxes();
  } catch (e) {
    await alert(t('common.deleteFailed'), e.message || '');
  }
}

async function toggleRead(index, read) {
  const endpoint = read ? 'read' : 'unread';
  try {
    await api.put(`/api/mail/${selectedUser.value}/${index}/${endpoint}`);
    const m = allMails.value.find((x) => x.index === index);
    if (m) {
      const wasRead = m.read;
      m.read = read;
      if (wasRead && !read) unread.value++;
      if (!wasRead && read) unread.value--;
    }
    toast.toast(read ? t('mail.markedRead') : t('mail.markedUnread'), 'success');
  } catch (e) {
    await alert(t('common.operationFailed'), e.message || '');
  }
}

function toggleCheck(index) {
  const next = new Set(checked.value);
  if (next.has(index)) next.delete(index);
  else next.add(index);
  checked.value = next;
}

function toggleCheckAll() {
  if (allMails.value.length > 0 && allMails.value.every((m) => checked.value.has(m.index))) {
    const next = new Set(checked.value);
    for (const m of allMails.value) next.delete(m.index);
    checked.value = next;
  } else {
    const next = new Set(checked.value);
    for (const m of allMails.value) next.add(m.index);
    checked.value = next;
  }
}

onMounted(loadMailboxes);
onUnmounted(() => {
  if (observer) observer.disconnect();
});
</script>

<template>
  <div class="page-header">
    <h1>{{ t('mail.title') }}</h1>
    <p>{{ t('mail.subtitle') }}</p>
  </div>

  <div class="toolbar">
    <select v-model="selectedUser" @change="selectMailbox" class="filter-input">
      <option v-for="mb in mailboxes" :key="mb.user" :value="mb.user">
        {{ mb.user }} ({{ mb.unread }}/{{ mb.total }}, {{ fmtBytes(mb.size) }})
      </option>
    </select>
    <span v-if="total" class="text-dim">
      {{ t('mail.unreadCount', { n: unread, total }) }}
    </span>
    <div class="btn-group" style="margin-left: auto;">
      <button class="btn-secondary btn-sm" @click="loadMails" :disabled="loading">{{ t('common.refresh') }}</button>
      <button
        class="btn-danger btn-sm"
        @click="batchDelete"
        :disabled="!checked.size"
      >{{ t('common.delete') }} ({{ checked.size }})</button>
      <button
        class="btn-danger btn-sm"
        @click="clearMailbox"
        :disabled="!total"
      >{{ t('mail.clearAll') }}</button>
    </div>
  </div>

  <div class="card" style="padding: 0;">
    <table>
      <thead>
        <tr>
          <th style="width: 32px;">
            <input
              type="checkbox"
              :checked="allMails.length > 0 && allMails.every((m) => checked.has(m.index))"
              @change="toggleCheckAll"
            />
          </th>
          <th style="width: 28px;"></th>
          <th>{{ t('mail.from') }}</th>
          <th>{{ t('mail.subject') }}</th>
          <th style="width: 160px;">{{ t('common.createdAt') }}</th>
          <th style="width: 70px;">{{ t('common.size') }}</th>
          <th style="width: 180px;">{{ t('common.actions') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-if="loading">
          <td colspan="7" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</td>
        </tr>
        <tr v-else-if="!allMails.length">
          <td colspan="7" class="empty">{{ t('common.noData') }}</td>
        </tr>
        <template v-else>
          <tr
            v-for="m in allMails"
            :key="m.index"
            :class="{ 'mail-unread': !m.read, 'mail-row': true }"
            @click="openMail(m.index)"
          >
            <td @click.stop>
              <input
                type="checkbox"
                :checked="checked.has(m.index)"
                @change="toggleCheck(m.index)"
              />
            </td>
            <td @click.stop>
              <i v-if="!m.read" class="fa-solid fa-circle text-dim" style="font-size: 6px;"></i>
            </td>
            <td class="text-dim" style="max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
              {{ m.from || '—' }}
            </td>
            <td style="max-width: 400px; word-break: break-word;">
              {{ m.subject || ('(' + t('mail.noSubject') + ')') }}
            </td>
            <td class="mono text-dim">{{ m.date }}</td>
            <td class="mono text-dim">{{ fmtBytes(m.size) }}</td>
            <td @click.stop>
              <div class="btn-group">
                <button
                  class="btn-secondary btn-sm"
                  @click="toggleRead(m.index, !m.read)"
                >{{ m.read ? t('mail.markUnread') : t('mail.markRead') }}</button>
                <button
                  class="btn-danger btn-sm"
                  @click="deleteMail(m.index)"
                >{{ t('common.delete') }}</button>
              </div>
            </td>
          </tr>
        </template>
      </tbody>
    </table>
    <!-- Infinite scroll sentinel -->
    <div ref="sentinel" style="padding: 12px; text-align: center;">
      <span v-if="loadingMore" class="spinner"></span>
      <span v-if="loadingMore" class="text-dim" style="margin-left: 8px;">{{ t('common.loading') }}</span>
      <span v-if="!loadingMore && !hasMore && allMails.length" class="text-dim">{{ t('common.noData') }}</span>
    </div>
  </div>

  <!-- Mail detail overlay -->
  <div v-if="detail" class="mail-overlay" @click.self="closeDetail">
    <div class="mail-detail">
      <div class="mail-detail-header">
        <h2>{{ detail.subject || ('(' + t('mail.noSubject') + ')') }}</h2>
        <div class="btn-group" style="flex-shrink: 0;">
          <button class="btn-secondary btn-sm" @click="closeDetail">{{ t('common.close') }}</button>
          <button class="btn-danger btn-sm" @click="deleteMail(detail.index)">{{ t('common.delete') }}</button>
        </div>
      </div>
      <div class="mail-detail-meta">
        <div><span class="text-dim">{{ t('mail.from') }}:</span> {{ detail.from || '—' }}</div>
        <div><span class="text-dim">{{ t('mail.to') }}:</span> {{ detail.to || '—' }}</div>
        <div><span class="text-dim">{{ t('common.createdAt') }}:</span> {{ detail.date || '—' }}</div>
      </div>
      <details class="mail-headers">
        <summary class="text-dim">{{ t('mail.allHeaders') }}</summary>
        <table class="mail-header-table">
          <tbody>
            <tr v-for="(h, i) in detail.headers" :key="i">
              <td class="text-dim">{{ h[0] }}</td>
              <td>{{ h[1] }}</td>
            </tr>
          </tbody>
        </table>
      </details>
      <pre class="mail-body">{{ detail.body }}</pre>
    </div>
  </div>
</template>

<style scoped>
.mail-unread {
  font-weight: 600;
}

.mail-row {
  cursor: pointer;
}

.mail-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: var(--modal-overlay);
  z-index: 40;
  display: flex;
  justify-content: center;
  align-items: flex-start;
  overflow-y: auto;
  padding: 40px 20px;
}

.mail-detail {
  background: var(--bg-elev);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  max-width: 900px;
  width: 100%;
  display: flex;
  flex-direction: column;
  max-height: calc(100vh - 80px);
}

.mail-detail-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  padding: 16px 20px;
  border-bottom: 1px solid var(--border);
  gap: 12px;
}

.mail-detail-header h2 {
  font-size: 16px;
  font-weight: 600;
  margin: 0;
  flex: 1 1 auto;
  min-width: 0;
  word-break: break-word;
}

.mail-detail-meta {
  padding: 12px 20px;
  border-bottom: 1px solid var(--border);
  font-size: 13px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.mail-headers {
  padding: 8px 20px;
  font-size: 12px;
}

.mail-headers summary {
  cursor: pointer;
  padding: 4px 0;
}

.mail-header-table {
  width: 100%;
  margin-top: 8px;
  font-size: 12px;
}

.mail-header-table td {
  padding: 2px 8px 2px 0;
  vertical-align: top;
  word-break: break-all;
}

.mail-header-table td:first-child {
  white-space: nowrap;
  max-width: 140px;
}

.mail-body {
  flex: 1;
  overflow: auto;
  padding: 16px 20px;
  margin: 0;
  font-family: "SF Mono", Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text);
}
</style>
