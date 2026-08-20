// Primary navigation definition: shared by Sidebar (renders nav items) and TopBar (resolves the current page title).
import type { LucideIcon } from "lucide-react";
import { BarChart3, KeyRound, LayoutDashboard, Network, ScrollText, Settings } from "lucide-react";

export interface NavItem {
  path: string;
  label: string;
  Icon: LucideIcon;
}

export const NAV_ITEMS: NavItem[] = [
  { path: "/dashboard", label: "仪表盘", Icon: LayoutDashboard },
  { path: "/usage", label: "用量", Icon: BarChart3 },
  { path: "/channels", label: "渠道", Icon: Network },
  { path: "/api-keys", label: "密钥", Icon: KeyRound },
  { path: "/logs", label: "日志", Icon: ScrollText },
  { path: "/settings", label: "设置", Icon: Settings },
];
