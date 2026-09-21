import { describe, expect, it } from 'vitest';
import { Epoch } from './epoch';

describe('Epoch', () => {
  it('a fresh id is current', () => {
    const e = new Epoch();
    const id = e.begin();
    expect(e.isCurrent(id)).toBe(true);
  });

  it('a newer begin supersedes older ids (stale result is dropped)', () => {
    const e = new Epoch();
    const a = e.begin();
    const b = e.begin();
    expect(e.isCurrent(a)).toBe(false); // A 迟到 → 丢弃
    expect(e.isCurrent(b)).toBe(true);
  });

  it('ids strictly increase so a repeated old id can never be current', () => {
    const e = new Epoch();
    const a = e.begin();
    e.begin();
    e.begin();
    expect(e.isCurrent(a)).toBe(false);
  });

  it('last of rapid successive clicks wins', () => {
    const e = new Epoch();
    const ids = [e.begin(), e.begin(), e.begin(), e.begin()];
    expect(ids).toEqual([1, 2, 3, 4]);
    expect(e.isCurrent(ids[3])).toBe(true);
  });
});
