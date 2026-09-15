// provider 编辑器的纯函数层：脱敏配置 → 行数据、表单校验/变换、
// default_provider 下拉语义。与 DOM / tauri API 零依赖，vitest 直接单测
// （settings.ts 只做渲染与 invoke 接线）。

import type { FieldSpec } from './settings-logic';

/** read_config 脱敏输出里的单个 provider（api_key 永不在读侧出现）。 */
export interface ProviderRow {
  index: number;
  id: string;
  type: string;
  base_url: string;
  model: string;
  has_api_key: boolean;
}

/** 脱敏配置 → 带下标的编辑行。providers 缺失/非数组 → []（read_config 已把
 * 非数组形状置换为 []，此处双保险）；单字段缺失/类型异常以空串/false 兜底。 */
export function toRows(cfg: Record<string, unknown>): ProviderRow[] {
  const providers = (cfg.llm as Record<string, unknown> | undefined)?.providers;
  if (!Array.isArray(providers)) return [];
  return providers.map((p, index) => {
    const o = (p ?? {}) as Record<string, unknown>;
    return {
      index,
      id: String(o.id ?? ''),
      type: String(o.type ?? ''),
      base_url: String(o.base_url ?? ''),
      model: String(o.model ?? ''),
      has_api_key: o.has_api_key === true,
    };
  });
}

/** 编辑表单值（index=null 新增；apiKey 空 = 保留现值，与 Rust 写入式语义一致）。 */
export interface ProviderForm {
  index: number | null;
  id: string;
  type: string;
  base_url: string;
  model: string;
  apiKey: string;
}

/** 与 Rust provider_upsert_at / core ProviderConfig 同规则（gui/src-tauri
 * PROVIDER_TYPES）；前置拦截，文案含字段名便于用户定位。 */
export const PROVIDER_TYPES = ['openai', 'anthropic', 'ollama'] as const;

/** 校验表单：返回错误文案，通过返回 null（与 Rust 校验同构，顺序一致）。 */
export function validateForm(f: ProviderForm): string | null {
  if (f.id === '') return 'provider id must be non-empty';
  if (f.base_url === '') return 'provider base_url must be non-empty';
  if (f.model === '') return 'provider model must be non-empty';
  if (!(PROVIDER_TYPES as readonly string[]).includes(f.type)) {
    return `unknown provider type "${f.type}", expected one of openai/anthropic/ollama`;
  }
  return null;
}

/** 表单 → provider_upsert 参数：apiKey 空 → null（Rust 侧 None = 保留现值，
 * 清除走 provider_clear_key）；provider 字段名与 ProviderInput serde 对齐。 */
export function formToUpsert(f: ProviderForm): {
  index: number | null;
  provider: { id: string; type: string; base_url: string; model: string; api_key: string | null };
} {
  return {
    index: f.index,
    provider: {
      id: f.id,
      type: f.type,
      base_url: f.base_url,
      model: f.model,
      api_key: f.apiKey === '' ? null : f.apiKey,
    },
  };
}

/** 「不设置」哨兵：下拉里代表空串（core 语义：default_provider 留空 = 不启用 LLM）。 */
export const NOT_SET = '(不设置)';

/** 写后生效对的跨字段校验（core 在 load 期做、config_set 白名单查不到的
 * 那半）：default_engine=llm 时 default_provider 必须解析到某个现有
 * provider id（空串/悬空都算不合格）。写入前拦截，避免落盘一个
 * pardond 下次启动拒绝加载的配置。返回错误文案或 null。 */
export function validateEngineProviderPair(
  engine: string,
  provider: string,
  rows: ProviderRow[],
): string | null {
  if (engine !== 'llm') return null;
  if (rows.some((r) => r.id === provider)) return null;
  return '默认引擎为 llm 时必须选择一个有效的 provider（先把 provider 加进来，或把默认引擎改为 google/youdao/bing）';
}

/** default_provider 的字段规格。**不进 FIELDS/decideWrites**：options 随
 * providers 动态生成，且「(不设置)→空串」的写语义与 decideWrites 的
 * core 默认值回落不兼容——由 settings.ts 单独读写（写入走
 * config_set(table:'llm', key:'default_provider')）。 */
export const DEFAULT_PROVIDER_FIELD: FieldSpec = {
  key: 'default_provider',
  table: 'llm',
  label: '默认 LLM provider',
  kind: 'select',
  restart: true,
  hint: '留空则不指定（默认引擎为 llm 时必须选定一个）',
};

/** 下拉 options：「(不设置)」+ 各 provider id。当前值是悬空指向（如手改
 * 配置残留、core 仅在 default_engine=llm 时才拒绝）时原样附加为选项——
 * 否则浏览器会回落选中「(不设置)」，一次无关保存就把悬空值静默清空。 */
export function defaultProviderOptions(rows: ProviderRow[], current: unknown): string[] {
  const ids = rows.map((r) => r.id);
  const opts = [NOT_SET, ...ids];
  const cur = typeof current === 'string' && current !== '' ? current : null;
  if (cur && !ids.includes(cur)) opts.push(cur);
  return opts;
}

/** 下拉选中值 → 待写值：「(不设置)」→ 空串，其余原样。 */
export function defaultProviderValue(selected: string): string {
  return selected === NOT_SET ? '' : selected;
}
