// 主题三态应用：light / dark / system。
// system 不写 data-theme，由 CSS 媒体查询（prefers-color-scheme）响应，零 JS 监听。
// 持久化暂用 localStorage；settings 后端（ticket 12，tauri-plugin-store）落地后迁移到 settingsApi。
export type Theme = "light" | "dark" | "system";

export const THEME_STORAGE_KEY = "revue-gate-theme";

export const THEME_OPTIONS: Theme[] = ["light", "dark", "system"];

/** 应用主题：显式 light/dark 写 data-theme 属性；system 移除属性交给 CSS 媒体查询。 */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

/** 读取本地存储的主题，非法或缺失时回退为跟随系统。 */
export function getStoredTheme(): Theme {
  const stored = localStorage.getItem(THEME_STORAGE_KEY);
  return stored === "light" || stored === "dark" || stored === "system"
    ? stored
    : "system";
}

export function setStoredTheme(theme: Theme): void {
  localStorage.setItem(THEME_STORAGE_KEY, theme);
}
