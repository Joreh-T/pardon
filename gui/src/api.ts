import { invoke } from '@tauri-apps/api/core';
import type { HistoryEntry, StatusInfo, Translation, WordCard } from './types';

const ipc = (method: string, params: Record<string, unknown>) =>
  invoke<unknown>('ipc_request', { method, params });

export const ping = () => ipc('ping', {});
export const status = () => ipc('status', {}) as Promise<StatusInfo>;
export const translate = (text: string) => ipc('translate', { text }) as Promise<Translation>;
export const lookup = (word: string) => ipc('lookup', { word }) as Promise<WordCard>;
export const historyList = (limit = 50) => ipc('history', { limit }) as Promise<HistoryEntry[]>;
export const reload = () => ipc('reload', {}) as Promise<{ restart_required: boolean }>;
