import { describe, expect, it } from 'vitest';
import {
  DEFAULT_PROVIDER_FIELD,
  NOT_SET,
  defaultProviderOptions,
  defaultProviderValue,
  formToUpsert,
  toRows,
  validateEngineProviderPair,
  validateForm,
} from './providers';

const cfg = { llm: { default_provider: 'glm', providers: [
  { id: 'glm', type: 'openai', base_url: 'https://a', model: 'm1', has_api_key: true },
  { id: 'ds', type: 'openai', base_url: 'https://b', model: 'm2', has_api_key: false },
]}};

describe('toRows', () => {
  it('maps sanitized providers with indexes', () => {
    const rows = toRows(cfg);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ index: 0, id: 'glm', has_api_key: true });
  });
  it('tolerates missing llm section', () => {
    expect(toRows({})).toEqual([]);
  });
});

describe('validateForm', () => {
  it('rejects empty fields and bad type', () => {
    expect(validateForm({ index: null, id: '', type: 'openai', base_url: 'x', model: 'y', apiKey: '' })).toContain('id');
    expect(validateForm({ index: 0, id: 'a', type: 'gpt', base_url: 'x', model: 'y', apiKey: '' })).toContain('type');
  });
  it('accepts a valid form', () => {
    expect(validateForm({ index: null, id: 'a', type: 'ollama', base_url: 'http://x', model: 'y', apiKey: '' })).toBeNull();
  });
});

describe('formToUpsert', () => {
  it('empty apiKey means keep (null), non-empty means overwrite', () => {
    expect(formToUpsert({ index: 1, id: 'a', type: 'openai', base_url: 'u', model: 'm', apiKey: '' }).provider.api_key).toBeNull();
    expect(formToUpsert({ index: 1, id: 'a', type: 'openai', base_url: 'u', model: 'm', apiKey: 'k' }).provider.api_key).toBe('k');
  });
});

describe('default_provider 下拉语义', () => {
  it('sentinel option maps to empty-string write; ids pass through', () => {
    expect(defaultProviderValue(NOT_SET)).toBe('');
    expect(defaultProviderValue('glm')).toBe('glm');
  });

  it('options = (不设置) + provider ids; dangling current kept visible', () => {
    const rows = toRows(cfg);
    expect(defaultProviderOptions(rows, undefined)).toEqual([NOT_SET, 'glm', 'ds']);
    // 悬空指向（如手改配置残留）：原样附加为选项，避免无关保存把它静默清空
    expect(defaultProviderOptions(rows, 'ghost')).toEqual([NOT_SET, 'glm', 'ds', 'ghost']);
    // 空串 = 未设置，不当作悬空值附加
    expect(defaultProviderOptions(rows, '')).toEqual([NOT_SET, 'glm', 'ds']);
  });

  it('field spec targets the llm table and is restart-flagged', () => {
    expect(DEFAULT_PROVIDER_FIELD.key).toBe('default_provider');
    expect(DEFAULT_PROVIDER_FIELD.table).toBe('llm');
    expect(DEFAULT_PROVIDER_FIELD.restart).toBe(true);
  });
});

describe('validateEngineProviderPair（写后生效对，core load 期校验的前置拦截）', () => {
  it('engine=llm + provider 命中现有行 → null', () => {
    expect(validateEngineProviderPair('llm', 'glm', toRows(cfg))).toBeNull();
    expect(validateEngineProviderPair('llm', 'ds', toRows(cfg))).toBeNull();
  });

  it('engine=llm + provider 空串/悬空/无 providers → 错误文案', () => {
    expect(validateEngineProviderPair('llm', '', toRows(cfg))).toContain('provider');
    expect(validateEngineProviderPair('llm', 'ghost', toRows(cfg))).toContain('provider');
    expect(validateEngineProviderPair('llm', 'glm', [])).toContain('provider');
  });

  it('engine 非 llm → 不约束（provider 可为空）', () => {
    expect(validateEngineProviderPair('youdao', '', toRows(cfg))).toBeNull();
    expect(validateEngineProviderPair('bing', 'ghost', [])).toBeNull();
  });
});
