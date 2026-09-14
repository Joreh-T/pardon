import { invoke } from '@tauri-apps/api/core';
import { reload } from './api';
import { escapeHtml } from './render';
import './style.css';

interface FieldSpec {
  key: string;
  table: 'daemon' | null;
  label: string;
  kind: 'bool' | 'number' | 'select';
  options?: string[]; // kind === 'select'
  restart?: boolean;
  hint?: string;
}

// 键/表与 Rust 侧 WRITABLE 白名单一一对应（多写会被 config_set 拒绝）。
const FIELDS: FieldSpec[] = [
  { key: 'auto_translate', table: 'daemon', label: '复制即翻译（剪贴板自动监听）', kind: 'bool' },
  { key: 'popup', table: 'daemon', label: '翻译结果用 GUI 弹窗展示（无 GUI 时回退通知）', kind: 'bool' },
  { key: 'show_word_badge', table: 'daemon', label: '词卡显示学习徽章行', kind: 'bool' },
  { key: 'copy_translation', table: 'daemon', label: '译文自动写回剪贴板', kind: 'bool' },
  { key: 'notify_timeout_ms', table: 'daemon', label: '通知显示时长（毫秒）', kind: 'number' },
  { key: 'max_text_bytes', table: 'daemon', label: '自动翻译文本上限（字节）', kind: 'number' },
  { key: 'dedup_window_ms', table: 'daemon', label: '同内容去重窗口（毫秒）', kind: 'number' },
  { key: 'default_engine', table: null, label: '默认引擎', kind: 'select', options: ['llm', 'youdao', 'bing'], restart: true, hint: '重启 pardond 后生效' },
];

interface SanitizedProvider {
  id?: string;
  type?: string;
  base_url?: string;
  model?: string;
  has_api_key?: boolean;
}

const current = (f: FieldSpec, cfg: Record<string, unknown>): unknown =>
  f.table ? (cfg[f.table] as Record<string, unknown>)?.[f.key] : cfg[f.key];

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
    fieldHTML(f, current(f, cfg)),
  ).join('');
  document.getElementById('providers')!.innerHTML = providersHTML(cfg);
}

async function save(): Promise<void> {
  const msg = document.getElementById('msg')!;
  const btnRestart = document.getElementById('btn-restart') as HTMLButtonElement;
  try {
    const cfg = (await invoke('read_config')) as Record<string, unknown>;
    const changed: FieldSpec[] = [];
    for (const f of FIELDS) {
      const el = document.getElementById(`f-${f.key}`) as HTMLInputElement | HTMLSelectElement;
      const now = f.kind === 'bool' ? (el as HTMLInputElement).checked : el.value;
      // 统一按字符串比较（checkbox → 'true'/'false'，number → '5000'）
      if (String(now) !== String(current(f, cfg) ?? '')) {
        const value = f.kind === 'number' ? Number(now) : now;
        await invoke('config_set', { table: f.table, key: f.key, value });
        changed.push(f);
      }
    }
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
