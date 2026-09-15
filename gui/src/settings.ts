import { invoke } from '@tauri-apps/api/core';
import { reload } from './api';
import { escapeHtml } from './render';
import { decideWrites, effectiveValue, FIELDS } from './settings-logic';
import type { FieldSpec, FormValues } from './settings-logic';
import {
  DEFAULT_PROVIDER_FIELD,
  PROVIDER_TYPES,
  defaultProviderOptions,
  defaultProviderValue,
  formToUpsert,
  toRows,
  validateForm,
} from './providers';
import type { ProviderForm, ProviderRow } from './providers';
import './style.css';

/** 最近一次 load 的 provider 行（行内按钮与清除 key 后重填表单都按 index 取）。 */
let rows: ProviderRow[] = [];
/** 正在编辑的 provider（null = 表单隐藏）；hasApiKey 驱动 placeholder 与清除按钮。 */
let editing: { index: number | null; hasApiKey: boolean } | null = null;

function fieldHTML(f: FieldSpec, v: unknown): string {
  const id = `f-${f.key}`;
  const control =
    f.kind === 'bool'
      ? `<input type="checkbox" id="${id}" ${v === true ? 'checked' : ''} />`
      : f.kind === 'select'
        ? `<select id="${id}">${(f.options ?? [])
            .map(
              (o) =>
                `<option value="${escapeHtml(o)}" ${v === o ? 'selected' : ''}>${escapeHtml(o)}</option>`,
            )
            .join('')}</select>`
        : `<input type="number" id="${id}" value="${Number(v ?? 0)}" />`;
  const badge = f.restart ? '<span class="restart">重启生效</span>' : '';
  const hint = f.hint ? `<span class="dim">${f.hint}</span>` : '';
  return `<label class="field">${control}<span class="field-label">${f.label}${badge}${hint}</span></label>`;
}

/** provider 列表（每行 data-index；「编辑/删除」走 #providers 容器的事件委托）。 */
function providersHTML(rs: ProviderRow[]): string {
  if (!rs.length) return '<div class="empty">无 LLM provider（点「新增 provider」添加）</div>';
  return rs
    .map((r) => {
      const key = r.has_api_key
        ? '<span class="key-ok">key 已配置</span>'
        : '<span class="key-miss">未配置 key</span>';
      const ops = `<span class="provider-ops"><button data-act="edit" data-index="${r.index}">编辑</button><button data-act="delete" data-index="${r.index}">删除</button></span>`;
      const line = `${escapeHtml(r.id)} · ${escapeHtml(r.type)} · ${escapeHtml(r.model)}`;
      const url = escapeHtml(r.base_url);
      return `<div class="provider"><div class="provider-line">${line}${key}${ops}</div><div class="provider-url">${url}</div></div>`;
    })
    .join('');
}

/** provider 编辑表单（新增/编辑共用）。api_key 只写不读：password 输入，
 * placeholder 按 has_api_key 区分（留空 = 保留现值，清除走专门按钮）。 */
function providerFormHTML(f: ProviderForm, hasApiKey: boolean): string {
  const typeOpts = PROVIDER_TYPES.map(
    (t) => `<option value="${t}" ${f.type === t ? 'selected' : ''}>${t}</option>`,
  ).join('');
  const clearBtn =
    f.index !== null && hasApiKey ? '<button id="pf-clear-key" type="button">清除已存 key</button>' : '';
  const keyPlaceholder = hasApiKey ? '已设置（留空保留）' : '未设置';
  return `
      <div class="field"><input type="text" id="pf-id" value="${escapeHtml(f.id)}" placeholder="唯一标识，default_provider 引用它" /><span class="field-label">id</span></div>
      <div class="field"><select id="pf-type">${typeOpts}</select><span class="field-label">type</span></div>
      <div class="field"><input type="text" id="pf-base-url" value="${escapeHtml(f.base_url)}" placeholder="https://api.example.com/v1" /><span class="field-label">base_url</span></div>
      <div class="field"><input type="text" id="pf-model" value="${escapeHtml(f.model)}" placeholder="模型名" /><span class="field-label">model</span></div>
      <div class="field"><input type="password" id="pf-api-key" placeholder="${keyPlaceholder}" autocomplete="new-password" /><span class="field-label">api_key（只写不回显）</span></div>
      <div class="settings-actions pf-actions"><button id="pf-save" type="button">保存 provider</button><button id="pf-cancel" type="button">取消</button>${clearBtn}</div>`;
}

function showForm(f: ProviderForm, hasApiKey: boolean): void {
  editing = { index: f.index, hasApiKey };
  const box = document.getElementById('provider-form')!;
  box.innerHTML = providerFormHTML(f, hasApiKey);
  box.hidden = false;
  document.getElementById('pf-save')?.addEventListener('click', () => void saveProvider());
  document.getElementById('pf-cancel')?.addEventListener('click', hideForm);
  document.getElementById('pf-clear-key')?.addEventListener('click', () => void clearProviderKey());
  document.getElementById('pf-id')?.focus();
}

/** 关表单即清 DOM：password 框里的值不在隐藏节点里滞留。 */
function hideForm(): void {
  editing = null;
  const box = document.getElementById('provider-form');
  if (box) {
    box.hidden = true;
    box.innerHTML = '';
  }
}

function readForm(): ProviderForm {
  const val = (id: string) => (document.getElementById(id) as HTMLInputElement | HTMLSelectElement).value;
  return {
    index: editing?.index ?? null,
    id: val('pf-id'),
    type: val('pf-type'),
    base_url: val('pf-base-url'),
    model: val('pf-model'),
    apiKey: val('pf-api-key'),
  };
}

/** provider 写操作统一收尾：reload → 按需复用重启横幅 → 成功文案 → 重渲染。 */
async function afterProviderWrite(): Promise<void> {
  const msg = document.getElementById('msg')!;
  const btnRestart = document.getElementById('btn-restart') as HTMLButtonElement;
  const { restart_required } = await reload();
  if (restart_required) {
    msg.textContent = '已保存。引擎相关改动需重启 pardond 生效。';
    msg.classList.remove('fail');
    btnRestart.hidden = false;
  } else {
    msg.textContent = '已保存并即时生效。';
    msg.classList.remove('fail');
    btnRestart.hidden = true;
  }
  await load();
}

async function saveProvider(): Promise<void> {
  const msg = document.getElementById('msg')!;
  const f = readForm();
  // 校验在纯函数层（与 Rust 同构），前端前置拦截
  const err = validateForm(f);
  if (err) {
    msg.textContent = `保存失败：${err}`;
    msg.classList.add('fail');
    return;
  }
  try {
    await invoke('provider_upsert', formToUpsert(f));
    await afterProviderWrite();
    hideForm();
  } catch (e) {
    msg.textContent = `保存失败：${String(e)}`;
    msg.classList.add('fail');
  }
}

async function deleteProvider(index: number): Promise<void> {
  const msg = document.getElementById('msg')!;
  const row = rows[index];
  if (!row || !confirm(`删除 provider "${row.id}"？`)) return;
  try {
    await invoke('provider_delete', { index });
    // 删除后下标整体前移，编辑中的表单不再可信
    hideForm();
    await afterProviderWrite();
  } catch (e) {
    msg.textContent = `删除失败：${String(e)}`;
    msg.classList.add('fail');
  }
}

/** 仅编辑模式且有已存 key 时可见（按钮随 providerFormHTML 条件渲染）。 */
async function clearProviderKey(): Promise<void> {
  const msg = document.getElementById('msg')!;
  const index = editing?.index;
  if (index === null || index === undefined) return;
  try {
    await invoke('provider_clear_key', { index });
    await afterProviderWrite();
    // 表单保持打开，按清除后的行数据重填（placeholder 转「未设置」、清除按钮消失）
    const row = rows[index];
    if (row) {
      showForm({ index, id: row.id, type: row.type, base_url: row.base_url, model: row.model, apiKey: '' }, row.has_api_key);
    } else {
      hideForm();
    }
  } catch (e) {
    msg.textContent = `清除失败：${String(e)}`;
    msg.classList.add('fail');
  }
}

async function load(): Promise<void> {
  const cfg = (await invoke('read_config')) as Record<string, unknown>;
  rows = toRows(cfg);
  // default_provider 选择追加在 default_engine 之后：options 随 providers 动态
  const dp: FieldSpec = {
    ...DEFAULT_PROVIDER_FIELD,
    options: defaultProviderOptions(rows, (cfg.llm as Record<string, unknown> | undefined)?.default_provider),
  };
  document.getElementById('form')!.innerHTML =
    FIELDS.map((f) => fieldHTML(f, effectiveValue(f, cfg))).join('') + fieldHTML(dp, effectiveValue(dp, cfg));
  document.getElementById('providers')!.innerHTML = providersHTML(rows);
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
    // default_provider 单独读写（不进 FIELDS/decideWrites：动态 options 与
    // 「(不设置)→空串」语义不合）；仅当选择 ≠ 当前值才写。
    const dpNow = String((cfg.llm as Record<string, unknown> | undefined)?.default_provider ?? '');
    const dpSel = defaultProviderValue(
      (document.getElementById(`f-${DEFAULT_PROVIDER_FIELD.key}`) as HTMLSelectElement).value,
    );
    if (dpSel !== dpNow) {
      writes.push({ table: DEFAULT_PROVIDER_FIELD.table, key: DEFAULT_PROVIDER_FIELD.key, value: dpSel });
    }
    for (const w of writes) {
      await invoke('config_set', { table: w.table, key: w.key, value: w.value });
    }
    const changed = [...FIELDS, DEFAULT_PROVIDER_FIELD].filter((f) => writes.some((w) => w.key === f.key));
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
  document.getElementById('btn-provider-add')?.addEventListener('click', () =>
    showForm({ index: null, id: '', type: 'openai', base_url: '', model: '', apiKey: '' }, false),
  );
  // 行内「编辑/删除」：列表每次写操作后整体重渲染，监听挂 #providers 容器上委托
  document.getElementById('providers')?.addEventListener('click', (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>('button[data-act]');
    if (!btn) return;
    const index = Number(btn.dataset.index);
    if (btn.dataset.act === 'edit') {
      const row = rows[index];
      if (row) {
        showForm({ index, id: row.id, type: row.type, base_url: row.base_url, model: row.model, apiKey: '' }, row.has_api_key);
      }
    } else if (btn.dataset.act === 'delete') {
      void deleteProvider(index);
    }
  });
});
