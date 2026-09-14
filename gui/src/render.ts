import type { HistoryEntry, Translation, WordCard } from './types';

export function escapeHtml(s: string): string {
  return s
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

// daemon 侧 WordCard 的 Option/Vec 字段带 skip_serializing_if，键缺席时
// 运行时是 undefined 而非 null——数组类字段一律 `?? []`，可空字段用 `?.`。
const phoneticLine = (c: WordCard): string => {
  const p = c.phonetic?.uk || c.phonetic?.us;
  return p ? `<div class="phonetic">/${escapeHtml(p)}/</div>` : '';
};

const posLines = (c: WordCard): string =>
  (c.pos ?? [])
    .map((pg) => {
      // pos 为纯文本前缀（不包 span）：渲染出的可见文本是「v. 释义」连续串
      const pos = pg.pos ? `${escapeHtml(pg.pos)} ` : '';
      return `<div class="pos-line">${pos}${(pg.gloss ?? []).map(escapeHtml).join('；')}</div>`;
    })
    .join('');

const exchangeLine = (c: WordCard): string => {
  const ex = c.exchange;
  if (!ex) return '';
  const forms = [ex.past, ex.ing, ex.third, ex.plural, ex.comparative, ex.superlative]
    .filter((f): f is string => !!f)
    .filter((f, i, a) => a.indexOf(f) === i)
    .slice(0, 4);
  return forms.length ? `<div class="exchange">词形：${forms.map(escapeHtml).join(' · ')}</div>` : '';
};

const badgeLine = (c: WordCard): string => {
  const parts: string[] = [];
  if (c.collins && c.collins > 0) parts.push(`柯林斯 ${'★'.repeat(Math.min(c.collins, 5))}`);
  if (c.oxford) parts.push('牛津核心');
  const tagLabel: Record<string, string> = {
    zk: '中考', gk: '高考', cet4: '四级', cet6: '六级', ky: '考研',
    toefl: '托福', ielts: '雅思', gre: 'GRE',
  };
  parts.push(...(c.tags ?? []).map((t) => tagLabel[t] ?? t));
  return parts.length ? `<div class="badge">${parts.map(escapeHtml).join(' · ')}</div>` : '';
};

export function renderCardHTML(c: WordCard): string {
  if (!c.found) {
    const sug = (c.suggestions ?? []).length
      ? `<div class="suggest">未收录；试试：${(c.suggestions ?? []).slice(0, 3).map(escapeHtml).join('、')}</div>`
      : '<div class="suggest">未收录</div>';
    return `<section class="card miss">${sug}</section>`;
  }
  return `<section class="card">
    <h2 class="word">${escapeHtml(c.word)}</h2>
    ${phoneticLine(c)}
    ${posLines(c)}
    ${(c.definition ?? []).map((d) => `<div class="def">${escapeHtml(d)}</div>`).join('')}
    ${exchangeLine(c)}
    ${badgeLine(c)}
  </section>`;
}

export function renderTranslationHTML(t: Translation): string {
  const chip = t.engine ? `<span class="chip">${escapeHtml(t.engine)}</span>` : '';
  const body = t.translation
    ? `<div class="trans-text">${escapeHtml(t.translation)}</div>`
    : '<div class="trans-text fail">翻译失败（引擎链全部失败）</div>';
  return `<section class="sentence"><div class="row">${chip}<span class="src">${escapeHtml(t.text)}</span></div>${body}</section>`;
}

export function renderHistoryHTML(entries: HistoryEntry[]): string {
  if (!entries.length) return '<div class="empty">暂无历史</div>';
  return entries
    .map(
      (e, i) => `
      <div class="hist-item" data-idx="${i}">
        <span class="hist-text">${escapeHtml(e.text)}</span>
        <span class="hist-trans">${escapeHtml(e.translation)}</span>
      </div>`
    )
    .join('');
}
