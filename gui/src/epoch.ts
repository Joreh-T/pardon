/**
 * 翻译世代守卫：连续触发翻译时，只有最新一代允许写 UI。
 *
 * GUI 的 translate 是「发出去就等结果」的 fire-and-await 模型（daemon 不支持
 * 取消），连点两次翻译时 A、B 两路并发，返回顺序不定——迟到的一代若不设防，
 * 会把新结果覆盖掉。`doTranslate` 开始时 `begin()`，结果返回后 `isCurrent()`
 * 校验，非当代直接丢弃。
 */
export class Epoch {
  private current = 0;

  /** 开始新一代，返回该代 id。 */
  begin(): number {
    return ++this.current;
  }

  /** `id` 是否仍是最新一代（未被更晚的翻译取代）。 */
  isCurrent(id: number): boolean {
    return id === this.current;
  }
}
