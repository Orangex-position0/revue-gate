// Sidebar: brand area + primary nav items (NavLink highlights the active route).
import { useState } from "react";
import { NavLink } from "react-router-dom";
import { Activity, Check, Copy, Server } from "lucide-react";
import { APP_NAME } from "@/lib/constants";
import { useServerStore } from "@/stores/use-server-store";
import { NAV_ITEMS } from "./nav";

export function Sidebar() {
  const { running, host, port } = useServerStore();
  const [copied, setCopied] = useState(false);
  const endpoint = running && host && port ? `http://${host}:${port}/v1` : null;

  async function copyEndpoint() {
    if (!endpoint) return;
    await navigator.clipboard.writeText(endpoint);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  }

  return (
    <aside className="app-sidebar flex w-64 shrink-0 flex-col border-r border-border bg-card/80 px-3 py-3">
      <div className="brand-panel rounded-2xl border border-border bg-background/70 p-4">
        <div className="flex items-center gap-3">
          <div className="brand-mark flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary text-sm font-bold text-primary-foreground shadow-sm">
            RG
          </div>
          <div className="min-w-0">
            <div className="text-base font-bold tracking-tight">{APP_NAME}</div>
            <div className="mt-0.5 text-[11px] text-muted-foreground">本地 LLM API 网关</div>
          </div>
        </div>
      </div>

      <nav aria-label="主导航" className="mt-5 min-h-0 flex-1 space-y-1 overflow-y-auto">
        <div className="mb-2 px-3 text-[10px] font-semibold uppercase tracking-[0.16em] text-muted-foreground/70">工作台</div>
        {NAV_ITEMS.map(({ path, label, Icon }) => (
          <NavLink
            key={path}
            to={path}
            className={({ isActive }) =>
              `nav-item group flex items-center gap-3 rounded-xl px-3 py-2.5 text-sm transition-colors ${
                isActive
                  ? "bg-accent font-semibold text-accent-foreground shadow-sm"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground"
              }`
            }
          >
            <span className="nav-icon flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-transparent bg-background/50 transition-colors group-hover:border-border group-hover:bg-card">
              <Icon className="h-4 w-4" />
            </span>
            <span>{label}</span>
          </NavLink>
        ))}
      </nav>

      <section className="service-card mt-4 rounded-2xl border border-border bg-background/70 p-3.5" aria-label="服务状态">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-[11px] font-medium text-muted-foreground">服务状态</div>
            <div className="mt-1 flex items-center gap-2 text-sm font-semibold">
              <span className={`h-2 w-2 rounded-full ${running ? "bg-success shadow-[0_0_0_3px_var(--success-soft)]" : "bg-muted-foreground/50"}`} />
              {running ? "运行中" : "未启动"}
            </div>
          </div>
          <Activity className={`h-4 w-4 ${running ? "text-success" : "text-muted-foreground"}`} />
        </div>
        <button
          type="button"
          onClick={() => void copyEndpoint()}
          disabled={!endpoint}
          className="mt-3 flex w-full items-center gap-2 rounded-xl border border-border bg-card px-3 py-2 text-left text-xs transition-colors hover:border-primary/40 hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
          title={endpoint ? "复制 API Base URL" : "服务启动后可复制地址"}
        >
          <Server className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">{endpoint ?? "等待服务启动"}</span>
          {copied ? <Check className="h-3.5 w-3.5 shrink-0 text-success" /> : <Copy className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />}
        </button>
      </section>
    </aside>
  );
}
