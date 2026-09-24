// Top bar: current page title + server status indicator (driven by useServerStore) + theme tri-state switch.
import { useLocation } from "react-router-dom";
import { ThemeSelector } from "@/components/ThemeSelector";
import { useTheme } from "@/hooks/use-theme";
import { useServerStore } from "@/stores/use-server-store";
import { NAV_ITEMS, SETTINGS_SECTIONS } from "./nav";

export function TopBar() {
  const { pathname } = useLocation();
  const { theme, changeTheme, themeError } = useTheme();
  const { running, host, port } = useServerStore();

  const settingsSection = pathname.startsWith("/settings/")
    ? SETTINGS_SECTIONS.find((entry) => pathname === `/settings/${entry.id}`)
    : null;
  const currentLabel = settingsSection
    ? `设置 / ${settingsSection.title}`
    : NAV_ITEMS.find((item) => pathname.startsWith(item.path))?.label ?? "工作台";
  const endpoint = running && host && port ? `${host}:${port}` : "服务未启动";

  return (
    <header className="topbar flex h-16 shrink-0 items-center justify-between border-b border-border bg-background/90 px-5 backdrop-blur-sm sm:px-7">
      <div className="min-w-0">
        <div className="text-[11px] font-medium uppercase tracking-[0.16em] text-muted-foreground/70">控制台</div>
        <h1 className="mt-0.5 truncate text-lg font-semibold tracking-tight">{currentLabel}</h1>
      </div>

      <div className="flex items-center gap-3">
        <div
          className={`hidden items-center gap-2 rounded-full border px-3 py-1.5 text-xs sm:flex ${running ? "border-success/25 bg-success/10 text-success" : "border-border bg-card text-muted-foreground"}`}
          title={running ? `数据面监听 ${endpoint}` : "数据面未启动"}
        >
          <span className={`h-1.5 w-1.5 rounded-full ${running ? "bg-success" : "bg-muted-foreground/60"}`} />
          <span>{running ? "服务运行中" : "未启动"}</span>
          <span className="max-w-36 truncate font-mono opacity-80">{endpoint}</span>
        </div>

        {/* Theme tri-state: switching applies the theme and persists it; system is handled by the CSS media query. */}
        {themeError && <span className="max-w-40 text-xs text-danger" role="alert">主题保存失败：{themeError}</span>}
        <ThemeSelector theme={theme} onChange={changeTheme} />
      </div>
    </header>
  );
}
