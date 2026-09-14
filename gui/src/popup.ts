import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { renderCardHTML, renderTranslationHTML } from './render';
import type { Translation, WordCard } from './types';
import './style.css';

interface PopupPayload {
  translation: Translation;
  card: WordCard | null;
}

async function renderPopup(p: PopupPayload): Promise<void> {
  const el = document.getElementById('popup-content')!;
  const card = p.card ?? null; // daemon 侧 card 键可能缺席（句子翻译）
  let html = '';
  if (card) html += renderCardHTML(card);
  html += renderTranslationHTML(p.translation);
  el.innerHTML = html;
  document.title = p.translation.text.slice(0, 30) || 'pardon';
  // 自适应高度：量整页自然高度（内容 + 动作行 + 内边距；#popup-content 的
  // max-height 保证长内容不会撑破 Rust 侧 480 上限），一帧后申请调整窗口
  // 高度（Rust 侧夹 120–480）。页面不设 100vh，短内容可缩到 clamp 下限。
  requestAnimationFrame(() => {
    const h = document.body.getBoundingClientRect().height;
    void invoke('popup_resize', { height: Math.ceil(h) });
  });
}

window.addEventListener('DOMContentLoaded', () => {
  // Esc 全局关闭（弹窗 show 时已聚焦，keydown 必达）
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') void invoke('hide_popup');
  });
  document.getElementById('btn-copy')?.addEventListener('click', () => {
    const text = document.getElementById('popup-content')?.innerText ?? '';
    void navigator.clipboard.writeText(text);
  });
  document.getElementById('btn-speak')?.addEventListener('click', () => {
    const src = document.querySelector<HTMLElement>('.src, .word')?.textContent;
    if (src) void invoke('speak', { text: src });
  });
  // 冷启动竞态兜底：webview 加载完成前到达的 popup 事件由 Rust 侧
  // PopupCache 缓存，就绪时经 popup_last 重放。先注册 listen 再 await
  // 重放（等待期间到达的新事件不丢）；若等待期间已有 live 事件渲染，
  // 跳过可能已过期的重放，避免旧负载覆盖新内容。
  let live = false;
  void listen('popup', (e) => {
    live = true;
    void renderPopup(e.payload as PopupPayload);
  });
  void (async () => {
    const last = (await invoke('popup_last')) as PopupPayload | null;
    if (last && !live) void renderPopup(last);
  })();
});
