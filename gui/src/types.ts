// 与 pardon-core serde 输出一一对应（UDS 协议契约的 GUI 侧镜像）。
export interface Phonetic { uk: string | null; us: string | null }
export interface PosGloss { pos: string; gloss: string[] }
export interface Exchange {
  past: string | null; pp: string | null; ing: string | null;
  third: string | null; comparative: string | null; superlative: string | null;
  plural: string | null; lemma: string | null;
}
export interface WordCard {
  found: boolean; word: string;
  phonetic: Phonetic | null;
  pos: PosGloss[]; definition: string[];
  exchange: Exchange | null;
  collins: number | null; oxford: boolean; tags: string[];
  source: string; suggestions: string[];
}
export interface Translation {
  source_lang: 'en' | 'zh'; target_lang: 'en' | 'zh';
  text: string; translation: string; engine: string;
}
export interface HistoryEntry {
  ts_ms: number; text: string; translation: string; engine: string; origin: string;
}
export interface StatusInfo {
  version: string; uptime_s: number; clipboard_watching: boolean;
  auto_translate: boolean; default_engine: string; gui_connected: number;
  show_word_badge: boolean;
  counters: { clipboard_events: number; translations: number; notifications: number; triggers: number };
}
