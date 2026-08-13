// 顶栏：当前页面标题 + 服务状态灯（useServerStore 驱动）+ 主题三态切换。
import type { LucideIcon } from "lucide-react";
import { Monitor, Moon, Sun } from "lucide-react";
import { useLocation } from "react-router-dom";
import { useTheme } from "@/hooks/use-theme";
import { type Theme, THEME_OPTIONS } from "@/lib/theme";
import { useServerStore } from "@/stores/use-server-store";
import { NAV_ITEMS } from "./nav";

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

export function TopBar() {
  const { pathname } = useLocation();
  const { theme, changeTheme } = useTheme();
  const { running, host, port } = useServerStore();

  const currentLabel = NAV_ITEMS.find((item) => pathname.startsWith(item.path))?.label ?? "";
  const endpoint = running && host && port ? `${host}:${port}` : "已停止";

  return (
    <header className="flex h-14 shrink-0 items-center justify-between border-b border-border bg-card px-6">
      <h1 className="text-base font-semibold">{currentLabel}</h1>

      <div className="flex items-center gap-4">
        {/* 服务状态灯：store 只由 server 事件写入，状态灯与后端启停一致 */}
        <div
          className="flex items-center gap-2 text-sm"
          title={running ? `数据面监听 ${endpoint}` : "数据面未启动"}
        >
          <span
            className={`h-2 w-2 rounded-full ${
              running ? "bg-success" : "bg-muted-foreground"
            }`}
          />
          <span className="text-muted-foreground">{endpoint}</span>
        </div>

        {/* 主题三态：切换即 applyTheme；system 由 CSS 媒体查询响应 */}
        <div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
          {THEME_OPTIONS.map((option) => {
            const Icon = THEME_ICONS[option];
            const active = theme === option;
            return (
              <button
                key={option}
                type="button"
                onClick={() => changeTheme(option)}
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
      </div>
    </header>
  );
}
