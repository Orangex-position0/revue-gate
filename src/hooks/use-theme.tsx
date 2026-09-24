// Theme state provider: keeps one UI theme state and one persistence path for the whole application.
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { settingsApi } from "@/lib/api";
import { applyTheme, getStoredTheme, setStoredTheme, type Theme } from "@/lib/theme";

interface ThemeContextValue {
  theme: Theme;
  changeTheme: (theme: Theme) => void;
  themeError: string | null;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setTheme] = useState<Theme>(getStoredTheme);
  const userChanged = useRef(false);
  const writes = useRef<Promise<unknown>>(Promise.resolve());
  const writeVersion = useRef(0);
  const [themeError, setThemeError] = useState<string | null>(null);

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
    setThemeError(null);
    const version = ++writeVersion.current;
    // Preserve click order while the backend serializes updates from other settings pages.
    writes.current = writes.current.catch(() => {}).then(() =>
      settingsApi.saveSection({ section: "appearance", value: next }),
    ).catch((error: unknown) => {
      if (writeVersion.current === version) {
        setThemeError(error instanceof Error ? error.message : String(error));
      }
    });
  }

  return <ThemeContext.Provider value={{ theme, changeTheme, themeError }}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) {
    throw new Error("useTheme must be used inside ThemeProvider");
  }
  return value;
}
