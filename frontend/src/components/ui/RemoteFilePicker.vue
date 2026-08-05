<script setup>
import { ref, computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { api } from '../../lib/api.js';
import { createTreeState } from '../../lib/fileTree.js';
import FileTreeRow from './FileTreeRow.vue';

// Remote directory browser — a tree mirroring FilePicker, but each directory is
// listed over SSH via GET /api/rsync/browse. The host (and port) come from the
// opening field's spec, so there is no connect box: opening the picker already
// knows which host to browse. Clicking a folder expands it in place (lazy fetch),
// double-click confirms the selection — identical to the local FilePicker.

const props = defineProps({
  initialSpec: { type: String, default: '' }, // [user@]host[:/path]
  port: { type: [Number, String], default: 22 },
});
const emit = defineEmits(['select', 'close']);
const { t } = useI18n();

const selected = ref('');
const loading = ref(false);
const error = ref('');

const portVal = ref(Number(props.port) || 22);

/// Parse `[user@]host[:/path]` → { hostPart, path }. Bare host / empty path
/// default to `/`. Returns null if no host.
function parseSpec(spec) {
  const s = (spec || '').trim();
  if (!s) return null;
  let hp, p;
  const ci = s.indexOf(':');
  if (ci < 0) { hp = s; p = '/'; }
  else { hp = s.slice(0, ci); p = s.slice(ci + 1) || '/'; }
  if (!hp || hp.includes('/') || !p.startsWith('/')) return null;
  return { hostPart: hp, path: p };
}

const parsed = parseSpec(props.initialSpec);
const hostPart = ref(parsed?.hostPart || '');
// Tree root = the remote root spec, e.g. `root@host:/`.
const ROOT = computed(() => `${hostPart.value}:/`);

const { expanded, treeChildren, toggleExpand, ensureAncestors, getChildren } =
  createTreeState({
    // Fetch children of a remote spec via SSH-backed browse API.
    fetchDir: async (spec) => {
      const list = await api.get(
        `/api/rsync/browse?spec=${encodeURIComponent(spec)}&port=${portVal.value}`,
      );
      // Directory picker: keep only folders.
      return list.filter((e) => e.is_dir);
    },
    // Split a spec into ancestor specs from root down, e.g.
    // `root@h:/a/b` → [`root@h:/`, `root@h:/a`, `root@h:/a/b`].
    ancestorPaths: (spec) => {
      const ci = spec.indexOf(':');
      const hp = spec.slice(0, ci);
      const p = (spec.slice(ci + 1) || '/').replace(/\/+$/, '');
      const rootSpec = `${hp}:/`;
      const out = [rootSpec];
      let cur = '';
      for (const part of p.split('/').filter(Boolean)) {
        cur += '/' + part;
        out.push(`${hp}:${cur}`);
      }
      return out;
    },
  });

function isSelectable(e) {
  return e.is_dir;
}

function rowClick(e) {
  if (e.is_dir) {
    toggleExpand(e.path);
    selected.value = e.path;
  }
}

function rowDblClick(e) {
  if (e.is_dir) {
    selected.value = e.path;
    doSelect();
  }
}

function canConfirm() {
  return !!selected.value;
}

function doSelect() {
  if (!canConfirm()) return;
  emit('select', selected.value);
}

onMounted(async () => {
  if (!parsed) {
    error.value = t('rsync.invalidTarget');
    return;
  }
  loading.value = true;
  try {
    treeChildren.set(ROOT.value, await api.get(
      `/api/rsync/browse?spec=${encodeURIComponent(ROOT.value)}&port=${portVal.value}`,
    ).then((l) => l.filter((e) => e.is_dir)));
    expanded.add(ROOT.value);
    selected.value = ROOT.value;
    // If a deeper path was supplied, expand down to it and select it. Strip a
    // trailing slash so the selected spec matches the canonical tree node path.
    const targetPath = parsed.path.replace(/\/+$/, '');
    if (targetPath && targetPath !== '/') {
      await ensureAncestors(`${hostPart.value}:${targetPath}`);
      selected.value = `${hostPart.value}:${targetPath}`;
    }
  } catch (e) {
    error.value = e.message || t('rsync.browseError');
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="modal-overlay fp-overlay">
    <div class="modal fp-modal">
      <div class="fp-header">
        <h3>{{ t('rsync.selectRemoteDir') }}</h3>
        <div v-if="selected" class="fp-selected-path mono">{{ selected }}</div>
      </div>

      <div class="fp-tree">
        <div v-if="loading" class="empty"><span class="spinner"></span> {{ t('common.loading') }}</div>
        <div v-else-if="error" class="empty" style="color:var(--danger);">{{ error }}</div>
        <div v-else class="fp-tree-body">
          <!-- Remote root (host:/) -->
          <div class="ft-node">
            <div
              :class="['ft-row', { 'ft-active': selected === ROOT, 'ft-selectable': true }]"
              style="padding-left:6px"
              @click="selected = ROOT"
            >
              <span class="ft-arrow" @click.stop="toggleExpand(ROOT)">
                <i :class="expanded.has(ROOT) ? 'fa-solid fa-caret-down' : 'fa-solid fa-caret-right'"></i>
              </span>
              <span class="ft-name">
                <span class="ft-ico"><i class="fa-solid fa-folder-tree"></i></span>{{ ROOT }}
              </span>
            </div>
            <div v-if="expanded.has(ROOT)" class="ft-children">
              <FileTreeRow
                v-for="child in (getChildren(ROOT) || [])"
                :key="child.path"
                :entry="child"
                :depth="1"
                :expanded="expanded"
                :tree-children="treeChildren"
                :toggle-expand="toggleExpand"
                :selected-path="selected || ''"
                :is-selectable="isSelectable(child)"
                @rowclick="rowClick"
                @rowdblclick="rowDblClick"
              />
            </div>
          </div>
        </div>
      </div>

      <div class="modal-actions">
        <button class="btn-secondary" @click="emit('close')">{{ t('common.cancel') }}</button>
        <button @click="doSelect" :disabled="!canConfirm()">{{ t('common.confirm') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fp-overlay { z-index: 60; }
.fp-modal { display: flex; flex-direction: column; max-height: 80vh; min-width: 500px; max-width: 700px; }
.fp-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
.fp-selected-path {
  font-size: 12px; color: var(--text-dim); background: var(--bg-elev);
  padding: 4px 8px; border-radius: 4px; max-width: 380px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.fp-tree {
  flex: 1; overflow: auto; background: var(--input-bg);
  border: 1px solid var(--border); border-radius: var(--radius);
  min-height: 200px; max-height: 50vh;
}
.fp-tree-body { padding: 4px; }
</style>
