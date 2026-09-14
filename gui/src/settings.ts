import { invoke } from '@tauri-apps/api/core';
import { reload } from './api';
import { escapeHtml } from './render';
import { decideWrites, effectiveValue, FIELDS } from './settings-logic';
import type { FieldSpec, FormValues } from './settings-logic';
import './style.css';

interface SanitizedProvider {
  id?: string;
  type?: string;
  base_url?: string;
  model?: string;
  has_api_key?: boolean;
}

function fieldHTML(f: FieldSpec, v: unknown): string {
  const id = `f-${f.key}`;
  const control =
    f.kind === 'bool'
      ? `<input type="checkbox" id="${id}" ${v === true ? 'checked' : ''} />`
      : f.kind === 'select'
        ? `<select id="${id}">${(f.options ?? [])
            .map((o) => `<option value="${o}" ${v === o ? 'selected' : ''}>${o}</option>}`)
            .join('')}</select>`
        : `<input type="number" id="${id}" value="${Number(v ?? 0)}" />`;
  const badge = f.restart ? '<span class="restart">重启生效</span>' : '';
  const hint = f.hint ? `<span class="dim">${f.hint}</span>` : '';
  return `<label class="field">${control}<span class="field-label">${f.label}${badge}${hint}</span></label>`;
}

function providersHTML(cfg: Record<string, unknown>): string {
  const providers = (cfg.llm as Record<string, unknown> | undefined)?.providers as
    | SanitizedProvider[]
    | undefined;
  if (!providers?.length) return '<div class="empty">无 LLM provider（在 config.toml 的 [[llm.providers]] 添加）</div>';
  return providers
    .map((p) => {
      const key = p.has_api_key
        ? '<span class="key-ok">key 已配置</span>'
        : '<span class="key-miss">未配置 key</span>';
      const line = `${escapeHtml(p.id ?? '?')} · ${escapeHtml(p.type ?? '?')} · ${escapeHtml(p.model ?? '?')}`;
      const url = escapeHtml(p.base_url ?? '');
      return `<div class="provider"><div class="provider-line">${line}${key}</div><div class="provider-url">${url}</div></div>`;
    })
    .join('');
}

async function load(): Promise<void> {
  const cfg = (await invoke('read_config')) as Record<string, unknown>;
  document.getElementById('form')!.innerHTML = FIELDS.map((f) =>
    fieldHTML(f, effectiveValue(f, cfg)),
  ).join('');
  document.getElementById('providers')!.innerHTML = providersHTML(cfg);
}

async function save(): Promise<void> {
  const msg = document.getElementById('msg')!;
  const btnRestart = document.getElementById('btn-restart') as HTMLButtonElement;
  try {
    const cfg = (await invoke('read_config')) as Record<string, unknown>;
    const values: FormValues = {};
    for (const f of FIELDS) {
      const el = document.getElementById(`f-${f.key}`) as HTMLInputElement | HTMLSelectElement;
      values[f.key] = f.kind === 'bool' ? (el as HTMLInputElement).checked : el.value;
    }
    // 写入决策（缺失键以 core 默认为基准、空数字跳过）在纯函数层，单测覆盖。
    const writes = decideWrites(FIELDS, cfg, values);
    for (const w of writes) {
      await invoke('config_set', { table: w.table, key: w.key, value: w.value });
    }
    const changed = FIELDS.filter((f) => writes.some((w) => w.key === f.key));
    const { restart_required } = await reload();
    if (restart_required || changed.some((f) => f.restart)) {
      msg.textContent = '已保存。引擎相关改动需重启 pardond 生效。';
      msg.classList.remove('fail');
      btnRestart.hidden = false;
    } else {
      msg.textContent = '已保存并即时生效。';
      msg.classList.remove('fail');
      btnRestart.hidden = true;
    }
  } catch (e) {
    msg.textContent = `保存失败：${String(e)}`;
    msg.classList.add('fail');
  }
}

async function restart(): Promise<void> {
  const msg = document.getElementById('msg')!;
  const btn = document.getElementById('btn-restart') as HTMLButtonElement;
  try {
    await invoke('restart_daemon');
    msg.textContent = 'pardond 已重启。';
    msg.classList.remove('fail');
    btn.hidden = true;
  } catch (e) {
    msg.textContent = `重启失败：${String(e)}`;
    msg.classList.add('fail');
  }
}

window.addEventListener('DOMContentLoaded', () => {
  load().catch((e) => {
    const msg = document.getElementById('msg');
    if (msg) {
      msg.textContent = `读取配置失败：${String(e)}`;
      msg.classList.add('fail');
    }
  });
  document.getElementById('btn-save')?.addEventListener('click', () => void save());
  document.getElementById('btn-restart')?.addEventListener('click', () => void restart());
});
