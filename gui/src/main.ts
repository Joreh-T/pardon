import { listen } from '@tauri-apps/api/event';
import { historyList, lookup, status, translate } from './api';
import { Epoch } from './epoch';
import { escapeHtml, renderCardHTML, renderHistoryHTML, renderTranslationHTML } from './render';
import type { HistoryEntry } from './types';
import { invoke } from '@tauri-apps/api/core';
import { bootstrapTheme } from './theme'; // 顶层副作用：加载即应用持久化主题
import './style.css';

const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
const $input = () => document.querySelector<HTMLTextAreaElement>('#input')!;

let historyEntries: HistoryEntry[] = [];
// 词卡徽章开关：status 轮询缓存；status 不可达时保持上次值（启动默认关）。
let showBadge = false;
// daemon 自动拉起：每次「连接成功→失联」的下降沿自动尝试一次（GUI 从启动器/
// 快捷键拉起时顺带把整个 pardon 栈带起来）；失败才落回手动按钮。成功后复位，
// daemon 下次掉线再自动拉一次——不会在持续失联时循环重试。
let autoStartArmed = true;
// 翻译世代守卫：连续翻译时只让最新一代写结果区，迟到的旧结果丢弃
// （daemon 无取消协议，A 请求仍在后台跑完，但不再覆盖 B 的显示）。
const translateEpoch = new Epoch();

async function refreshHistory(): Promise<void> {
  try {
    historyEntries = await historyList(50);
    $('#history').innerHTML = renderHistoryHTML(historyEntries);
  } catch {
    $('#history').innerHTML = '<div class="empty">历史不可用</div>';
  }
}

async function doTranslate(text: string): Promise<void> {
  const t = text.trim();
  if (!t) return;
  const me = translateEpoch.begin();
  $('#result').innerHTML = '<div class="empty">翻译中…</div>';
  // 词卡与翻译并行；词卡命中则上方展示（句子翻译只有译文区）
  const [t2, card] = await Promise.allSettled([translate(t), lookup(t)]);
  if (!translateEpoch.isCurrent(me)) return; // 已被更新的翻译取代
  if (t2.status === 'rejected') {
    $('#result').innerHTML = `<div class="empty fail">${escapeHtml(String(t2.reason))}</div>`;
    return;
  }
  let html = '';
  if (card.status === 'fulfilled' && card.value.found) html += renderCardHTML(card.value, showBadge);
  html += renderTranslationHTML(t2.value);
  $('#result').innerHTML = html;
  void refreshHistory();
}

async function refreshStatus(): Promise<void> {
  try {
    const s = await status();
    showBadge = s.show_word_badge === true;
    autoStartArmed = true;
    $('#conn').textContent = `已连接 pardond ${s.version} · 引擎 ${s.default_engine}`;
    $('#btn-start-daemon').hidden = true;
  } catch {
    showBadge = false;
    if (autoStartArmed) {
      autoStartArmed = false;
      $('#conn').textContent = 'pardond 未运行，正在自动启动…';
      try {
        await invoke('pardon_daemon_start');
      } catch (e) {
        $('#conn').textContent = `自动启动失败：${String(e)}`;
      }
      // pardond 起来要加载词典（~1-2s），稍候再查；仍失联则下一轮显示手动按钮
      setTimeout(() => void refreshStatus(), 1500);
      return;
    }
    $('#conn').textContent = '未连接 pardond';
    $('#btn-start-daemon').hidden = false;
  }
}

function wire(): void {
  $('#btn-translate').addEventListener('click', () => void doTranslate($input().value));
  $input().addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void doTranslate($input().value);
    }
  });
  $('#btn-settings').addEventListener('click', () => void invoke('open_settings'));
  $('#btn-start-daemon').addEventListener('click', async () => {
    try {
      await invoke('pardon_daemon_start');
    } catch (e) {
      // 启动失败（非零退出）→ stderr 文案写状态栏；1.5s 后照常刷新连接状态
      $('#conn').textContent = `启动失败：${String(e)}`;
    }
    setTimeout(() => void refreshStatus(), 1500);
  });
  $('#btn-speak').addEventListener('click', () => {
    const src = document.querySelector<HTMLElement>('#result .src')?.textContent;
    if (src) void invoke('speak', { text: src });
  });
  $('#history').addEventListener('click', (e) => {
    const item = (e.target as HTMLElement).closest<HTMLElement>('.hist-item');
    if (!item) return;
    const entry = historyEntries[Number(item.dataset.idx)];
    if (entry) {
      $input().value = entry.text;
      void doTranslate(entry.text);
    }
  });
  void listen('ipc-up', () => void refreshStatus());
  // 设置页切换主题后广播；各窗口重读 localStorage（同源共享）重应用
  void listen('theme-changed', () => bootstrapTheme());
}

window.addEventListener('DOMContentLoaded', () => {
  wire();
  void refreshStatus();
  void refreshHistory();
  setInterval(() => void refreshStatus(), 5000);
});
