// Theme tri-state application: light / dark / system.
// system writes no data-theme; the CSS media query (prefers-color-scheme) responds, with zero JS listeners.
// The theme type lives in types/ (aligned with the backend Theme enum); re-exported here to keep existing import paths.
// Persistence goes through backend settings (settingsApi, ticket 12); localStorage is only a pre-render first-frame cache.
import type { Theme } from "@/types";

export type { Theme };

export const THEME_STORAGE_KEY = "revue-gate-theme";

export const THEME_OPTIONS: Theme[] = ["light", "dark", "system"];

/** Apply the theme: explicit light/dark set the data-theme attribute; system removes it, leaving it to the CSS media query. */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

/** Read the locally stored theme, falling back to system when invalid or missing. */
export function getStoredTheme(): Theme {
  const stored = localStorage.getItem(THEME_STORAGE_KEY);
  return stored === "light" || stored === "dark" || stored === "system"
    ? stored
    : "system";
}

export function setStoredTheme(theme: Theme): void {
  localStorage.setItem(THEME_STORAGE_KEY, theme);
}
