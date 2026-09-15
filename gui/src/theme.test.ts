import { describe, expect, it } from 'vitest';
import { normalizeTheme, THEMES } from './theme';

describe('normalizeTheme', () => {
  it('passes valid names through', () => {
    for (const t of ['auto', 'catppuccin-mocha', 'catppuccin-latte', 'macos-light', 'nord'] as const) {
      expect(normalizeTheme(t)).toBe(t);
    }
  });
  it('falls back to auto on null/garbage', () => {
    expect(normalizeTheme(null)).toBe('auto');
    expect(normalizeTheme('dark-mode' as never)).toBe('auto');
    expect(normalizeTheme('')).toBe('auto');
  });
});

describe('THEMES', () => {
  it('has auto plus four presets, in order', () => {
    expect(THEMES.map((t) => t.name)).toEqual(['auto', 'catppuccin-mocha', 'catppuccin-latte', 'macos-light', 'nord']);
  });
});
