// Theme hook: reads/updates the theme, applies it immediately on change, and persists to backend settings (settingsApi).
// The backend is the theme source of truth: loaded and reconciled on mount; localStorage is only a pre-render first-frame cache (pre-applied in main.tsx).
// The top bar and settings page each hold their own instance and manage their own loading; persistence re-reads the backend
// latest settings on every changeTheme before merging the theme, avoiding overwriting other fields just saved by the settings page (no caching of a stale settings snapshot).
import { useEffect, useRef, useState } from "react";
import { settingsApi } from "@/lib/api";
import { applyTheme, getStoredTheme, setStoredTheme, type Theme } from "@/lib/theme";

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(getStoredTheme);
  // If the user already switched the theme before the mount-time load finished, do not override with the backend value (avoids "click dark, it snaps back").
  const userChanged = useRef(false);

  // On mount, load the backend settings to reconcile the theme (source of truth).
  useEffect(() => {
    let cancelled = false;
    settingsApi
      .get()
      .then((settings) => {
        if (cancelled) return;
        if (!userChanged.current) setTheme(settings.theme);
      })
      .catch(() => {}); // keep localStorage as fallback when the backend is unavailable (pure browser dev)
    return () => {
      cancelled = true;
    };
  }, []);

  // Apply the theme on every change (system is handled by the CSS media query).
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const changeTheme = (next: Theme) => {
    userChanged.current = true;
    setStoredTheme(next);
    setTheme(next);
    // Persistence: re-read the latest backend settings first, then merge the theme; failures are silent (the next load will reconcile anyway).
    void settingsApi
      .get()
      .then((settings) => settingsApi.save({ ...settings, theme: next }))
      .catch(() => {});
  };

  return { theme, changeTheme };
}
