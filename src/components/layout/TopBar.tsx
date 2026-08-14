// Top bar: current page title + server status indicator (driven by useServerStore) + theme tri-state switch.
import { useLocation } from "react-router-dom";
import { ThemeSelector } from "@/components/ThemeSelector";
import { useTheme } from "@/hooks/use-theme";
import { useServerStore } from "@/stores/use-server-store";
import { NAV_ITEMS } from "./nav";

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
        {/* Server status indicator: the store is written only by server events, so the indicator stays in sync with the backend */}
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

        {/* Theme tri-state: switching applies the theme and persists it; system is handled by the CSS media query */}
        <ThemeSelector theme={theme} onChange={changeTheme} />
      </div>
    </header>
  );
}
