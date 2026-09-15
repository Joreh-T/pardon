/**
 * 主题注册与应用。合法值集合以本文件为唯一权威（R4）：
 * 三页 <head> 的内联防闪脚本是其浏览器侧复制品，改集合时两处同步。
 */

export type ThemeName = 'auto' | 'catppuccin-mocha' | 'catppuccin-latte' | 'macos-light' | 'nord';

export const THEMES: { name: ThemeName; label: string }[] = [
  { name: 'auto', label: '跟随系统' },
  { name: 'catppuccin-mocha', label: 'Catppuccin 摩卡（暗）' },
  { name: 'catppuccin-latte', label: 'Catppuccin 拿铁（亮）' },
  { name: 'macos-light', label: 'macOS 浅色（清新）' },
  { name: 'nord', label: 'Nord（暗蓝）' },
];

const VALID = new Set<string>(THEMES.map((t) => t.name));

/** localStorage / 内联脚本共用的合法值判定：非法或缺失一律归 auto。 */
export function normalizeTheme(raw: string | null): ThemeName {
  return VALID.has(raw ?? '') ? (raw as ThemeName) : 'auto';
}

const STORAGE_KEY = 'pardon-theme';

/** 写 dataset（驱动 CSS 变量组）+ localStorage（跨窗口共享、重启记忆）。 */
export function applyTheme(name: ThemeName): void {
  try {
    localStorage.setItem(STORAGE_KEY, name);
  } catch {
    // 持久化不可用（隐私模式 / node 测试环境）时仅本次会话生效
  }
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = name;
  }
}

export function currentTheme(): ThemeName {
  try {
    return normalizeTheme(localStorage.getItem(STORAGE_KEY));
  } catch {
    return 'auto';
  }
}

/** 幂等：与三页 <head> 内联防闪脚本重复执行无副作用差。 */
export function bootstrapTheme(): void {
  applyTheme(currentTheme());
}

// 模块加载即应用持久化主题：入口只需 import 本模块即完成引导。
// vitest node 环境下 document/localStorage 均缺席，各函数已容错为 no-op。
bootstrapTheme();
