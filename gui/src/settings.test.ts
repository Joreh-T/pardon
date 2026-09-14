import { describe, expect, it } from 'vitest';
import { FIELD_DEFAULTS, decideWrites, effectiveValue, FIELDS } from './settings-logic';
import type { FormValues } from './settings-logic';

/** 全默认表单值：bool → checkbox 勾选态，number/select → 输入框字符串。 */
const defaultForm = (): FormValues => ({
  auto_translate: true,
  popup: false,
  show_word_badge: false,
  copy_translation: false,
  notify_timeout_ms: String(FIELD_DEFAULTS.notify_timeout_ms),
  max_text_bytes: String(FIELD_DEFAULTS.max_text_bytes),
  dedup_window_ms: String(FIELD_DEFAULTS.dedup_window_ms),
  default_engine: String(FIELD_DEFAULTS.default_engine),
});

describe('decideWrites', () => {
  // C1 回归：DEFAULT_CONFIG_TOML 不写 [daemon] 段——全新配置（cfg = {}）
  // 所有 daemon 键缺席。表单显示的是 core 默认值，用户原样保存（未改任何
  // 字段）必须零写入；否则会把 max_text_bytes=0 等非法值写进配置文件。
  it('fresh config + all-default form values → zero writes', () => {
    const writes = decideWrites(FIELDS, {}, defaultForm());
    expect(writes).toEqual([]);
  });

  it('cleared number input → that key is skipped', () => {
    const values = { ...defaultForm(), max_text_bytes: '' };
    const writes = decideWrites(FIELDS, { daemon: { max_text_bytes: 5120 } }, values);
    expect(writes).toEqual([]);
  });

  it('one changed bool → exactly one write with table/key/value', () => {
    const values = { ...defaultForm(), auto_translate: false };
    const writes = decideWrites(FIELDS, {}, values);
    expect(writes).toEqual([{ table: 'daemon', key: 'auto_translate', value: false }]);
  });

  // 已有合法配置值时，默认回落不得遮蔽真实值（缺失才回落）。
  it('explicit config value wins over field default', () => {
    const values = { ...defaultForm(), notify_timeout_ms: '5000' };
    const writes = decideWrites(FIELDS, { daemon: { notify_timeout_ms: 8000 } }, values);
    expect(writes).toEqual([
      { table: 'daemon', key: 'notify_timeout_ms', value: 5000 },
    ]);
  });
});

describe('effectiveValue', () => {
  it('falls back to core defaults for missing keys', () => {
    expect(effectiveValue(FIELDS[0], {})).toBe(true); // auto_translate
    expect(effectiveValue(FIELDS[4], {})).toBe(5000); // notify_timeout_ms
  });

  it('keeps explicit config values', () => {
    expect(effectiveValue(FIELDS[0], { daemon: { auto_translate: false } })).toBe(false);
  });
});
