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
  void listen('popup', (e) => void renderPopup(e.payload as PopupPayload));
});
