import { describe, expect, it } from 'vitest';
import { escapeHtml, renderCardHTML, renderTranslationHTML } from './render';
import type { Translation, WordCard } from './types';

const card: WordCard = {
  found: true, word: 'run',
  phonetic: { uk: 'rʌn', us: null },
  pos: [{ pos: 'v.', gloss: ['跑', '运转'] }], definition: [],
  exchange: { past: 'ran', pp: null, ing: 'running', third: null, comparative: null, superlative: null, plural: null, lemma: null },
  collins: 3, oxford: true, tags: ['cet4'], source: 'ecdict', suggestions: [],
};

describe('escapeHtml', () => {
  it('escapes the dangerous five', () => {
    expect(escapeHtml(`<img src=x onerror="a">&'"`)).toBe(
      '&lt;img src=x onerror=&quot;a&quot;&gt;&amp;&#39;&quot;'
    );
  });
});

describe('renderCardHTML', () => {
  it('renders word, phonetic, pos lines, exchange, badges', () => {
    const h = renderCardHTML(card, true);
    expect(h).toContain('run');
    expect(h).toContain('/rʌn/');
    expect(h).toContain('v. 跑；运转');
    expect(h).toContain('ran · running');
    expect(h).toContain('柯林斯');
  });
  it('escapes hostile dictionary text', () => {
    const evil = { ...card, word: '<script>x</script>' };
    expect(renderCardHTML(evil)).not.toContain('<script>');
  });
  it('renders miss suggestions', () => {
    const miss: WordCard = { ...card, found: false, phonetic: null, pos: [], suggestions: ['running'] };
    const h = renderCardHTML(miss);
    expect(h).toContain('未收录');
    expect(h).toContain('running');
  });
});

describe('renderCardHTML badge gating', () => {
  const cardWithBadge: WordCard = card;
  it('hides badge line when showBadge=false (default)', () => {
    expect(renderCardHTML(cardWithBadge)).not.toContain('柯林斯');
    expect(renderCardHTML(cardWithBadge, false)).not.toContain('牛津核心');
  });
  it('shows badge line when showBadge=true', () => {
    expect(renderCardHTML(cardWithBadge, true)).toContain('柯林斯');
    expect(renderCardHTML(cardWithBadge, true)).toContain('四级');
  });
});

describe('renderTranslationHTML', () => {
  it('renders engine chip and text with escaping', () => {
    const t: Translation = {
      source_lang: 'en', target_lang: 'zh',
      text: '<b>hi</b>', translation: '你好', engine: 'glm',
    };
    const h = renderTranslationHTML(t);
    expect(h).toContain('glm');
    expect(h).toContain('你好');
    expect(h).not.toContain('<b>');
  });
  it('empty translation shows failure hint', () => {
    const t = { source_lang: 'en' as const, target_lang: 'zh' as const, text: 'x', translation: '', engine: '' };
    expect(renderTranslationHTML(t)).toContain('翻译失败');
  });
});
