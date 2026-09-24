// Theme state provider: keeps one UI theme state and one persistence path for the whole application.
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { settingsApi } from "@/lib/api";
import { applyTheme, getStoredTheme, setStoredTheme, type Theme } from "@/lib/theme";

interface ThemeContextValue {
  theme: Theme;
  changeTheme: (theme: Theme) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setTheme] = useState<Theme>(getStoredTheme);
  const userChanged = useRef(false);

  useEffect(() => {
    let cancelled = false;
    void settingsApi.get().then((settings) => {
      if (!cancelled && !userChanged.current) {
        setTheme(settings.theme);
      }
    }).catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  function changeTheme(next: Theme) {
    userChanged.current = true;
    setStoredTheme(next);
    setTheme(next);
    void settingsApi.get()
      .then((settings) => settingsApi.save({ ...settings, theme: next }))
      .catch(() => {});
  }

  return <ThemeContext.Provider value={{ theme, changeTheme }}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) {
    throw new Error("useTheme must be used inside ThemeProvider");
  }
  return value;
}
