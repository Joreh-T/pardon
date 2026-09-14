// 设置窗口的纯逻辑层：字段规格 + 写入决策。与 DOM / tauri API 零依赖，
// vitest 直接单测（settings.ts 只做渲染与 invoke——见 settings.test.ts）。

export interface FieldSpec {
  key: string;
  table: 'daemon' | null;
  label: string;
  kind: 'bool' | 'number' | 'select';
  options?: string[]; // kind === 'select'
  restart?: boolean;
  hint?: string;
}

/** 表单值：bool → checked（boolean），number/select → el.value（string）。 */
export type FormValues = Record<string, boolean | string>;

/** 单条待写入项（table=null 为根表）。 */
export interface WriteOp {
  table: FieldSpec['table'];
  key: string;
  value: boolean | number | string;
}

// 键/表与 Rust 侧 WRITABLE 白名单一一对应（多写会被 config_set 拒绝）。
export const FIELDS: FieldSpec[] = [
  { key: 'auto_translate', table: 'daemon', label: '复制即翻译（剪贴板自动监听）', kind: 'bool' },
  { key: 'popup', table: 'daemon', label: '翻译结果用 GUI 弹窗展示（无 GUI 时回退通知）', kind: 'bool' },
  { key: 'show_word_badge', table: 'daemon', label: '词卡显示学习徽章行', kind: 'bool' },
  { key: 'copy_translation', table: 'daemon', label: '译文自动写回剪贴板', kind: 'bool' },
  { key: 'notify_timeout_ms', table: 'daemon', label: '通知显示时长（毫秒）', kind: 'number' },
  { key: 'max_text_bytes', table: 'daemon', label: '自动翻译文本上限（字节）', kind: 'number' },
  { key: 'dedup_window_ms', table: 'daemon', label: '同内容去重窗口（毫秒）', kind: 'number' },
  { key: 'default_engine', table: null, label: '默认引擎', kind: 'select', options: ['llm', 'youdao', 'bing'], restart: true, hint: '重启 pardond 后生效' },
];

/** 字段默认值，对齐 core DaemonConfig / 根表默认（crates/core/src/config.rs；
 * DEFAULT_CONFIG_TOML 不写 [daemon] 段——全新配置里这些键全部缺席）。 */
export const FIELD_DEFAULTS: Record<string, boolean | number | string> = {
  auto_translate: true,
  popup: false,
  show_word_badge: false,
  copy_translation: false,
  notify_timeout_ms: 5000,
  max_text_bytes: 5120,
  dedup_window_ms: 10000,
  default_engine: 'youdao',
};

/** 配置文件里的当前值（键缺失 → undefined）。 */
export const currentValue = (f: FieldSpec, cfg: Record<string, unknown>): unknown =>
  f.table ? (cfg[f.table] as Record<string, unknown> | undefined)?.[f.key] : cfg[f.key];

/** 生效值：配置值缺失时回落 core 默认（渲染基准）。 */
export const effectiveValue = (f: FieldSpec, cfg: Record<string, unknown>): unknown =>
  currentValue(f, cfg) ?? FIELD_DEFAULTS[f.key];

/**
 * 决定要写哪些键：仅当表单值 ≠（配置值 ?? core 默认）才写（按字符串统一
 * 比较：checkbox → 'true'/'false'，number → '5000'）。缺失键以 core 默认
 * 为基准——否则全新配置（无 [daemon] 段）一次保存会把 max_text_bytes
 * 等写成 0，core 校验拒绝后 pardond 无法启动。
 * 数字输入清空 → 跳过该键（不写 0/NaN）。
 */
export function decideWrites(
  fields: FieldSpec[],
  cfg: Record<string, unknown>,
  values: FormValues,
): WriteOp[] {
  const ops: WriteOp[] = [];
  for (const f of fields) {
    const now = values[f.key];
    if (f.kind === 'number' && typeof now === 'string' && now.trim() === '') continue;
    const cur = currentValue(f, cfg) ?? FIELD_DEFAULTS[f.key];
    if (String(now) !== String(cur)) {
      ops.push({ table: f.table, key: f.key, value: f.kind === 'number' ? Number(now) : now });
    }
  }
  return ops;
}
