// Theme tri-state selector: light / dark / system icon button group. Shared by the top bar and settings page.
// Controlled component: theme and onChange are injected by the caller (useTheme).
import type { LucideIcon } from "lucide-react";
import { Monitor, Moon, Sun } from "lucide-react";
import { type Theme, THEME_OPTIONS } from "@/lib/theme";

const THEME_ICONS: Record<Theme, LucideIcon> = {
  light: Sun,
  dark: Moon,
  system: Monitor,
};

const THEME_LABELS: Record<Theme, string> = {
  light: "浅色",
  dark: "深色",
  system: "跟随系统",
};

interface ThemeSelectorProps {
  theme: Theme;
  onChange: (theme: Theme) => void;
}

export function ThemeSelector({ theme, onChange }: ThemeSelectorProps) {
  return (
    <div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
      {THEME_OPTIONS.map((option) => {
        const Icon = THEME_ICONS[option];
        const active = theme === option;
        return (
          <button
            key={option}
            type="button"
            onClick={() => onChange(option)}
            aria-label={THEME_LABELS[option]}
            aria-pressed={active}
            title={THEME_LABELS[option]}
            className={`rounded p-1.5 transition-colors ${
              active
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            <Icon className="h-4 w-4" />
          </button>
        );
      })}
    </div>
  );
}
