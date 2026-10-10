// Shared supervisor create/edit form, used by both the list page and the
// detail page so the field set and submit mapping stay in one place.

import { api } from './api.js';

export function openSupervisorForm(formModal, t, toast, existing = null, onSaved = null) {
  const isEdit = !!existing;
  return formModal(
    isEdit ? t('common.edit') : t('common.create'),
    [
      { key: 'name', label: t('common.name'), value: existing?.name || '', required: true, placeholder: 'my-daemon' },
      { key: 'path', label: t('supv.path'), value: existing?.path || '', required: true, placeholder: '/usr/local/bin/myapp', picker: 'file' },
      { key: 'args', label: t('supv.args'), type: 'textarea', value: (existing?.args || []).join('\n'), placeholder: '-c\n/etc/myapp.conf', help: t('supv.argsHint') },
      { key: 'workdir', label: t('supv.workdir'), value: existing?.workdir || '', placeholder: '/var/tmp', picker: 'dir', half: true },
      { key: 'user', label: t('supv.runUser'), value: existing?.user || '', placeholder: 'root', help: t('supv.runUserHint'), half: true },
      { key: 'env', label: t('supv.envVars'), type: 'textarea', value: (existing?.env || []).join('\n'), placeholder: 'LANG=C\nHOME=/var/empty', help: t('supv.envHint') },
      {
        key: '_log_flags', label: '', type: 'checkbox-group',
        options: [
          { key: 'logging', label: t('supv.logging'), value: existing ? !!existing.logging : true },
        ],
      },
      { key: 'log_file', label: t('supv.logFile'), value: existing?.log_file || '', placeholder: '/var/log/myapp.log', picker: 'file', showIf: { logging: true }, hint: t('supv.logFileHint') },
      {
        key: '_log_rotate_flags', label: '', type: 'checkbox-group', showIf: { logging: true },
        options: [
          { key: 'log_rotate', label: t('supv.logRotate'), value: existing ? !!existing.log_rotate : true, help: t('supv.logRotateHint') },
        ],
      },
      {
        key: '_start_flags', label: '', type: 'checkbox-group',
        options: [
          { key: 'autostart', label: t('supv.autostart'), value: existing ? !!existing.autostart : false, help: t('supv.autostartHint') },
        ],
      },
      {
        key: '_restart_flags', label: '', type: 'checkbox-group',
        options: [
          { key: 'restart', label: t('supv.autoRestart'), value: existing ? !!existing.restart : true, help: t('supv.autoRestartHint') },
        ],
      },
      { key: 'restart_delay', label: t('supv.restartDelay'), inputType: 'number', value: existing?.restart_delay ?? 1, half: true, showIf: { restart: true } },
      { key: 'restart_max', label: t('supv.restartMax'), inputType: 'number', value: existing?.restart_max ?? '', placeholder: '∞', half: true, showIf: { restart: true }, hint: t('supv.restartMaxHint') },
    ],
    {
      submitLabel: isEdit ? t('common.save') : t('common.create'),
      submitHandler: async (r) => {
        const body = {
          name: r.name.trim(),
          path: r.path.trim(),
          args: (r.args || '').split('\n').map((s) => s.trim()).filter(Boolean),
          workdir: r.workdir?.trim() || '',
          user: r.user?.trim() || '',
          env: (r.env || '').split('\n').map((s) => s.trim()).filter(Boolean),
          logging: !!r.logging,
          log_file: r.log_file?.trim() || '',
          log_rotate: !!r.log_rotate,
          autostart: !!r.autostart,
          restart_delay: r.restart ? (parseInt(r.restart_delay, 10) || 1) : 1,
          restart_max: r.restart && r.restart_max !== '' && r.restart_max != null
            ? parseInt(r.restart_max, 10)
            : null,
        };
        if (isEdit) {
          await api.put(`/api/supervisor/${encodeURIComponent(existing.id)}`, body);
          toast.toast(t('supv.updated', { name: body.name }));
        } else {
          await api.post('/api/supervisor', body);
          toast.toast(t('supv.created', { name: body.name }));
        }
        if (onSaved) await onSaved();
      },
    },
  );
}
